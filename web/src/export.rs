//! Deterministic, complete CSV export of a month (spec §6, §13.6).
//!
//! One file, one header, a `record_type` column per row:
//! `month`, `income_line`, `paycheck`, `category`, `expense_line`,
//! `allocation`, `transaction`. Amounts are plain decimals in the month's
//! currency (e.g. `1234.56`). Rows are ordered by date / display order.

use crate::i18n::t;
use crate::money::plain;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use paycheckzero_core::{Cents, Month};

pub const HEADER: &[&str] = &[
    "record_type",
    "month",
    "date",
    "income_line",
    "category",
    "expense_line",
    "payee",
    "notes",
    "status",
    "schedule",
    "planned",
    "actual",
    "spent",
    "remaining",
    "amount",
    "current_balance",
    "minimum_payment",
    "split_group",
];

fn esc(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[derive(Default)]
struct Row<'a> {
    record_type: &'a str,
    date: String,
    income_line: String,
    category: String,
    expense_line: String,
    payee: String,
    notes: String,
    status: String,
    schedule: String,
    planned: Option<Cents>,
    actual: Option<Cents>,
    spent: Option<Cents>,
    remaining: Option<Cents>,
    amount: Option<Cents>,
    current_balance: Option<Cents>,
    minimum_payment: Option<Cents>,
    split_group: String,
}

#[must_use]
pub fn to_csv(m: &Month) -> String {
    let ym = m.year_month.format("%Y-%m").to_string();
    let mut rows: Vec<Row> = Vec::new();
    let status = if m.reassigning { "reassigning" } else { m.status.as_str() };
    rows.push(Row {
        record_type: "month",
        status: status.into(),
        planned: Some(m.total_planned_income()),
        amount: Some(m.total_planned_expense()),
        remaining: Some(m.zero_difference()),
        spent: Some(m.expense_lines.iter().map(|l| m.line_spent(&l.id)).sum()),
        ..Row::default()
    });
    for l in &m.income_lines {
        rows.push(Row {
            record_type: "income_line",
            income_line: l.name.clone(),
            schedule: l.recurrence_rule.as_ref().map_or_else(|| "one_off".into(), paycheckzero_core::Recurrence::to_json),
            planned: Some(l.planned_amount),
            ..Row::default()
        });
    }
    let name_of = |id: &paycheckzero_core::Id| m.income_line(id).map(|l| l.name.clone()).unwrap_or_default();
    for p in m.paychecks_by_date() {
        rows.push(Row {
            record_type: "paycheck",
            date: p.date.to_string(),
            income_line: name_of(&p.income_line_id),
            status: p.status.as_str().into(),
            planned: Some(p.planned_amount),
            actual: p.actual_amount,
            remaining: Some(m.safe_to_spend(&p.id)),
            amount: Some(m.paycheck_allocated(&p.id)),
            ..Row::default()
        });
    }
    for c in m.category_views() {
        rows.push(Row {
            record_type: "category",
            category: c.name.clone(),
            status: c.kind.as_str().into(),
            planned: Some(c.planned),
            spent: Some(c.spent),
            remaining: Some(c.remaining),
            ..Row::default()
        });
        for l in &c.lines {
            rows.push(Row {
                record_type: "expense_line",
                category: c.name.clone(),
                expense_line: l.name.clone(),
                planned: Some(l.planned),
                spent: Some(l.spent),
                remaining: Some(l.remaining),
                current_balance: l.current_balance,
                minimum_payment: l.minimum_payment,
                ..Row::default()
            });
            for f in &l.funders {
                rows.push(Row {
                    record_type: "allocation",
                    date: f.date.to_string(),
                    income_line: m.paycheck(&f.paycheck_id).map(|p| name_of(&p.income_line_id)).unwrap_or_default(),
                    category: c.name.clone(),
                    expense_line: l.name.clone(),
                    amount: Some(f.amount),
                    ..Row::default()
                });
            }
        }
    }
    let mut txs: Vec<(usize, &paycheckzero_core::Transaction)> = m.transactions.iter().enumerate().collect();
    txs.sort_by_key(|(i, t)| (t.date, *i));
    for (_, tx) in txs {
        let line = tx.expense_line_id.as_ref().and_then(|l| m.expense_line(l));
        rows.push(Row {
            record_type: "transaction",
            date: tx.date.to_string(),
            income_line: tx
                .paycheck_id
                .as_ref()
                .and_then(|p| m.paycheck(p))
                .map(|p| format!("{} {}", name_of(&p.income_line_id), p.date))
                .unwrap_or_default(),
            category: line.and_then(|l| m.category(&l.category_id)).map(|c| c.name.clone()).unwrap_or_default(),
            expense_line: line.map(|l| l.name.clone()).unwrap_or_default(),
            payee: tx.payee.clone().unwrap_or_default(),
            notes: tx.notes.clone().unwrap_or_default(),
            amount: Some(tx.amount),
            split_group: tx.split_group.as_ref().map(ToString::to_string).unwrap_or_default(),
            ..Row::default()
        });
    }

    let money = |c: Option<Cents>| c.map(plain).unwrap_or_default();
    let mut out = HEADER.join(",");
    out.push_str("\r\n");
    for r in rows {
        let fields = [
            r.record_type.to_string(),
            ym.clone(),
            r.date,
            r.income_line,
            r.category,
            r.expense_line,
            r.payee,
            r.notes,
            r.status,
            r.schedule,
            money(r.planned),
            money(r.actual),
            money(r.spent),
            money(r.remaining),
            money(r.amount),
            money(r.current_balance),
            money(r.minimum_payment),
            r.split_group,
        ];
        out.push_str(&fields.iter().map(|f| esc(f)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

#[must_use]
pub fn filename(m: &Month, ext: &str) -> String {
    format!("{}-{}.{ext}", t("app.file_prefix"), m.year_month.format("%Y-%m"))
}

#[must_use]
pub fn csv_response(m: &Month) -> Response {
    let mut r = to_csv(m).into_response();
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename(m, "csv"))) {
        r.headers_mut().insert(CONTENT_DISPOSITION, v);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use paycheckzero_core::*;

    #[test]
    fn csv_is_deterministic_and_escaped() {
        let mut m = Month::create(Id::new("m"), NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(), CopyMode::Blank, None);
        m.add_income_line("Pay", Cents::new(100_000), Schedule::OneOff { dates: vec![NaiveDate::from_ymd_opt(2026, 9, 4).unwrap()] })
            .unwrap();
        let cat = m.categories[0].id.clone();
        let l = m.add_expense_line(&cat, "Tithe, \"church\"").unwrap();
        let p = m.paychecks[0].id.clone();
        m.set_allocation(&p, &l, Cents::new(10_000)).unwrap();
        let a = to_csv(&m);
        assert_eq!(a, to_csv(&m.clone()));
        assert!(a.starts_with("record_type,month,date,"));
        assert!(a.contains("expense_line,2026-09,,,Giving,\"Tithe, \"\"church\"\"\",,,,,100.00,,0.00,100.00,,,,"));
        assert!(a.contains("allocation,2026-09,2026-09-04,Pay,Giving,"));
    }
}
