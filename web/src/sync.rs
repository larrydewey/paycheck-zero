//! Offline sync (spec §13.7). The browser queues creates/edits made while
//! offline and replays them here. Each op carries the record as the client
//! last saw it (`base`); if the server copy changed meanwhile the op is
//! reported as a conflict for the user to resolve (keep mine = resend with
//! `force`, keep server = discard). Ops are idempotent by `op_id`.

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::Shared;
use axum::extract::State;
use axum::{Extension, Json};
use chrono::NaiveDate;
use paycheckzero_core::{Cents, DomainError, Id, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxData {
    pub date: NaiveDate,
    pub amount: i64,
    #[serde(default)]
    pub payee: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub expense_line_id: Option<Id>,
    #[serde(default)]
    pub paycheck_id: Option<Id>,
    #[serde(default)]
    pub account_id: Option<Id>,
}

impl TxData {
    fn from_tx(t: &Transaction) -> TxData {
        TxData {
            date: t.date,
            amount: t.amount.get(),
            payee: t.payee.clone(),
            notes: t.notes.clone(),
            expense_line_id: t.expense_line_id.clone(),
            paycheck_id: t.paycheck_id.clone(),
            account_id: t.account_id.clone(),
        }
    }

    fn into_tx(self, id: Id) -> Transaction {
        let clean = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        Transaction {
            id,
            date: self.date,
            amount: Cents::new(self.amount),
            payee: clean(self.payee),
            notes: clean(self.notes),
            expense_line_id: self.expense_line_id.filter(|i| !i.as_str().is_empty()),
            paycheck_id: self.paycheck_id.filter(|i| !i.as_str().is_empty()),
            split_group: None,
            account_id: self.account_id.filter(|i| !i.as_str().is_empty()),
            transfer_account_id: None,
            external_id: None,
        }
    }

    fn normalized(self) -> TxData {
        TxData::from_tx(&self.into_tx(Id::new("")))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpKind {
    CreateTransaction { month_id: Id, id: Id, tx: TxData },
    UpdateTransaction { month_id: Id, id: Id, base: TxData, tx: TxData },
    DeleteTransaction { month_id: Id, id: Id, base: TxData },
    SetActual { month_id: Id, paycheck_id: Id, base_actual: Option<i64>, actual: Option<i64> },
}

#[derive(Debug, Deserialize)]
pub struct Op {
    pub op_id: String,
    #[serde(default)]
    pub force: bool,
    #[serde(flatten)]
    pub kind: OpKind,
}

#[derive(Debug, Deserialize)]
pub struct SyncReq {
    pub ops: Vec<Op>,
}

#[derive(Debug, Serialize)]
pub struct OpResult {
    pub op_id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<Value>,
}

enum Outcome {
    Applied,
    Conflict(Value),
}

async fn apply(st: &Shared, user: &AuthUser, op: Op) -> Result<Outcome, AppError> {
    let force = op.force;
    match op.kind {
        OpKind::CreateTransaction { month_id, id, tx } => {
            st.mutate(&user.0, &month_id, |m| {
                if m.transaction(&id).is_some() {
                    return Ok(());
                }
                m.add_transaction(tx.into_tx(id.clone())).map(|_| ())
            })
            .await?;
            Ok(Outcome::Applied)
        }
        OpKind::UpdateTransaction { month_id, id, base, tx } => {
            let current = st.load(&user.0, &month_id).await?.month.transaction(&id).map(TxData::from_tx);
            match current {
                None => Ok(Outcome::Conflict(Value::Null)),
                Some(cur) if !force && cur != base.normalized() => Ok(Outcome::Conflict(json!(cur))),
                Some(_) => {
                    st.mutate(&user.0, &month_id, |m| m.update_transaction(tx.into_tx(id.clone()))).await?;
                    Ok(Outcome::Applied)
                }
            }
        }
        OpKind::DeleteTransaction { month_id, id, base } => {
            let current = st.load(&user.0, &month_id).await?.month.transaction(&id).map(TxData::from_tx);
            match current {
                None => Ok(Outcome::Applied),
                Some(cur) if !force && cur != base.normalized() => Ok(Outcome::Conflict(json!(cur))),
                Some(_) => {
                    st.mutate(&user.0, &month_id, |m| m.delete_transaction(&id)).await?;
                    Ok(Outcome::Applied)
                }
            }
        }
        OpKind::SetActual { month_id, paycheck_id, base_actual, actual } => {
            let month = st.load(&user.0, &month_id).await?.month;
            let cur = month
                .paycheck(&paycheck_id)
                .ok_or(AppError::Domain(DomainError::NotFound { kind: "paycheck", id: paycheck_id.clone() }))?
                .actual_amount
                .map(Cents::get);
            if !force && cur != base_actual && cur != actual {
                return Ok(Outcome::Conflict(json!({ "actual_amount": cur })));
            }
            st.mutate(&user.0, &month_id, |m| m.set_paycheck_actual(&paycheck_id, actual.map(Cents::new))).await?;
            Ok(Outcome::Applied)
        }
    }
}

pub async fn sync(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Json(req): Json<SyncReq>) -> Json<Value> {
    let mut results = Vec::with_capacity(req.ops.len());
    for op in req.ops {
        let op_id = op.op_id.clone();
        if let Ok(Some(prev)) = st.store.sync_result(user.id(), &op_id).await {
            if let Ok(v) = serde_json::from_str::<Value>(&prev) {
                results.push(v);
                continue;
            }
        }
        let result = match apply(&st, &user, op).await {
            Ok(Outcome::Applied) => OpResult { op_id: op_id.clone(), status: "applied", message: None, server: None },
            Ok(Outcome::Conflict(server)) => OpResult {
                op_id: op_id.clone(),
                status: "conflict",
                message: Some(crate::i18n::t("sync.conflict")),
                server: Some(server),
            },
            Err(e) => OpResult {
                op_id: op_id.clone(),
                status: "error",
                message: Some(e.human(&user.0.currency, None)),
                server: None,
            },
        };
        let v = serde_json::to_value(&result).unwrap_or(Value::Null);
        if result.status == "applied" {
            let _ = st.store.record_sync(user.id(), &op_id, &v.to_string()).await;
        }
        results.push(v);
    }
    Json(json!({ "results": results }))
}
