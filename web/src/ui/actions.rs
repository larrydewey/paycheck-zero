//! Datastar action handlers. Each mutation goes through the service layer
//! (server-side invariants), then answers with SSE that re-renders the
//! originating view and shows a toast.

use super::pages::content_sse;
use super::*;
use crate::api::parse_year_month;
use crate::auth::{self, AuthUser};
use crate::error::{AppError, AppResult};
use crate::money::parse as parse_money;
use crate::sse::Sse;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form, Json};
use paycheckzero_core::suggest::suggest_income;
use paycheckzero_core::*;
use paycheckzero_storage::{Owner, UserRecord};
use serde::Deserialize;
use std::collections::HashMap;

pub(super) type F = Form<HashMap<String, String>>;

pub(super) fn field<'a>(f: &'a HashMap<String, String>, k: &str) -> &'a str {
    f.get(k).map_or("", |s| s.trim())
}

pub(super) fn opt_id(f: &HashMap<String, String>, k: &str) -> Option<Id> {
    let v = field(f, k);
    (!v.is_empty()).then(|| Id::new(v))
}

pub(super) fn view_of(f: &HashMap<String, String>, fallback: View) -> View {
    View::decode(field(f, "view")).unwrap_or(fallback)
}

/// Parses a required, non-negative money field.
pub(super) fn money_field(f: &HashMap<String, String>, k: &str) -> AppResult<Cents> {
    match parse_money(field(f, k)) {
        Some(Ok(c)) => Ok(c),
        Some(Err(())) => Err(AppError::bad(t("err.money_format"))),
        None => Err(AppError::bad(t("err.money_required"))),
    }
}

/// Parses an optional money field (empty = `None`).
pub(super) fn opt_money_field(f: &HashMap<String, String>, k: &str) -> AppResult<Option<Cents>> {
    match parse_money(field(f, k)) {
        Some(Ok(c)) => Ok(Some(c)),
        Some(Err(())) => Err(AppError::bad(t("err.money_format"))),
        None => Ok(None),
    }
}

/// Tells the user what a cascade changed and whether to re-balance
/// (invariants 6 and 7).
fn impact_toasts(user: &UserRecord, m: &Month, impact: &Impact) -> Vec<Markup> {
    if impact.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = impact.reduced_lines.iter().filter_map(|l| m.expense_line(l)).map(|l| l.name.clone()).collect();
    let mut msg = String::new();
    if !impact.removed_paychecks.is_empty() {
        let dates: Vec<String> = impact.removed_paychecks.iter().map(|d| short_date(*d)).collect();
        msg.push_str(&tf("impact.removed", &[("dates", &dates.join(", "))]));
        msg.push(' ');
    }
    if !names.is_empty() {
        msg.push_str(&tf("impact.reduced", &[("lines", &names.join(", "))]));
        msg.push(' ');
    }
    if !m.is_zero() {
        msg.push_str(&tf("impact.rebalance", &[("amount", &crate::money::format(m.zero_difference(), &user.currency))]));
    }
    vec![toast(ToastKind::Warning, msg.trim(), None)]
}

/// Standard response: re-render the view, then append toasts.
pub(super) async fn done(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View, toasts: Vec<Markup>) -> Sse {
    // Success: empty the "add something" form that was just submitted.
    let mut sse = content_sse(st, user, headers, view).await.script("pz.clearDone(); pz.closeSheet()");
    for t in toasts {
        sse = sse.patch_into("#toasts", "append", t);
    }
    sse
}

/// Error response: human message + technical details; the view is
/// re-rendered so inputs snap back to the server's values.
pub(super) async fn failed(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View, err: &AppError) -> Sse {
    let month = match view.month() {
        Some(mid) => st.load(user, mid).await.ok().map(|l| l.month),
        None => None,
    };
    let msg = err.human(&user.currency, month.as_ref());
    content_sse(st, user, headers, view)
        .await
        .patch_into("#toasts", "append", toast(ToastKind::Error, &msg, Some(&err.technical())))
        .script("pz.resetInline()")
}

async fn finish<T>(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View, r: AppResult<T>, ok_toasts: impl FnOnce(T) -> Vec<Markup>) -> Sse {
    match r {
        Ok(v) => done(st, user, headers, view, ok_toasts(v)).await,
        Err(e) => failed(st, user, headers, view, &e).await,
    }
}

// ----------------------------------------------------------------------
// Auth
// ----------------------------------------------------------------------

fn auth_error(msg: &str) -> Sse {
    Sse::new().patch(html! { div id="auth-error" class="field-error" role="alert" { (msg) } })
}

fn is_datastar(headers: &HeaderMap) -> bool {
    headers.contains_key("datastar-request")
}

fn user_agent(headers: &HeaderMap) -> &str {
    headers.get(axum::http::header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("-")
}

/// Sign-in outcome for both Datastar (SSE) and plain form posts (303).
fn auth_response(st: &Shared, headers: &HeaderMap, result: Result<(auth::Tokens, String), AppError>, back: &str) -> Response {
    let datastar = is_datastar(headers);
    match result {
        Ok((tokens, to)) => {
            let cookies = auth::session_cookies(st, &tokens);
            if datastar {
                Sse::new().with_cookies(cookies).redirect(&to).into_response()
            } else {
                let mut r = axum::response::Redirect::to(&to).into_response();
                for c in cookies {
                    r.headers_mut().append(axum::http::header::SET_COOKIE, c);
                }
                r
            }
        }
        Err(e) => {
            if datastar {
                auth_error(&e.human("USD", None)).into_response()
            } else {
                axum::response::Redirect::to(&format!("{back}?error={}", e.code())).into_response()
            }
        }
    }
}

pub async fn login(State(st): State<Shared>, headers: HeaderMap, Form(f): F) -> Response {
    let email = field(&f, "email").to_string();
    let r = async {
        let user = st.login(&email, f.get("password").map_or("", String::as_str)).await?;
        let tokens = auth::issue(&st, &user).await?;
        Ok::<_, AppError>((tokens, "/?signed_in=1".to_string()))
    }
    .await;
    match &r {
        Ok(_) => tracing::info!(email = %email, ua = %user_agent(&headers), js = is_datastar(&headers), "sign-in ok"),
        Err(e) => tracing::info!(email = %email, ua = %user_agent(&headers), js = is_datastar(&headers), code = e.code(), "sign-in failed"),
    }
    auth_response(&st, &headers, r, "/login")
}

pub async fn register(State(st): State<Shared>, headers: HeaderMap, Form(f): F) -> Response {
    let r = async {
        let user = st.register(field(&f, "email"), f.get("password").map_or("", String::as_str), field(&f, "timezone")).await?;
        let tokens = auth::issue(&st, &user).await?;
        // Lightweight onboarding: start the current month and go add a paycheck.
        let today = st.today_for(&user);
        let m = st.create_month(&user, today, CopyMode::Blank, None).await?;
        st.store.set_last_month(&user.id, Some(&m.id)).await?;
        Ok::<_, AppError>((tokens, format!("/months/{}/income?welcome=1&signed_in=1", m.id)))
    }
    .await;
    if let Err(e) = &r {
        tracing::info!(ua = %user_agent(&headers), code = e.code(), "registration failed");
    }
    auth_response(&st, &headers, r, "/register")
}

pub async fn logout(State(st): State<Shared>, headers: HeaderMap) -> Sse {
    if let Some(rt) = auth::cookie_value(&headers, auth::REFRESH_COOKIE) {
        let _ = auth::revoke(&st, &rt).await;
    }
    Sse::new().with_cookies(auth::clear_cookies(&st)).redirect("/login")
}

pub async fn logout_all(State(st): State<Shared>, Extension(user): Extension<AuthUser>) -> Sse {
    let _ = auth::revoke_all(&st, &user.login().id).await;
    Sse::new().with_cookies(auth::clear_cookies(&st)).redirect("/login")
}

// ----------------------------------------------------------------------
// Settings
// ----------------------------------------------------------------------

pub async fn change_currency(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    match st.change_currency(&user, field(&f, "currency"), field(&f, "rate")).await {
        Ok(()) => Sse::new().redirect("/settings?saved=currency"),
        Err(e) => failed(&st, &user, &headers, &View::Settings, &e).await,
    }
}

pub async fn change_timezone(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    let tz = field(&f, "timezone");
    let r = if crate::service::valid_timezone(tz) {
        st.store.set_timezone(&user.id, tz).await.map_err(AppError::from)
    } else {
        Err(AppError::bad(t("err.timezone")))
    };
    let user = st.store.user_by_id(&user.id).await.ok().flatten().unwrap_or(user);
    finish(&st, &user, &headers, &View::Settings, r, |()| vec![toast(ToastKind::Success, &t("settings.timezone_saved"), None)]).await
}

pub async fn add_member(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let r = st.add_member(&user, field(&f, "email"), f.get("password").map_or("", String::as_str)).await;
    finish(&st, &user.0, &headers, &View::Settings, r, |m| vec![toast(ToastKind::Success, &tf("household.added", &[("email", &m.email)]), None)]).await
}

pub async fn remove_member(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>) -> Sse {
    let r = st.remove_member(&user, &id).await;
    finish(&st, &user.0, &headers, &View::Settings, r, |()| vec![toast(ToastKind::Success, &t("household.removed"), None)]).await
}

pub async fn change_password(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let get = |k: &str| f.get(k).map_or("", String::as_str);
    let r = st.change_password(&user, get("current"), get("new")).await;
    finish(&st, &user.0, &headers, &View::Settings, r, |()| vec![toast(ToastKind::Success, &t("password.changed"), None)]).await
}

// ----------------------------------------------------------------------
// Months
// ----------------------------------------------------------------------

pub async fn create_month(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, View::Months { archived: false });
    let r = async {
        let ym = parse_year_month(field(&f, "year_month"))?;
        let mode = CopyMode::parse(field(&f, "copy_mode")).unwrap_or(CopyMode::Blank);
        let source = opt_id(&f, "source_month_id");
        st.create_month(&user, ym, mode, source.as_ref()).await
    }
    .await;
    match r {
        Ok(m) => Sse::new().redirect(&format!("/months/{}", m.id)),
        Err(e) => failed(&st, &user, &headers, &view, &e).await,
    }
}

#[derive(Deserialize)]
pub struct ImportReq {
    snapshot: serde_json::Value,
    #[serde(default)]
    replace: bool,
}

pub async fn import_month(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Json(r): Json<ImportReq>) -> Json<serde_json::Value> {
    let user = user.0;
    let res = match serde_json::from_value::<crate::service::Snapshot>(r.snapshot) {
        Ok(snap) => st.restore(&user, snap, r.replace).await,
        Err(_) => Err(AppError::bad(t("err.snapshot_format"))),
    };
    Json(match res {
        Ok(m) => serde_json::json!({ "redirect": format!("/months/{}", m.id) }),
        Err(e) => serde_json::json!({ "error": e.human(&user.currency, None), "technical": e.technical() }),
    })
}

pub async fn archive_month(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Months { archived: false });
    let r = st.archive(&user.0, &id, true).await;
    finish(&st, &user.0, &headers, &view, r, |()| vec![toast(ToastKind::Success, &t("months.archived_toast"), None)]).await
}

pub async fn restore_month(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Months { archived: true });
    let r = st.archive(&user.0, &id, false).await;
    finish(&st, &user.0, &headers, &view, r, |()| vec![toast(ToastKind::Success, &t("months.restored_toast"), None)]).await
}

pub async fn delete_month(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Months { archived: false });
    let r = st.delete_month(&user.0, &id, field(&f, "confirm")).await;
    finish(&st, &user.0, &headers, &view, r, |()| vec![toast(ToastKind::Success, &t("months.deleted_toast"), None)]).await
}

/// Runs a domain op on a month and re-renders.
pub(super) async fn month_action<T>(
    st: &Shared,
    user: &UserRecord,
    headers: &HeaderMap,
    view: View,
    month: AppResult<Id>,
    op: impl FnOnce(&mut Month) -> Result<T, DomainError>,
    toasts: impl FnOnce(&T, &Month) -> Vec<Markup>,
) -> Sse {
    let r = match month {
        Ok(mid) => st.mutate(user, &mid, op).await,
        Err(e) => Err(e),
    };
    match r {
        Ok((v, m)) => {
            let ts = toasts(&v, &m);
            done(st, user, headers, &view, ts).await
        }
        Err(e) => failed(st, user, headers, &view, &e).await,
    }
}

fn no_toasts<T>(_: &T, _: &Month) -> Vec<Markup> {
    Vec::new()
}

pub async fn lock_month(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Overview { month: id.clone() });
    month_action(&st, &user.0, &headers, view, Ok(id), Month::lock, |_, _| vec![toast(ToastKind::Success, &t("status.locked_toast"), None)]).await
}

pub async fn begin_reassign(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Overview { month: id.clone() });
    month_action(&st, &user.0, &headers, view, Ok(id), Month::begin_reassignment, no_toasts).await
}

pub async fn finish_reassign(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Overview { month: id.clone() });
    month_action(&st, &user.0, &headers, view, Ok(id), Month::finish_reassignment, |_, _| vec![toast(ToastKind::Success, &t("status.relocked_toast"), None)]).await
}

// ----------------------------------------------------------------------
// Income
// ----------------------------------------------------------------------

fn schedule_from_form(f: &HashMap<String, String>) -> AppResult<Schedule> {
    let parse_date = |k: &str| chrono::NaiveDate::parse_from_str(field(f, k), "%Y-%m-%d").map_err(|_| AppError::bad(t("err.date")));
    let days = || -> AppResult<Vec<u8>> {
        field(f, "days")
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<u8>().map_err(|_| AppError::bad(t("err.days"))))
            .collect()
    };
    Ok(match field(f, "kind") {
        "one_off" => Schedule::OneOff { dates: vec![parse_date("date")?] },
        "weekly" => Schedule::Recurring { recurrence_rule: Recurrence::Weekly { anchor: parse_date("anchor")? } },
        "biweekly" => Schedule::Recurring { recurrence_rule: Recurrence::Biweekly { anchor: parse_date("anchor")? } },
        "semi_monthly" => {
            let d = days()?;
            if d.len() != 2 {
                return Err(AppError::bad(t("err.semi_days")));
            }
            Schedule::Recurring { recurrence_rule: Recurrence::SemiMonthly { days: [d[0], d[1]] } }
        }
        "monthly" => Schedule::Recurring { recurrence_rule: Recurrence::Monthly { days: days()? } },
        _ => return Err(AppError::bad(t("err.schedule"))),
    })
}

pub async fn add_income(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, View::Income { month: id.clone(), welcome: false });
    let input = money_field(&f, "amount").and_then(|a| schedule_from_form(&f).map(|s| (a, s)));
    let r = match input {
        Ok((amount, schedule)) => st.mutate(&user, &id, |m| m.add_income_line(field(&f, "name"), amount, schedule)).await,
        Err(e) => Err(e),
    };
    match r {
        Ok((lid, m)) => {
            // First paycheck of the month: go straight to funding it (§4 onboarding).
            let first = m.paychecks_by_date().into_iter().find(|p| p.income_line_id == lid).map(|p| p.id.clone());
            let msg = tf("income.added_toast", &[("count", &m.paychecks.iter().filter(|p| p.income_line_id == lid).count().to_string())]);
            if m.income_lines.len() == 1 {
                if let Some(p) = first {
                    return Sse::new().redirect(&format!("/months/{id}/paychecks/{p}"));
                }
            }
            done(&st, &user, &headers, &view, vec![toast(ToastKind::Success, &msg, None)]).await
        }
        Err(e) => failed(&st, &user, &headers, &view, &e).await,
    }
}

pub async fn update_income(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let mid = st.resolve(&user, Owner::IncomeLine, &id).await;
    let view = view_of(&f, View::Income { month: mid.as_ref().cloned().unwrap_or_default(), welcome: false });
    let input = money_field(&f, "amount").and_then(|a| schedule_from_form(&f).map(|s| (a, s)));
    let (amount, schedule) = match input {
        Ok(v) => v,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let u = user.clone();
    month_action(&st, &user, &headers, view, mid, |m| m.update_income_line(&id, Some(field(&f, "name")), Some(amount), Some(schedule)), move |i, m| {
        let mut v = impact_toasts(&u, m, i);
        v.push(toast(ToastKind::Success, &t("common.saved"), None));
        v
    })
    .await
}

pub async fn delete_income(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let mid = st.resolve(&user, Owner::IncomeLine, &id).await;
    let view = view_of(&f, View::Income { month: mid.as_ref().cloned().unwrap_or_default(), welcome: false });
    let u = user.clone();
    month_action(&st, &user, &headers, view, mid, |m| m.delete_income_line(&id), move |i, m| impact_toasts(&u, m, i)).await
}

pub async fn add_paycheck(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let mid = st.resolve(&user, Owner::IncomeLine, &id).await;
    let view = view_of(&f, View::Income { month: mid.as_ref().cloned().unwrap_or_default(), welcome: false });
    let date = match chrono::NaiveDate::parse_from_str(field(&f, "date"), "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return failed(&st, &user, &headers, &view, &AppError::bad(t("err.date"))).await,
    };
    month_action(&st, &user, &headers, view, mid, |m| m.add_paycheck(&id, date, None), |_, _| {
        vec![toast(ToastKind::Success, &t("income.paycheck_added"), None)]
    })
    .await
}

#[derive(Deserialize)]
pub struct SuggestQuery {
    #[serde(default)]
    q: String,
}

/// Smart suggestions while typing an income line name (spec §2.2).
pub async fn income_suggestions(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>, Query(q): Query<SuggestQuery>) -> Sse {
    let user = user.0;
    let (Ok(loaded), Ok(history)) = (st.load(&user, &id).await, st.income_history(&user).await) else {
        return Sse::new().patch(html! { div id="income-suggestions" data-show="$_sugoff === false" {} });
    };
    let list = suggest_income(&q.q, &history, loaded.month.year_month, 4);
    let js_str = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    Sse::new().patch(html! {
        div id="income-suggestions" class="suggest-box" data-show="$_sugoff === false" {
            @if !list.is_empty() {
                p class="hint" { (t("income.suggest_intro")) }
                ul class="suggest-list" {
                    @for s in &list {
                        @let (kind, date, anchor, days) = match &s.schedule {
                            Schedule::OneOff { dates } => ("one_off", dates.first().map(ToString::to_string).unwrap_or_default(), String::new(), String::new()),
                            Schedule::Recurring { recurrence_rule: Recurrence::Weekly { anchor } } => ("weekly", String::new(), anchor.to_string(), String::new()),
                            Schedule::Recurring { recurrence_rule: Recurrence::Biweekly { anchor } } => ("biweekly", String::new(), anchor.to_string(), String::new()),
                            Schedule::Recurring { recurrence_rule: Recurrence::SemiMonthly { days } } => ("semi_monthly", String::new(), String::new(), format!("{}, {}", days[0], days[1])),
                            Schedule::Recurring { recurrence_rule: Recurrence::Monthly { days } } => ("monthly", String::new(), String::new(), days.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")),
                        };
                        @let js = format!(
                            "pz.fillIncome({{name: {}, amount: {}, kind: {}, date: {}, anchor: {}, days: {}}}); $_newkind = {}; $_sugoff = document.getElementById('new-name').value",
                            js_str(&s.name), js_str(&crate::money::format(s.planned_amount, &user.currency)), js_str(kind), js_str(&date), js_str(&anchor), js_str(&days), js_str(kind)
                        );
                        li {
                            button type="button" class="suggest" data-on:click=(js) {
                                strong { (s.name) } " · " (crate::money::format(s.planned_amount, &user.currency))
                                span class="muted small" { " " (tf("income.suggest_from", &[("month", &month_label(s.source_month))])) }
                            }
                        }
                    }
                }
                button type="button" class="link small" data-on:click="$_sugoff = document.getElementById('new-name').value" { (t("income.suggest_dismiss")) }
            }
        }
    })
}

// ----------------------------------------------------------------------
// Paychecks
// ----------------------------------------------------------------------

async fn paycheck_view(st: &Shared, user: &UserRecord, f: &HashMap<String, String>, pid: &Id) -> (AppResult<Id>, View) {
    let mid = st.resolve(user, Owner::Paycheck, pid).await;
    let view = view_of(f, View::Paycheck { month: mid.as_ref().cloned().unwrap_or_default(), paycheck: pid.clone() });
    (mid, view)
}

pub async fn paycheck_planned(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = paycheck_view(&st, &user, &f, &pid).await;
    let amount = match money_field(&f, "amount") {
        Ok(a) => a,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let u = user.clone();
    month_action(&st, &user, &headers, view, mid, |m| m.set_paycheck_planned(&pid, amount), move |i, m| impact_toasts(&u, m, i)).await
}

pub async fn paycheck_actual(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = paycheck_view(&st, &user, &f, &pid).await;
    let amount = match opt_money_field(&f, "amount") {
        Ok(a) => a,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    month_action(&st, &user, &headers, view, mid, |m| m.set_paycheck_actual(&pid, amount), |_, _| vec![toast(ToastKind::Success, &t("paycheck.actual_saved"), None)]).await
}

pub async fn paycheck_status(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = paycheck_view(&st, &user, &f, &pid).await;
    let Some(status) = PaycheckStatus::parse(field(&f, "status")) else {
        return failed(&st, &user, &headers, &view, &AppError::bad(t("err.status"))).await;
    };
    let u = user.clone();
    month_action(&st, &user, &headers, view, mid, |m| m.set_paycheck_status(&pid, status), move |i, m| impact_toasts(&u, m, i)).await
}

pub async fn apply_actual(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = paycheck_view(&st, &user, &f, &pid).await;
    let u = user.clone();
    month_action(&st, &user, &headers, view, mid, |m| m.apply_actual(&pid), move |i, m| {
        let mut v = impact_toasts(&u, m, i);
        v.push(toast(ToastKind::Success, &t("variance.applied"), None));
        v
    })
    .await
}

/// One-click 10% giving to Giving → Tithe (user request).
pub async fn give(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = paycheck_view(&st, &user, &f, &pid).await;
    let cur = user.currency.clone();
    let pid2 = pid.clone();
    month_action(&st, &user, &headers, view, mid, |m| m.give_percent(&pid, 10), move |_, m| {
        vec![toast(ToastKind::Success, &tf("give.toast", &[("amount", &crate::money::format(m.tithe_amount(&pid2, 10), &cur))]), None)]
    })
    .await
}

pub async fn delete_paycheck(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, _) = paycheck_view(&st, &user, &f, &pid).await;
    let view = View::Income { month: mid.as_ref().cloned().unwrap_or_default(), welcome: false };
    let u = user.clone();
    let r = match &mid {
        Ok(id) => st.mutate(&user, id, |m| m.delete_paycheck(&pid)).await,
        Err(_) => Err(AppError::NotFound),
    };
    match r {
        // The paycheck is gone: navigate to the month's default paycheck.
        Ok((i, m)) => {
            let target = m.default_paycheck(st.today_for(&u)).map_or_else(|| view.url(), |p| format!("/months/{}/paychecks/{p}", m.id));
            let mut sse = Sse::new();
            if let Some(tt) = impact_toasts(&u, &m, &i).into_iter().next() {
                sse = sse.script(&format!("sessionStorage.setItem('pz-toast', {})", serde_json::to_string(&tt.into_string()).unwrap_or_default()));
            }
            sse.redirect(&target)
        }
        Err(e) => failed(&st, &user, &headers, &view, &e).await,
    }
}

/// Inline Planned edit on the paycheck view: sets this paycheck's
/// allocation to a line (0 or blank removes it).
pub async fn set_allocation(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path((pid, lid)): Path<(Id, Id)>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = paycheck_view(&st, &user, &f, &pid).await;
    let amount = match opt_money_field(&f, "amount") {
        Ok(a) => a.unwrap_or(Cents::ZERO),
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    if field(&f, "sheet") != "line" {
        return month_action(&st, &user, &headers, view, mid, |m| m.set_allocation(&pid, &lid, amount), no_toasts).await;
    }
    let r = match mid {
        Ok(mid) => st.mutate(&user, &mid, |m| m.set_allocation(&pid, &lid, amount)).await.map(|_| ()),
        Err(e) => Err(e),
    };
    line_sheet_result(&st, &user, &headers, &view, &lid, r, Vec::new()).await
}

/// After a funding change made in a line's sheet: the screen and the sheet
/// both show the new numbers, and the sheet stays open.
async fn line_sheet_result(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View, lid: &Id, r: AppResult<()>, toasts: Vec<Markup>) -> Sse {
    let base = match r {
        Ok(()) => {
            let mut sse = content_sse(st, user, headers, view).await;
            for t in toasts {
                sse = sse.patch_into("#toasts", "append", t);
            }
            sse
        }
        Err(e) => failed(st, user, headers, view, &e).await,
    };
    base.append(super::sheets::line_sse(st, user, headers, lid, &Some(view.encode()), None).await)
}

/// Moves part of a line's funding from one paycheck to another in one step.
pub async fn move_funding(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(lid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = owner_view(&st, &user, &f, Owner::ExpenseLine, &lid).await;
    let (from, to) = (Id::new(field(&f, "from")), Id::new(field(&f, "to")));
    let amount = match money_field(&f, "amount") {
        Ok(a) => a,
        Err(e) => return line_sheet_result(&st, &user, &headers, &view, &lid, Err(e), Vec::new()).await,
    };
    let r = match mid {
        Ok(mid) => st.mutate(&user, &mid, |m| m.transfer(&from, &to, &lid, amount)).await,
        Err(e) => Err(e),
    };
    let msg = r.as_ref().ok().map(|(_, m)| {
        let date = |p: &Id| m.paycheck(p).map(|p| short_date(p.date)).unwrap_or_default();
        tf("line.moved", &[("amount", &crate::money::format(amount, &user.currency)), ("from", &date(&from)), ("to", &date(&to))])
    });
    let toasts = msg.map(|m| vec![toast(ToastKind::Success, &m, None)]).unwrap_or_default();
    line_sheet_result(&st, &user, &headers, &view, &lid, r.map(|_| ()), toasts).await
}

/// "Assign money from this paycheck": adds to a line (creating it if asked).
pub async fn fund(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let pid = Id::new(field(&f, "paycheck_id"));
    let view = view_of(&f, View::Paycheck { month: id.clone(), paycheck: pid.clone() });
    let amount = match money_field(&f, "amount") {
        Ok(a) if a.is_positive() => a,
        Ok(_) => return failed(&st, &user, &headers, &view, &AppError::Domain(DomainError::NonPositiveAmount)).await,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let line = field(&f, "line_id").to_string();
    let new_name = field(&f, "new_name").to_string();
    let new_cat = Id::new(field(&f, "new_category"));
    let (bal, min) = match (opt_money_field(&f, "current_balance"), opt_money_field(&f, "minimum_payment")) {
        (Ok(b), Ok(m)) => (b, m),
        (Err(e), _) | (_, Err(e)) => return failed(&st, &user, &headers, &view, &e).await,
    };
    month_action(&st, &user, &headers, view, Ok(id), move |m| {
        let lid = if line == "__new" {
            let lid = m.add_expense_line(&new_cat, &new_name)?;
            if (bal.is_some() || min.is_some()) && m.is_debt_line(&lid) {
                m.set_debt_fields(&lid, bal, min)?;
            }
            lid
        } else {
            Id::new(line)
        };
        let existing = m.allocation_for(&pid, &lid).map_or(Cents::ZERO, |a| a.amount);
        m.set_allocation(&pid, &lid, existing + amount)?;
        Ok(lid)
    }, |lid, m| {
        let name = m.expense_line(lid).map(|l| l.name.clone()).unwrap_or_default();
        vec![toast(ToastKind::Success, &tf("fund.done", &[("name", &name)]), None)]
    })
    .await
}

// ----------------------------------------------------------------------
// Categories & lines
// ----------------------------------------------------------------------

async fn owner_view(st: &Shared, user: &UserRecord, f: &HashMap<String, String>, kind: Owner, id: &Id) -> (AppResult<Id>, View) {
    let mid = st.resolve(user, kind, id).await;
    let view = view_of(f, View::Overview { month: mid.as_ref().cloned().unwrap_or_default() });
    (mid, view)
}

pub async fn add_category(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Overview { month: id.clone() });
    month_action(&st, &user.0, &headers, view, Ok(id), |m| m.add_category(field(&f, "name"), CategoryKind::Standard), no_toasts).await
}

pub async fn rename_category(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let (mid, view) = owner_view(&st, &user.0, &f, Owner::Category, &id).await;
    month_action(&st, &user.0, &headers, view, mid, |m| m.rename_category(&id, field(&f, "name")), no_toasts).await
}

pub async fn move_category(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let (mid, view) = owner_view(&st, &user.0, &f, Owner::Category, &id).await;
    month_action(&st, &user.0, &headers, view, mid, |m| m.move_category(&id, field(&f, "direction") == "up"), no_toasts).await
}

pub async fn delete_category(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let (mid, view) = owner_view(&st, &user.0, &f, Owner::Category, &id).await;
    month_action(&st, &user.0, &headers, view, mid, |m| m.delete_category(&id), |_, m| {
        if m.is_zero() { vec![] } else { vec![toast(ToastKind::Warning, &t("impact.deleted_funding"), None)] }
    })
    .await
}

/// Drag-and-drop placement of a line or category (overview).
pub async fn place(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Overview { month: id.clone() });
    let target = Id::new(field(&f, "id"));
    let category = Id::new(field(&f, "category_id"));
    let index: usize = field(&f, "index").parse().unwrap_or(0);
    let kind = field(&f, "kind").to_string();
    month_action(&st, &user.0, &headers, view, Ok(id), move |m| {
        if kind == "category" { m.place_category(&target, index) } else { m.place_line(&target, &category, index) }
    }, no_toasts)
    .await
}

pub async fn add_line(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let view = view_of(&f, View::Overview { month: id.clone() });
    let cat_field = field(&f, "category_id").to_string();
    let new_cat = field(&f, "new_category").to_string();
    let amount = match opt_money_field(&f, "amount") {
        Ok(a) => a.filter(|a| a.is_positive()),
        Err(e) => return failed(&st, &user.0, &headers, &view, &e).await,
    };
    let paycheck = opt_id(&f, "paycheck_id");
    let (bal, min) = match (opt_money_field(&f, "current_balance"), opt_money_field(&f, "minimum_payment")) {
        (Ok(b), Ok(m)) => (b, m),
        (Err(e), _) | (_, Err(e)) => return failed(&st, &user.0, &headers, &view, &e).await,
    };
    month_action(&st, &user.0, &headers, view, Ok(id), |m| {
        let cat = if cat_field == "__new" { m.add_category(&new_cat, CategoryKind::Standard)? } else { Id::new(cat_field) };
        let lid = m.add_expense_line(&cat, field(&f, "name"))?;
        // The form only offers these for a Debt category.
        if (bal.is_some() || min.is_some()) && m.is_debt_line(&lid) {
            m.set_debt_fields(&lid, bal, min)?;
        }
        if let (Some(a), Some(p)) = (amount, paycheck.as_ref()) {
            m.set_allocation(p, &lid, a)?;
        }
        Ok(lid)
    }, no_toasts)
    .await
}

/// Line sheet: rename and/or move to another category in one save.
pub async fn edit_line(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let (mid, view) = owner_view(&st, &user.0, &f, Owner::ExpenseLine, &id).await;
    let cat = opt_id(&f, "category_id");
    let pos = cat.as_ref().and_then(|c| field(&f, &format!("pos_{c}")).parse::<usize>().ok());
    month_action(&st, &user.0, &headers, view, mid, |m| {
        m.rename_expense_line(&id, field(&f, "name"))?;
        match (&cat, pos) {
            (Some(c), Some(p)) => m.place_line(&id, c, p)?,
            (Some(c), None) => m.set_line_category(&id, c)?,
            _ => {}
        }
        Ok(())
    }, |_, _| vec![toast(ToastKind::Success, &t("common.saved"), None)])
    .await
}

pub async fn rename_line(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let (mid, view) = owner_view(&st, &user.0, &f, Owner::ExpenseLine, &id).await;
    month_action(&st, &user.0, &headers, view, mid, |m| m.rename_expense_line(&id, field(&f, "name")), no_toasts).await
}

/// Monthly-overview Planned edit, translated into allocations (spec §14.6).
pub async fn line_planned(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = owner_view(&st, &user, &f, Owner::ExpenseLine, &id).await;
    let amount = match opt_money_field(&f, "amount") {
        Ok(a) => a.unwrap_or(Cents::ZERO),
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    month_action(&st, &user, &headers, view, mid, |m| m.set_line_planned(&id, amount), no_toasts).await
}

pub async fn line_debt(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let (mid, view) = owner_view(&st, &user, &f, Owner::ExpenseLine, &id).await;
    let (bal, min) = match (opt_money_field(&f, "current_balance"), opt_money_field(&f, "minimum_payment")) {
        (Ok(b), Ok(m)) => (b, m),
        (Err(e), _) | (_, Err(e)) => return failed(&st, &user, &headers, &view, &e).await,
    };
    month_action(&st, &user, &headers, view, mid, |m| m.set_debt_fields(&id, bal, min), no_toasts).await
}

pub async fn move_line(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let (mid, view) = owner_view(&st, &user.0, &f, Owner::ExpenseLine, &id).await;
    month_action(&st, &user.0, &headers, view, mid, |m| m.move_expense_line(&id, field(&f, "direction") == "up"), no_toasts).await
}

pub async fn delete_line(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let (mid, view) = owner_view(&st, &user.0, &f, Owner::ExpenseLine, &id).await;
    month_action(&st, &user.0, &headers, view, mid, |m| m.delete_expense_line(&id), |_, m| {
        if m.is_zero() { vec![] } else { vec![toast(ToastKind::Warning, &t("impact.deleted_funding"), None)] }
    })
    .await
}

// ----------------------------------------------------------------------
// Transactions
// ----------------------------------------------------------------------


/// Warning toasts for lines a transaction change pushed (further) over plan.
pub(super) fn overspend_toasts(user: &UserRecord, before: &[(Id, Cents)], m: &Month) -> Vec<Markup> {
    m.expense_lines
        .iter()
        .filter_map(|l| {
            let over = m.line_spent(&l.id) - m.line_planned(&l.id);
            let was = before.iter().find(|(id, _)| id == &l.id).map_or(Cents::ZERO, |(_, o)| *o);
            (over.is_positive() && over > was).then(|| {
                toast(ToastKind::Warning, &tf("over.toast", &[("name", &l.name), ("amount", &crate::money::format(over, &user.currency))]), None)
            })
        })
        .collect()
}

pub(super) fn overs(m: &Month) -> Vec<(Id, Cents)> {
    m.expense_lines.iter().map(|l| (l.id.clone(), m.line_spent(&l.id) - m.line_planned(&l.id))).collect()
}

pub async fn add_transaction(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, View::Transactions { month: id.clone(), filter: TxFilter::All });
    let entry = match entry_from_form(&f, Id::generate(), &user.currency) {
        Ok(e) => e,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let u = user.clone();
    month_action(&st, &user, &headers, view, Ok(id), |m| {
        let b = overs(m);
        match entry {
            TxEntry::Single(tx) => m.add_transaction(tx).map(|_| (b, false)),
            TxEntry::Split(sp) => m.save_split(None, sp.date, sp.payee, sp.notes, sp.account, sp.parts).map(|_| (b, true)),
        }
    }, move |(b, split), m| {
        let mut v = vec![toast(ToastKind::Success, &t(if *split { "split.saved" } else { "tx.saved" }), None)];
        v.extend(overspend_toasts(&u, b, m));
        v
    }).await
}

/// Saves an edit from the unified form. Works on plain and split
/// transactions, and converts between them when parts are added or removed.
pub async fn update_transaction(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let mid = st.resolve(&user, Owner::Transaction, &id).await;
    let view = view_of(&f, View::Transactions { month: mid.as_ref().cloned().unwrap_or_default(), filter: TxFilter::All });
    let entry = match entry_from_form(&f, id.clone(), &user.currency) {
        Ok(e) => e,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let u = user.clone();
    month_action(&st, &user, &headers, view, mid, |m| {
        let b = overs(m);
        let group = m.transaction(&id).ok_or(DomainError::NotFound { kind: "transaction", id: id.clone() })?.split_group.clone();
        match (group, entry) {
            (None, TxEntry::Single(tx)) => m.update_transaction(tx)?,
            (Some(g), TxEntry::Split(sp)) => {
                m.save_split(Some(&g), sp.date, sp.payee, sp.notes, sp.account, sp.parts)?;
            }
            (None, TxEntry::Split(sp)) => {
                m.delete_transaction(&id)?;
                m.save_split(None, sp.date, sp.payee, sp.notes, sp.account, sp.parts)?;
            }
            (Some(g), TxEntry::Single(mut tx)) => {
                m.delete_split(&g)?;
                tx.id = Id::generate();
                m.add_transaction(tx)?;
            }
        }
        Ok(b)
    }, move |b, m| {
        let mut v = vec![toast(ToastKind::Success, &t("tx.saved"), None)];
        v.extend(overspend_toasts(&u, b, m));
        v
    }).await
}

pub async fn delete_transaction(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let mid = st.resolve(&user, Owner::Transaction, &id).await;
    let view = view_of(&f, View::Transactions { month: mid.as_ref().cloned().unwrap_or_default(), filter: TxFilter::All });
    month_action(&st, &user, &headers, view, mid, |m| m.delete_transaction(&id), |_, _| vec![toast(ToastKind::Success, &t("tx.deleted"), None)]).await
}

/// Counts a transaction (every part of a split) in another month's budget,
/// keeping its date: a paycheck on the 30th can fund next month.
pub async fn move_transaction(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let src = st.resolve(&user, Owner::Transaction, &id).await;
    let view = view_of(&f, View::Transactions { month: src.as_ref().cloned().unwrap_or_default(), filter: TxFilter::All });
    let (src, dest) = match (src, opt_id(&f, "month_id")) {
        (Ok(s), Some(d)) => (s, d),
        (Err(e), _) => return failed(&st, &user, &headers, &view, &e).await,
        (_, None) => return failed(&st, &user, &headers, &view, &AppError::bad(t("tx.err_pick_month"))).await,
    };
    if src == dest {
        return done(&st, &user, &headers, &view, vec![]).await;
    }
    let r = st.mutate_months(&user, &[src, dest], |ms| {
        let (a, b) = ms.split_at_mut(1);
        let parts = a[0].take_transaction(&id)?;
        b[0].receive_transactions(&a[0], parts)
    }).await;
    match r {
        Ok(((), ms)) => {
            let msg = tf("tx.moved", &[("month", &month_label(ms[1].year_month))]);
            done(&st, &user, &headers, &view, vec![toast(ToastKind::Success, &msg, None)]).await
        }
        Err(e) => failed(&st, &user, &headers, &view, &e).await,
    }
}

/// Marks a payment as money moved to another account (e.g. paying a credit
/// card), so it no longer counts as spending next to the card's purchases.
/// If the other account already shows the money arriving (same or a
/// neighboring month), that copy is merged away.
pub async fn mark_transfer(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let src = st.resolve(&user, Owner::Transaction, &id).await;
    let view = view_of(&f, View::Transactions { month: src.as_ref().cloned().unwrap_or_default(), filter: TxFilter::All });
    let src = match src {
        Ok(s) => s,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let Some(to) = opt_id(&f, "to") else { return failed(&st, &user, &headers, &view, &AppError::bad(t("transfer.err_pick_to"))).await };
    let from = opt_id(&f, "from");
    if let Err(e) = st.check_accounts(&user, &[Some(&to), from.as_ref()]).await {
        return failed(&st, &user, &headers, &view, &e).await;
    }
    // The neighboring months the other side may have landed in.
    let metas = match st.store.list_months(&user.id, false).await {
        Ok(m) => m,
        Err(e) => return failed(&st, &user, &headers, &view, &e.into()).await,
    };
    let mut months = vec![src.clone()];
    if let Some(ym) = metas.iter().find(|x| x.id == src).map(|x| x.year_month) {
        let near = [recurrence::first_of_month(ym - chrono::Duration::days(1)), ym + chrono::Months::new(1)];
        months.extend(metas.iter().filter(|x| near.contains(&x.year_month)).map(|x| x.id.clone()));
    }
    let r = st.mutate_months(&user, &months, |ms| {
        let mut x = ms[0].transaction(&id).cloned().ok_or(DomainError::NotFound { kind: "transaction", id: id.clone() })?;
        if x.split_group.is_some() || !x.amount.is_negative() {
            return Err(DomainError::InvalidTransfer);
        }
        if let Some(a) = from {
            x.account_id = Some(a);
        }
        x.transfer_account_id = Some(to.clone());
        x.expense_line_id = None;
        x.paycheck_id = None;
        ms[0].update_transaction(x.clone())?;
        for m in ms.iter_mut() {
            if let Some(mirror) = paycheckzero_core::bank::mirror_of(m, &x, &to) {
                m.delete_transaction(&mirror)?;
                return Ok(true);
            }
        }
        Ok(false)
    }).await;
    match r {
        Ok((merged, _)) => {
            let mut msg = t("transfer.marked");
            if merged {
                msg = format!("{msg} {}", t("transfer.merged_other_side"));
            }
            done(&st, &user, &headers, &view, vec![toast(ToastKind::Success, &msg, None)]).await
        }
        Err(e) => failed(&st, &user, &headers, &view, &e).await,
    }
}

// ----------------------------------------------------------------------
// Split transactions (user request #9)
// ----------------------------------------------------------------------

struct SplitInput {
    date: chrono::NaiveDate,
    payee: Option<String>,
    notes: Option<String>,
    account: Option<Id>,
    parts: Vec<SplitPart>,
}

/// What the unified transaction form describes.
enum TxEntry {
    Single(Transaction),
    Split(SplitInput),
}

/// Reads the unified transaction form: one part row means a plain
/// transaction; two or more mean a split whose parts must add up to Amount.
fn entry_from_form(f: &HashMap<String, String>, id: Id, currency: &str) -> AppResult<TxEntry> {
    let date = chrono::NaiveDate::parse_from_str(field(f, "date"), "%Y-%m-%d").map_err(|_| AppError::bad(t("err.date")))?;
    let income = field(f, "direction") == "income";
    let sign = |c: Cents| if income { c } else { -c };
    let text = |k: &str| {
        let v = field(f, k);
        (!v.is_empty()).then(|| v.to_string())
    };
    let mut idx: Vec<usize> = f.keys().filter_map(|k| k.strip_prefix("part_line_").and_then(|n| n.parse().ok())).collect();
    idx.sort_unstable();
    let total = money_field(f, "amount")?;
    if idx.len() <= 1 {
        let i = idx.first().copied().unwrap_or(0);
        let line = opt_id(f, &format!("part_line_{i}")).or_else(|| opt_id(f, "expense_line_id"));
        let paycheck = opt_id(f, &format!("part_paycheck_{i}")).or_else(|| opt_id(f, "paycheck_id"));
        return Ok(TxEntry::Single(Transaction {
            id,
            date,
            amount: sign(total),
            payee: text("payee"),
            notes: text("notes"),
            expense_line_id: line,
            paycheck_id: paycheck,
            split_group: None,
            account_id: opt_id(f, "account_id"),
            transfer_account_id: None,
            external_id: None,
        }));
    }
    let mut parts = Vec::new();
    for i in idx {
        let amount = opt_money_field(f, &format!("part_amount_{i}"))?.unwrap_or(Cents::ZERO);
        if amount.is_zero() {
            continue;
        }
        parts.push(SplitPart {
            amount: sign(amount),
            expense_line_id: opt_id(f, &format!("part_line_{i}")),
            paycheck_id: opt_id(f, &format!("part_paycheck_{i}")),
        });
    }
    let sum: Cents = parts.iter().map(|p| p.amount.abs()).sum();
    if sum != total {
        return Err(AppError::bad(tf("err.split_total", &[
            ("parts", &crate::money::format(sum, currency)),
            ("total", &crate::money::format(total, currency)),
        ])));
    }
    Ok(TxEntry::Split(SplitInput { date, payee: text("payee"), notes: text("notes"), account: opt_id(f, "account_id"), parts }))
}

pub async fn delete_split(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(group): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let part = Id::new(field(&f, "part_of"));
    let mid = st.resolve(&user, Owner::Transaction, &part).await;
    let view = view_of(&f, View::Transactions { month: mid.as_ref().cloned().unwrap_or_default(), filter: TxFilter::All });
    month_action(&st, &user, &headers, view, mid, |m| m.delete_split(&group), |_, _| vec![toast(ToastKind::Success, &t("split.deleted"), None)]).await
}
