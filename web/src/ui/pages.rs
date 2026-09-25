//! Page handlers and view renderers.

use super::*;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::export;
use crate::sse::Sse;
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Json};
use paycheckzero_core::report::{self, Comparison, Figures, SpendingStatus};
use paycheckzero_core::suggest::{variance_suggestions, SuggestionKind};
use paycheckzero_core::*;
use paycheckzero_storage::MonthMeta;
use serde::Deserialize;

type Page = AppResult<Response>;

pub fn ctx(st: &Shared, user: &UserRecord, headers: &HeaderMap) -> Ctx {
    Ctx {
        user: user.clone(),
        today: st.today_for(user),
        collapsed: collapsed_from(headers),
        only_funded: crate::auth::cookie_value(headers, "pz_only_funded").is_some_and(|v| v == "1"),
    }
}

/// Renders an error as a recoverable in-page state.
fn page_error(e: &AppError, currency: &str) -> Markup {
    html! {
        div class="error-state" role="alert" {
            h2 { (t("error.load_title")) }
            p { (e.human(currency, None)) }
            details class="tech" { summary { (t("common.technical_details")) } code { (e.technical()) } }
            a class="btn" href="/months" { (t("error.back_to_months")) }
        }
    }
}

async fn header_for(st: &Shared, user: &UserRecord, m: &Month) -> AppResult<MonthHeader> {
    let metas = st.store.list_months(&user.id, false).await?;
    let prev_ym = report::previous_month(m.year_month);
    let next_ym = recurrence::last_of_month(m.year_month).succ_opt().unwrap_or(m.year_month);
    let find = |ym: NaiveDate| metas.iter().find(|x| x.year_month == ym).map(|x| x.id.clone());
    Ok(MonthHeader {
        id: m.id.clone(),
        year_month: m.year_month,
        prev: find(prev_ym),
        next: find(next_ym),
        default_paycheck: m.default_paycheck(st.today_for(user)),
    })
}

/// Renders any view's content (used by content endpoints and after every
/// mutation). Returns a redirect target when the view no longer exists.
pub async fn render_view(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View) -> AppResult<Result<Markup, String>> {
    let c = ctx(st, user, headers);
    Ok(Ok(match view {
        View::Months { archived } => {
            let metas = st.store.list_months(&user.id, *archived).await?;
            let months = st.all_months(user).await?;
            render_months(&c, &metas, &months, *archived)
        }
        View::Settings => render_settings(&c),
        other => {
            let Some(mid) = other.month() else { return Ok(Err("/months".into())) };
            let loaded = match st.load(user, mid).await {
                Ok(l) => l,
                Err(AppError::NotFound) => return Ok(Err("/months".into())),
                Err(e) => return Err(e),
            };
            let m = &loaded.month;
            match other {
                View::Paycheck { paycheck, .. } => {
                    if m.paycheck(paycheck).is_none() {
                        return Ok(Err(format!("/months/{mid}")));
                    }
                    render_paycheck(&c, m, loaded.archived, paycheck)
                }
                View::Overview { .. } => render_overview(&c, m, loaded.archived),
                View::Income { welcome, .. } => render_income(&c, m, loaded.archived, *welcome),
                View::Transactions { .. } => render_transactions(&c, m, loaded.archived),
                View::Reports { q, .. } => {
                    let all = st.all_months(user).await?;
                    render_reports(&c, m, &all, q)
                }
                View::Months { .. } | View::Settings => html! {},
            }
        }
    }))
}

/// SSE response carrying freshly rendered content for `view`.
pub async fn content_sse(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View) -> Sse {
    match render_view(st, user, headers, view).await {
        Ok(Ok(markup)) => Sse::new().patch(content(markup)),
        Ok(Err(redirect)) => Sse::new().redirect(&redirect),
        Err(e) => Sse::new().patch(content(page_error(&e, &user.currency))),
    }
}

async fn month_page(st: &Shared, user: &UserRecord, mid: &Id, view: View, active: &str, title: &str) -> Page {
    let loaded = match st.load(user, mid).await {
        Ok(l) => l,
        Err(AppError::NotFound) => return Ok(Redirect::to("/months").into_response()),
        Err(e) => return Err(e),
    };
    st.store.set_last_month(&user.id, Some(mid)).await?;
    let header = header_for(st, user, &loaded.month).await?;
    let title = format!("{title} · {}", month_label(loaded.month.year_month));
    Ok(layout(Some(user), &title, Some((&header, active)), Some(&view), None).into_response())
}

// ----------------------------------------------------------------------
// Public pages
// ----------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AuthQuery {
    #[serde(default)]
    error: Option<String>,
}

fn auth_query_error(q: &AuthQuery) -> Option<String> {
    q.error.as_deref().map(|code| match code {
        "INVALID_CREDENTIALS" => t("err.invalid_credentials"),
        "REGISTRATION_CLOSED" => t("err.registration_closed"),
        "COOKIE_BLOCKED" => t("auth.cookie_blocked"),
        "BAD_REQUEST" => t("auth.check_fields"),
        _ => t("err.internal"),
    })
}

pub async fn login_page(State(st): State<Shared>, headers: HeaderMap, Query(q): Query<AuthQuery>) -> Page {
    if crate::auth::session_user(&st, &headers).await.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    let open = st.registration_open().await?;
    Ok(layout(None, &t("auth.login_title"), None, None, Some(html! {
        section class="auth-card" {
            h1 { (t("auth.login_title")) }
            p class="muted" { (t("app.tagline")) }
            form id="login-form" method="post" action="/ui/login" data-on:submit__prevent=(post_form("/ui/login")) {
                label for="email" { (t("auth.email")) }
                input id="email" name="email" type="email" autocomplete="email" required;
                label for="password" { (t("auth.password")) }
                input id="password" name="password" type="password" autocomplete="current-password" required;
                div id="auth-error" {
                    @if let Some(e) = auth_query_error(&q) { div class="field-error" role="alert" { (e) } }
                }
                button type="submit" class="btn primary" { (t("auth.login")) }
            }
            @if open {
                p { (t("auth.no_account")) " " a href="/register" { (t("auth.create_account")) } }
            }
        }
    })).into_response())
}

pub async fn register_page(State(st): State<Shared>, Query(q): Query<AuthQuery>) -> Page {
    if !st.registration_open().await? {
        return Ok(Redirect::to("/login").into_response());
    }
    Ok(layout(None, &t("auth.register_title"), None, None, Some(html! {
        section class="auth-card" {
            h1 { (t("auth.register_title")) }
            p class="muted" { (t("auth.register_intro")) }
            form id="register-form" method="post" action="/ui/register" data-on:submit__prevent=(post_form("/ui/register")) {
                label for="email" { (t("auth.email")) }
                input id="email" name="email" type="email" autocomplete="email" required;
                label for="password" { (t("auth.password")) }
                input id="password" name="password" type="password" autocomplete="new-password" minlength=(crate::service::MIN_PASSWORD) required aria-describedby="pw-hint";
                p id="pw-hint" class="hint" { (tf("auth.password_hint", &[("n", &crate::service::MIN_PASSWORD.to_string())])) }
                input type="hidden" name="timezone" data-timezone;
                div id="auth-error" {
                    @if let Some(e) = auth_query_error(&q) { div class="field-error" role="alert" { (e) } }
                }
                button type="submit" class="btn primary" { (t("auth.create_account")) }
            }
            p { (t("auth.have_account")) " " a href="/login" { (t("auth.login")) } }
        }
    })).into_response())
}

pub async fn offline_page() -> Markup {
    layout(None, &t("offline.title"), None, None, Some(html! {
        div class="empty" {
            h1 { (t("offline.title")) }
            p { (t("offline.body")) }
            button type="button" class="btn" data-retry { (t("common.retry")) }
        }
    }))
}

// ----------------------------------------------------------------------
// Entry points
// ----------------------------------------------------------------------

pub async fn home(State(st): State<Shared>, Extension(user): Extension<AuthUser>) -> Page {
    let user = user.0;
    if let Some(last) = &user.last_month_id {
        if let Ok(l) = st.load(&user, last).await {
            if !l.archived {
                return Ok(Redirect::to(&format!("/months/{last}")).into_response());
            }
        }
    }
    let today = st.today_for(&user);
    let metas = st.store.list_months(&user.id, false).await?;
    let pick = metas
        .iter()
        .find(|m| m.year_month == recurrence::first_of_month(today))
        .or_else(|| metas.first());
    Ok(match pick {
        Some(m) => Redirect::to(&format!("/months/{}", m.id)).into_response(),
        None => Redirect::to("/months").into_response(),
    })
}

pub async fn month_home(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>) -> Page {
    let loaded = match st.load(&user.0, &id).await {
        Ok(l) => l,
        Err(AppError::NotFound) => return Ok(Redirect::to("/months").into_response()),
        Err(e) => return Err(e),
    };
    st.store.set_last_month(&user.0.id, Some(&id)).await?;
    Ok(match loaded.month.default_paycheck(st.today_for(&user.0)) {
        Some(p) => Redirect::to(&format!("/months/{id}/paychecks/{p}")).into_response(),
        None => Redirect::to(&format!("/months/{id}/income")).into_response(),
    })
}

// ----------------------------------------------------------------------
// Month pages
// ----------------------------------------------------------------------

pub async fn paycheck_page(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path((id, pid)): Path<(Id, Id)>) -> Page {
    month_page(&st, &user.0, &id, View::Paycheck { month: id.clone(), paycheck: pid }, "paycheck", &t("nav.paychecks")).await
}

pub async fn paycheck_content(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path((id, pid)): Path<(Id, Id)>) -> Sse {
    content_sse(&st, &user.0, &headers, &View::Paycheck { month: id, paycheck: pid }).await
}

pub async fn overview_page(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>) -> Page {
    month_page(&st, &user.0, &id, View::Overview { month: id.clone() }, "overview", &t("nav.overview")).await
}

pub async fn overview_content(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>) -> Sse {
    content_sse(&st, &user.0, &headers, &View::Overview { month: id }).await
}

#[derive(Deserialize)]
pub struct WelcomeQuery {
    #[serde(default)]
    welcome: Option<String>,
}

pub async fn income_page(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>, Query(q): Query<WelcomeQuery>) -> Page {
    let welcome = q.welcome.is_some();
    month_page(&st, &user.0, &id, View::Income { month: id.clone(), welcome }, "income", &t("nav.income")).await
}

pub async fn income_content(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Query(q): Query<WelcomeQuery>) -> Sse {
    content_sse(&st, &user.0, &headers, &View::Income { month: id, welcome: q.welcome.is_some() }).await
}

pub async fn transactions_page(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>) -> Page {
    month_page(&st, &user.0, &id, View::Transactions { month: id.clone() }, "transactions", &t("nav.transactions")).await
}

pub async fn transactions_content(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>) -> Sse {
    content_sse(&st, &user.0, &headers, &View::Transactions { month: id }).await
}

pub async fn reports_page(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>, Query(q): Query<ReportQuery>) -> Page {
    month_page(&st, &user.0, &id, View::Reports { month: id.clone(), q }, "reports", &t("nav.reports")).await
}

pub async fn reports_content(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Query(q): Query<ReportQuery>) -> Sse {
    content_sse(&st, &user.0, &headers, &View::Reports { month: id, q }).await
}

#[derive(Deserialize)]
pub struct MonthsQuery {
    #[serde(default)]
    archived: Option<String>,
}

pub async fn months_page(Extension(user): Extension<AuthUser>, Query(q): Query<MonthsQuery>) -> Markup {
    let view = View::Months { archived: q.archived.is_some() };
    layout(Some(&user.0), &t("months.title"), None, Some(&view), None)
}

pub async fn months_content(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Query(q): Query<MonthsQuery>) -> Sse {
    content_sse(&st, &user.0, &headers, &View::Months { archived: q.archived.is_some() }).await
}

pub async fn settings_page(Extension(user): Extension<AuthUser>) -> Markup {
    layout(Some(&user.0), &t("settings.title"), None, Some(&View::Settings), None)
}

pub async fn settings_content(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap) -> Sse {
    content_sse(&st, &user.0, &headers, &View::Settings).await
}

pub async fn export_csv(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>) -> Page {
    let m = st.load(&user.0, &id).await?.month;
    Ok(export::csv_response(&m))
}

pub async fn export_snapshot(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>) -> Page {
    let snap = st.snapshot(&user.0, &id).await?;
    let name = export::filename(&snap.month, "json");
    let mut r = Json(snap).into_response();
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        r.headers_mut().insert(CONTENT_DISPOSITION, v);
    }
    Ok(r)
}

// ----------------------------------------------------------------------
// Shared month widgets
// ----------------------------------------------------------------------

/// Month-level zero status, lock and variance controls (spec §2.1, §2.9, §5).
fn month_status(c: &Ctx, m: &Month, archived: bool, view: &View) -> Markup {
    let diff = m.zero_difference();
    let lock_url = format!("/ui/months/{}/lock", m.id);
    html! {
        section class="month-status" aria-label=(t("status.label")) {
            @if archived {
                p class="notice warn" { (icon("alert")) " " (t("status.archived")) }
            }
            @if m.is_locked() && !m.reassigning {
                p class="pill locked" { (icon("lock")) " " (t("status.locked")) }
            }
            div id="zero-status" class=(if m.paychecks.is_empty() { "zero todo" } else if diff.is_zero() && m.has_variance() { "zero caution" } else if diff.is_zero() { "zero ok" } else { "zero todo" }) aria-live="polite" {
                @if m.paychecks.is_empty() {
                    span { (t("status.no_income")) }
                } @else if diff.is_zero() && m.has_variance() {
                    span { (t("status.zero_with_variance")) }
                } @else if diff.is_zero() {
                    span { (icon("check")) " " (t("status.zero")) }
                } @else if diff.is_positive() {
                    span { (tf("status.left_to_assign", &[("amount", &c.money(diff))])) }
                } @else {
                    span { (tf("status.over_assigned", &[("amount", &c.money(diff.abs()))])) }
                }
                @if !m.is_locked() && !archived && !m.paychecks.is_empty() {
                    form class="inline" data-on:submit__prevent=(post_form(&lock_url)) {
                        (view_input(view))
                        button type="submit" class=(if diff.is_zero() { "btn primary" } else { "btn" }) aria-describedby="zero-status" { (icon("lock")) " " (t("status.lock")) }
                    }
                }
            }
            (overspent_banner(c, m, view))
            @if (m.has_variance() || m.reassigning) && !archived {
                (variance_panel(c, m, view))
            }
        }
    }
}

/// Month-level banner listing every overspent line.
fn overspent_banner(c: &Ctx, m: &Month, view: &View) -> Markup {
    let overs = overspent_lines(m);
    html! {
        @if !overs.is_empty() {
            div class="notice over" id="overspent-banner" role="status" {
                (icon("alert")) " "
                strong { (tf("over.banner", &[("n", &overs.len().to_string())])) } " "
                @for (i, (name, amt)) in overs.iter().enumerate() {
                    @if i > 0 { ", " }
                    (tf("over.item", &[("name", name), ("amount", &c.money(*amt))]))
                }
                @if !matches!(view, View::Overview { .. }) {
                    " " a href=(format!("/months/{}/overview", m.id)) { (t("over.review")) }
                }
            }
        }
    }
}

/// Lines whose Spent exceeds Planned, worst first.
fn overspent_lines(m: &Month) -> Vec<(String, Cents)> {
    let mut v: Vec<(String, Cents)> = m
        .expense_lines
        .iter()
        .filter_map(|l| {
            let over = m.line_spent(&l.id) - m.line_planned(&l.id);
            over.is_positive().then(|| (l.name.clone(), over))
        })
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    v
}

fn over_badge(c: &Ctx, l: &LineView) -> Markup {
    let over = l.spent - l.planned;
    html! {
        @if over.is_positive() {
            span class="over-badge" { (icon("alert")) " " (tf("over.line", &[("amount", &c.money(over))])) }
        }
    }
}

/// One panel for the whole variance flow (spec §2.9): what differs, the
/// steps to bring the month back to zero, and where to look.
fn variance_panel(c: &Ctx, m: &Month, view: &View) -> Markup {
    let report = variance_suggestions(m);
    let pending: Vec<&Paycheck> = m.paychecks_by_date().into_iter().filter(|p| p.has_variance()).collect();
    let step = |done: bool, active: bool, body: Markup| {
        html! {
            li class={ "step" @if done { " done" } @if active { " active" } } {
                span class="step-mark" aria-hidden="true" { @if done { (icon("check")) } }
                div class="step-body" { (body) }
            }
        }
    };
    html! {
        section class="variance card" id="variance-panel" aria-labelledby="variance-h" {
            h2 id="variance-h" class="h3" { (icon("alert")) " " (t("variance.title")) }
            @if m.reassigning {
                p class="notice info" { (t("status.reassigning")) }
            }
            @if !pending.is_empty() {
                p { (tf("variance.net", &[("amount", &c.money(report.net_variance))])) }
            }
            ul class="variance-list" {
                @for p in &pending {
                    li {
                        span { (tf("variance.paycheck", &[
                            ("date", &short_date(p.date)),
                            ("planned", &c.money(p.planned_amount)),
                            ("actual", &c.money(p.actual_amount.unwrap_or_default())),
                        ])) }
                        @if m.allocations_editable() {
                            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{}/apply-actual", p.id))) {
                                (view_input(view))
                                button type="submit" class="btn small primary" { (t("variance.apply")) }
                            }
                        }
                    }
                }
            }
            @if m.is_locked() {
                ol class="steps" aria-label=(t("variance.steps")) {
                    (step(m.reassigning, !m.reassigning, html! {
                        @if m.reassigning { (t("variance.step_open_done")) } @else {
                            form data-on:submit__prevent=(post_form(&format!("/ui/months/{}/reassign/begin", m.id))) {
                                (view_input(view))
                                button type="submit" class="btn primary" { (t("variance.begin")) }
                            }
                            span class="muted small" { (t("variance.step_open_hint")) }
                        }
                    }))
                    (step(m.reassigning && pending.is_empty(), m.reassigning && !pending.is_empty(), html! { (t("variance.step_apply")) }))
                    (step(m.reassigning && pending.is_empty() && m.is_zero(), m.reassigning && pending.is_empty() && !m.is_zero(), html! { (t("variance.step_balance")) }))
                    (step(false, m.reassigning && pending.is_empty() && m.is_zero(), html! {
                        @if m.reassigning {
                            form data-on:submit__prevent=(post_form(&format!("/ui/months/{}/reassign/finish", m.id))) {
                                (view_input(view))
                                button type="submit" class=(if pending.is_empty() && m.is_zero() { "btn primary" } else { "btn" }) { (icon("lock")) " " (t("status.finish_reassign")) }
                            }
                        } @else { (t("variance.step_relock")) }
                    }))
                }
            }
            @if !report.suggestions.is_empty() {
                p class="muted" { (t("variance.suggestions")) }
                ul class="suggestions" {
                    @for s in report.suggestions.iter().take(3) {
                        li {
                            strong { (s.line_name) } " — "
                            (match s.kind {
                                SuggestionKind::Overspent => tf("variance.overspent", &[("amount", &c.money(s.amount))]),
                                SuggestionKind::UnfundedTarget => tf("variance.unfunded", &[("amount", &c.money(s.amount))]),
                                SuggestionKind::Unspent => tf("variance.unspent", &[("amount", &c.money(s.amount))]),
                            })
                        }
                    }
                }
            }
        }
    }
}

fn category_details(c: &Ctx, cat: &CategoryView, summary_right: Markup, body: Markup) -> Markup {
    html! {
        details class="category" data-category=(cat.name) data-cat-id=(cat.id) open[c.is_open(&cat.name)] {
            summary {
                span class="cat-name" { (cat.name)
                    @if cat.lines.iter().any(|l| l.spent > l.planned) {
                        span class="cat-over" title=(t("over.category")) { (icon("alert")) span class="visually-hidden" { (t("over.category")) } }
                    }
                }
                (summary_right)
            }
            (body)
        }
    }
}

fn triad(c: &Ctx, planned: Cents, spent: Cents, remaining: Cents) -> Markup {
    html! {
        span class="num" data-col="planned" data-label=(t("col.planned")) { (c.money(planned)) }
        span class="num" data-col="spent" data-label=(t("col.spent")) { (c.money(spent)) }
        span class=(if remaining.is_negative() { "num neg" } else { "num" }) data-col="remaining" data-label=(t("col.remaining")) { (c.money(remaining)) }
    }
}

// ----------------------------------------------------------------------
// Paycheck view (primary, spec §14.5)
// ----------------------------------------------------------------------

fn paycheck_strip(c: &Ctx, m: &Month, active: &Id) -> Markup {
    html! {
        nav class="paycheck-strip" aria-label=(t("paycheck.all")) {
            @for p in m.paychecks_by_date() {
                @let v = m.paycheck_view(p);
                a href=(format!("/months/{}/paychecks/{}", m.id, p.id))
                    class={ "chip" @if &p.id == active { " active" } @if v.status == PaycheckStatus::Skipped { " skipped" } @else if v.fully_allocated { " full" } }
                    aria-current=[(&p.id == active).then_some("page")] {
                    span class="chip-date" { (short_date(p.date)) }
                    span class="chip-name" { (v.income_line_name) }
                    span class="chip-amt" { (c.money(v.planned_amount)) }
                    span class="chip-state" {
                        @if v.status == PaycheckStatus::Skipped { (t("paycheck.skipped")) }
                        @else if v.fully_allocated { (icon("check")) " " (t("paycheck.fully_assigned")) }
                        @else { (tf("paycheck.left", &[("amount", &c.money(v.unallocated))])) }
                    }
                }
            }
            a class="chip add" href=(format!("/months/{}/income", m.id)) { "+ " (t("paycheck.add")) }
        }
    }
}

pub fn render_paycheck(c: &Ctx, m: &Month, archived: bool, pid: &Id) -> Markup {
    let Some(p) = m.paycheck(pid) else { return html! {} };
    let v = m.paycheck_view(p);
    let view = View::Paycheck { month: m.id.clone(), paycheck: pid.clone() };
    let editable = m.allocations_editable() && !archived && p.status != PaycheckStatus::Skipped;
    let structure = !m.is_locked() && !archived;
    let all_cats = m.paycheck_category_views(pid);
    let targets: Vec<&ExpenseLine> =
        m.expense_lines.iter().filter(|l| m.line_unfunded_target(&l.id).is_positive()).collect();
    let fund_url = format!("/ui/months/{}/fund", m.id);
    html! {
        h1 class="visually-hidden" { (tf("paycheck.heading", &[("date", &short_date(p.date)), ("name", &v.income_line_name)])) }
        (paycheck_strip(c, m, pid))
        (month_status(c, m, archived, &view))

        section class="hero" aria-labelledby="sts-label" {
            p id="sts-label" class="hero-label" { (t("paycheck.safe_to_spend")) }
            p id="safe-to-spend" class=(if v.safe_to_spend.is_negative() { "hero-amount neg" } else { "hero-amount" }) aria-live="polite" {
                (c.money(v.safe_to_spend))
            }
            p class="hero-sub" { (tf("paycheck.hero_sub", &[("date", &short_date(p.date)), ("name", &v.income_line_name)])) }
            @if v.safe_to_spend.is_negative() {
                p class="hero-warn" role="status" { (icon("alert")) " " (tf("over.sts", &[("amount", &c.money(v.safe_to_spend.abs()))])) }
            }
            dl class="stats" {
                div { dt { (t("paycheck.planned")) } dd data-stat="planned" { (c.money(v.planned_amount)) } }
                div { dt { (t("paycheck.assigned")) } dd data-stat="assigned" { (c.money(v.allocated)) } }
                div { dt { (t("paycheck.unassigned")) } dd data-stat="unassigned" { (c.money(v.unallocated)) } }
                div { dt { (t("paycheck.tagged")) } dd data-stat="tagged" { (c.money(v.tagged_expense)) } }
                div { dt { (t("paycheck.budget_left")) } dd data-stat="budget-left" { (c.money(v.budget_left)) } }
                div { dt { (t("paycheck.rolling")) } dd data-stat="rolling" { (c.money(m.rolling_available(c.today))) } }
            }
        }

        @if v.status == PaycheckStatus::Skipped {
            p class="notice warn" { (t("paycheck.skipped_notice")) }
        } @else if v.unallocated.is_positive() {
            p class="notice todo" id="unassigned-nudge" role="status" { (tf("paycheck.nudge", &[("amount", &c.money(v.unallocated))])) }
        } @else {
            p class="notice ok" id="unassigned-nudge" {
                (icon("check")) " " (t("paycheck.all_assigned"))
                @if editable { " " span class="muted small" { (t("paycheck.all_assigned_hint")) } }
            }
        }

        @if editable && !targets.is_empty() && v.unallocated.is_positive() {
            section class="targets" aria-labelledby="targets-h" {
                h2 id="targets-h" class="h3" { (t("targets.title")) }
                ul {
                    @for l in &targets {
                        @let unfunded = m.line_unfunded_target(&l.id);
                        @let amount = unfunded.min(v.unallocated);
                        li {
                            span { strong { (l.name) } " " (tf("targets.line", &[("target", &c.money(l.target_amount.unwrap_or_default())), ("unfunded", &c.money(unfunded))])) }
                            form class="inline" data-on:submit__prevent=(post_form(&fund_url)) {
                                (view_input(&view))
                                input type="hidden" name="paycheck_id" value=(pid);
                                input type="hidden" name="line_id" value=(l.id);
                                input type="hidden" name="amount" value=(crate::money::plain(amount));
                                button type="submit" class="btn small" { (tf("targets.fund", &[("amount", &c.money(amount))])) }
                            }
                        }
                    }
                }
            }
        }

        @if editable && v.planned_amount.is_positive() {
            @let tithe = m.tithe_amount(pid, 10);
            @let current = m.categories.iter().find(|c| c.name.eq_ignore_ascii_case("Giving"))
                .and_then(|c| m.lines_of(&c.id).into_iter().find(|l| l.name.eq_ignore_ascii_case("Tithe")))
                .and_then(|l| m.allocation_for(pid, &l.id)).map_or(Cents::ZERO, |a| a.amount);
            div class="quick-actions" {
                @if current == tithe {
                    span class="pill ok" { (icon("check")) " " (tf("give.done", &[("amount", &c.money(tithe))])) }
                } @else {
                    form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/give"))) {
                        (view_input(&view))
                        button type="submit" class="btn" { (tf("give.button", &[("amount", &c.money(tithe))])) }
                    }
                }
            }
        }

        section class=(if c.only_funded { "funding five only-funded" } else { "funding five" }) id="funding" aria-labelledby="funding-h" {
            div class="section-head" {
                h2 id="funding-h" { (t("paycheck.funds")) }
                button type="button" class="chip-toggle" data-only-funded aria-pressed=(if c.only_funded { "true" } else { "false" }) {
                    (t("paycheck.only_funded"))
                }
            }
            @if all_cats.iter().all(|c| c.lines.is_empty()) && !structure {
                (empty_state(&t("paycheck.empty_title"), &t("paycheck.empty_body"), None))
            }
            div class="grid-head" aria-hidden="true" {
                span { (t("col.name")) } span { (t("col.this_paycheck")) } span { (t("col.planned")) } span { (t("col.spent")) } span { (t("col.remaining")) }
            }
            @for cat in &all_cats {
                @let funded = cat.lines.iter().any(|l| l.this_paycheck.is_positive());
                div class=(if funded { "cat-wrap" } else { "cat-wrap unfunded" }) {
                (category_details(c, cat, html! {
                    span class="num" data-col="this" data-label=(t("col.this_paycheck")) { (c.money(cat.this_paycheck)) }
                    (triad(c, cat.planned, cat.spent, cat.remaining))
                }, html! {
                    ul class="lines" {
                        @for l in &cat.lines {
                            li class={ "line" @if l.spent > l.planned { " over" } @if !l.this_paycheck.is_positive() { " unfunded" } } id=(format!("line-{}", l.id)) data-line=(l.name) {
                                div class="cell name" {
                                    @if structure {
                                        form data-on:submit__prevent=(post_form(&format!("/ui/lines/{}/rename", l.id))) {
                                            (view_input(&view))
                                            input type="text" name="name" value=(l.name) aria-label=(tf("line.name_label", &[("name", &l.name)])) data-on:change="el.form.requestSubmit()" required maxlength="100";
                                        }
                                    } @else {
                                        span class="name-text" { (l.name) }
                                    }
                                    @if l.funders.len() > 1 {
                                        span class="split-note" { (tf("line.split_note", &[("count", &l.funders.len().to_string())])) }
                                    }
                                    (over_badge(c, l))
                                }
                                div class="cell num" data-col="this" data-label=(t("col.this_paycheck")) {
                                    @if editable {
                                        form data-on:submit__prevent=(post_form_guarded(&format!("/ui/paychecks/{}/lines/{}", pid, l.id))) {
                                            (view_input(&view))
                                            (money_input("amount", Some(l.this_paycheck), &tf("line.planned_label", &[("name", &l.name)]), Some((l.this_paycheck + v.unallocated).get())))
                                            span class="field-error" aria-live="polite" {}
                                        }
                                    } @else {
                                        span { (c.money(l.this_paycheck)) }
                                    }
                                }
                                span class="cell num" data-col="planned" data-label=(t("col.planned")) { (c.money(l.planned)) }
                                span class="cell num" data-col="spent" data-label=(t("col.spent")) { (c.money(l.spent)) }
                                span class=(if l.remaining.is_negative() { "cell num neg" } else { "cell num" }) data-col="remaining" data-label=(t("col.remaining")) { (c.money(l.remaining)) }
                            }
                        }
                    }
                    @if structure {
                        form class="add-line" id=(format!("pc-add-line-{}", cat.id)) data-clear data-on:submit__prevent=(post_form_guarded(&format!("/ui/months/{}/lines", m.id))) {
                            (view_input(&view))
                            input type="hidden" name="category_id" value=(cat.id);
                            input type="hidden" name="paycheck_id" value=(pid);
                            span class="add-icon" aria-hidden="true" { (icon("plus")) }
                            input type="text" name="name" required maxlength="100" placeholder=(t("line.add_placeholder")) aria-label=(tf("line.add_label", &[("category", &cat.name)]));
                            @if editable {
                                span class="add-extra" {
                                    input type="text" inputmode="decimal" class="money" name="amount" placeholder="0.00" autocomplete="off"
                                        data-max-cents=(v.unallocated.get()) aria-label=(tf("line.add_from_this_label", &[("category", &cat.name)]));
                                }
                            }
                            button type="submit" class="btn small" { (t("line.add")) }
                        }
                    }
                }))
                }
            }
            @if structure {
                form class="add-category" id="pc-add-category" data-clear data-on:submit__prevent=(post_form(&format!("/ui/months/{}/categories", m.id))) {
                    (view_input(&view))
                    label for="pc-new-category" { (t("category.add_label")) }
                    input id="pc-new-category" type="text" name="name" required maxlength="100";
                    button type="submit" class="btn" { (t("category.add")) }
                }
            }
        }

        @if editable && v.unallocated.is_positive() {
            (fund_form(c, m, pid, &view, v.unallocated, structure))
        }

        details class="paycheck-settings" {
            summary { (t("paycheck.details")) }
            div class="settings-grid" {
                @if structure {
                    form data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/planned"))) {
                        (view_input(&view))
                        label for="pc-planned" { (t("paycheck.planned_amount")) }
                        div class="inline-field" {
                            input id="pc-planned" type="text" inputmode="decimal" class="money" name="amount" value=(crate::money::plain(p.planned_amount)) required;
                            button type="submit" class="btn" { (t("common.save")) }
                        }
                    }
                }
                @if !archived && p.status != PaycheckStatus::Skipped {
                    form data-offline="actual" data-month=(m.id) data-paycheck=(pid)
                        data-base-actual=[p.actual_amount.map(Cents::get)]
                        data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/actual"))) {
                        (view_input(&view))
                        label for="pc-actual" { (t("paycheck.actual_amount")) }
                        div class="inline-field" {
                            input id="pc-actual" type="text" inputmode="decimal" class="money" name="amount" value=[p.actual_amount.map(crate::money::plain)] placeholder=(t("paycheck.actual_placeholder"));
                            button type="submit" class="btn" { (t("paycheck.record_actual")) }
                        }
                    }
                }
                @if structure {
                    div class="danger-actions" {
                    form data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/status"))) {
                        (view_input(&view))
                        @if p.status == PaycheckStatus::Skipped {
                            input type="hidden" name="status" value="planned";
                            button type="submit" class="btn small" { (t("paycheck.unskip")) }
                        } @else {
                            input type="hidden" name="status" value="skipped";
                            button type="submit" class="btn small" data-confirm=(t("paycheck.skip_confirm")) { (t("paycheck.skip")) }
                        }
                    }
                    form data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/delete"))) {
                        (view_input(&view))
                        button type="submit" class="btn small danger" data-confirm=(t("paycheck.delete_confirm")) { (icon("trash")) " " (t("paycheck.delete")) }
                    }
                    }
                }
            }
        }
    }
}

/// "Assign money from this paycheck": pick an existing line or create one.
fn fund_form(c: &Ctx, m: &Month, pid: &Id, view: &View, unallocated: Cents, structure: bool) -> Markup {
    let url = format!("/ui/months/{}/fund", m.id);
    let cats = m.categories_sorted();
    let default_cat = cats.first().map(|c| c.id.clone());
    let fill_all = format!("document.getElementById('fund-amount').value = '{}'", crate::money::plain(unallocated));
    html! {
        section class="fund" aria-labelledby="fund-h" data-signals="{_fundline: ''}" {
            h2 id="fund-h" { (t("fund.title")) }
            form id="fund-form" data-clear data-on:submit__prevent=(post_form_guarded(&url)) {
                (view_input(view))
                input type="hidden" name="paycheck_id" value=(pid);
                div class="fund-grid" {
                    div class="field f-line" {
                        label for="fund-line" { (t("fund.line")) }
                        select id="fund-line" name="line_id" data-bind:_fundline required {
                            option value="" { (t("fund.choose")) }
                            @for cat in &cats {
                                @let lines = m.lines_of(&cat.id);
                                @if !lines.is_empty() {
                                    optgroup label=(cat.name) {
                                        @for l in lines {
                                            option value=(l.id) { (l.name) }
                                        }
                                    }
                                }
                            }
                            @if structure {
                                option value="__new" { (t("fund.new_line")) }
                            }
                        }
                    }
                    @if structure {
                        div class="field f-name" data-show="$_fundline == '__new'" {
                            label for="fund-new-name" { (t("fund.new_name")) }
                            input id="fund-new-name" name="new_name" type="text" maxlength="100";
                        }
                        div class="field f-cat" data-show="$_fundline == '__new'" {
                            label for="fund-new-cat" { (t("fund.new_category")) }
                            select id="fund-new-cat" name="new_category" {
                                @for cat in &cats {
                                    option value=(cat.id) selected[Some(&cat.id) == default_cat.as_ref()] { (cat.name) }
                                }
                            }
                        }
                    }
                    div class="field f-amount" {
                        label for="fund-amount" { (t("fund.amount")) }
                        input id="fund-amount" type="text" inputmode="decimal" class="money" name="amount" autocomplete="off" required
                            data-max-cents=(unallocated.get()) placeholder="0.00" aria-describedby="fund-hint";
                    }
                    button type="submit" class="btn primary f-btn" { (t("fund.submit")) }
                }
                p id="fund-hint" class="hint" {
                    (tf("fund.available", &[("amount", &c.money(unallocated))])) " "
                    button type="button" class="link" data-on:click=(fill_all) { (tf("fund.use_all", &[("amount", &c.money(unallocated))])) }
                }
                span class="field-error" aria-live="polite" {}
            }
        }
    }
}

// ----------------------------------------------------------------------
// Monthly overview (secondary, spec §14.6)
// ----------------------------------------------------------------------

fn signed(c: &Ctx, v: Cents) -> String {
    if v.is_positive() {
        format!("+{}", c.money(v))
    } else {
        c.money(v)
    }
}

fn summary_cards(c: &Ctx, m: &Month) -> Markup {
    let s = report::summary_cards(m);
    let pct = if s.planned_expense.is_positive() {
        (s.actual_expense.get().saturating_mul(100) / s.planned_expense.get()).clamp(0, 999)
    } else {
        0
    };
    html! {
        section class="cards" aria-label=(t("cards.label")) {
            div class="card" data-card="income" {
                p class="card-label" { (t("cards.income")) }
                p class="card-value" { (c.money(s.planned_income)) }
                p class="card-sub" { (tf("cards.received", &[
                    ("amount", &c.money(s.actual_income)),
                    ("n", &s.paychecks_received.to_string()),
                    ("total", &s.paychecks_total.to_string()),
                ])) }
            }
            div class="card" data-card="expenses" {
                p class="card-label" { (t("cards.expenses")) }
                p class="card-value" { (c.money(s.planned_expense)) }
                div class=(if pct > 100 { "meter over" } else { "meter" }) role="meter" aria-valuemin="0" aria-valuemax="100"
                    aria-valuenow=(pct.min(100)) aria-label=(t("cards.spent_meter")) {
                    span style=(format!("width: {}%", pct.min(100))) {}
                }
                p class="card-sub" { (tf("cards.spent", &[("amount", &c.money(s.actual_expense)), ("pct", &pct.to_string())])) }
            }
            div class="card" data-card="remaining" {
                p class="card-label" { (t("cards.remaining")) }
                p class="card-value" { (c.money(s.remaining_to_zero)) }
                p class="card-sub" { @if s.remaining_to_zero.is_zero() { (t("cards.at_zero")) } @else { (t("cards.not_zero")) } }
            }
            div class="card" data-card="variance" {
                p class="card-label" { (t("cards.variance")) }
                @if s.paychecks_received == 0 {
                    p class="card-value muted" { "—" }
                    p class="card-sub" { (t("cards.none_received")) }
                } @else {
                    p class=(if s.income_variance.is_negative() { "card-value neg" } else { "card-value" }) { (signed(c, s.income_variance)) }
                    p class="card-sub" { (tf("cards.variance_sub", &[("n", &s.paychecks_received.to_string())])) }
                }
            }
            div class=(match s.spending { SpendingStatus::Over => "card over", _ => "card" }) data-card="trend" {
                p class="card-label" { (t("cards.trend")) }
                p class="card-value" {
                    (match s.spending {
                        SpendingStatus::Over => t("cards.over"),
                        SpendingStatus::OnPlan => t("cards.on_plan"),
                        SpendingStatus::Under => t("cards.under"),
                    })
                }
                p class="card-sub" {
                    @if s.expense_variance.is_positive() { (tf("cards.over_by", &[("amount", &c.money(s.expense_variance))])) }
                    @else { (tf("cards.left_to_spend", &[("amount", &c.money(s.expense_variance.abs()))])) }
                }
            }
        }
    }
}

pub fn render_overview(c: &Ctx, m: &Month, archived: bool) -> Markup {
    let view = View::Overview { month: m.id.clone() };
    let structure = !m.is_locked() && !archived;
    let alloc = m.allocations_editable() && !archived;
    let cats = m.category_views();
    let free: Cents = m.paychecks.iter().map(|p| m.paycheck_unallocated(&p.id)).sum();
    let default_fund_pc = m
        .paychecks_by_date()
        .into_iter()
        .find(|p| m.paycheck_unallocated(&p.id).is_positive())
        .or_else(|| m.paychecks_by_date().into_iter().find(|p| p.status != PaycheckStatus::Skipped))
        .map(|p| p.id.clone());
    let move_btn = |url: String, dir: &str, label: String| {
        html! {
            form class="inline" data-on:submit__prevent=(post_form(&url)) {
                (view_input(&view))
                input type="hidden" name="direction" value=(dir);
                button type="submit" class="icon-btn" aria-label=(label) { (icon(dir)) }
            }
        }
    };
    html! {
        h1 { (tf("overview.title", &[("month", &month_label(m.year_month))])) }
        (month_status(c, m, archived, &view))
        (summary_cards(c, m))
        p class="muted" { (t("overview.explain")) }
        @if m.paychecks.is_empty() {
            (empty_state(&t("overview.no_income_title"), &t("overview.no_income_body"),
                Some(html! { a class="btn primary" href=(format!("/months/{}/income", m.id)) { (t("income.add")) } })))
        }
        @if structure {
            form id="dnd-form" hidden data-on:submit__prevent=(post_form(&format!("/ui/months/{}/place", m.id))) {
                (view_input(&view))
                input type="hidden" name="kind";
                input type="hidden" name="id";
                input type="hidden" name="category_id";
                input type="hidden" name="index";
            }
        }
        section class=(if structure { "overview five dnd" } else { "overview five" }) aria-label=(t("overview.categories")) {
            div class="grid-head" aria-hidden="true" {
                span { (t("col.name")) } span { (t("col.planned")) } span { (t("col.spent")) } span { (t("col.remaining")) } span {}
            }
            @for cat in &cats {
                (category_details(c, cat, html! {
                    (triad(c, cat.planned, cat.spent, cat.remaining))
                    span {}
                }, html! {
                    @if structure {
                        details class="cat-tools" {
                            summary { (tf("category.edit", &[("name", &cat.name)])) }
                            div class="cat-tools-row" {
                            form class="inline grow" data-on:submit__prevent=(post_form(&format!("/ui/categories/{}/rename", cat.id))) {
                                (view_input(&view))
                                input type="text" name="name" value=(cat.name) required maxlength="100" aria-label=(tf("category.name_label", &[("name", &cat.name)])) data-on:change="el.form.requestSubmit()";
                            }
                            (move_btn(format!("/ui/categories/{}/move", cat.id), "up", tf("category.move_up", &[("name", &cat.name)])))
                            (move_btn(format!("/ui/categories/{}/move", cat.id), "down", tf("category.move_down", &[("name", &cat.name)])))
                            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/categories/{}/delete", cat.id))) {
                                (view_input(&view))
                                button type="submit" class="icon-btn danger" aria-label=(tf("category.delete", &[("name", &cat.name)])) data-confirm=(tf("category.delete_confirm", &[("name", &cat.name)])) { (icon("trash")) }
                            }
                            }
                        }
                    }
                    @if cat.lines.is_empty() && !structure {
                        p class="muted small" { (t("category.empty")) }
                    }
                    ul class="lines" {
                        @for l in &cat.lines {
                            li class={ "line" @if l.spent > l.planned { " over" } } id=(format!("line-{}", l.id)) data-line=(l.name) data-line-id=[structure.then_some(&l.id)] data-cat-id=(cat.id) {
                                div class="cell name" {
                                    @if structure {
                                        span class="grip" title=(t("line.drag")) aria-hidden="true" { (icon("grip")) }
                                        form data-on:submit__prevent=(post_form(&format!("/ui/lines/{}/rename", l.id))) {
                                            (view_input(&view))
                                            input type="text" name="name" value=(l.name) required maxlength="100" aria-label=(tf("line.name_label", &[("name", &l.name)])) data-on:change="el.form.requestSubmit()";
                                        }
                                    } @else {
                                        span class="name-text" { (l.name) }
                                    }
                                    (over_badge(c, l))
                                    @if !l.funders.is_empty() {
                                        span class="split-note" {
                                            @for (i, f) in l.funders.iter().enumerate() {
                                                @if i > 0 { " · " }
                                                (tf("line.funder", &[("date", &short_date(f.date)), ("amount", &c.money(f.amount))]))
                                            }
                                        }
                                    }
                                    @if let Some(tgt) = l.target {
                                        @if l.unfunded_target.is_positive() {
                                            span class="split-note" { (tf("line.target_note", &[("target", &c.money(tgt)), ("unfunded", &c.money(l.unfunded_target))])) }
                                        }
                                    }
                                }
                                div class="cell num" data-col="planned" data-label=(t("col.planned")) {
                                    @if alloc {
                                        form data-on:submit__prevent=(post_form_guarded(&format!("/ui/lines/{}/planned", l.id))) {
                                            (view_input(&view))
                                            (money_input("amount", Some(l.planned), &tf("line.total_planned_label", &[("name", &l.name)]), Some((l.planned + free).get())))
                                            span class="field-error" aria-live="polite" {}
                                        }
                                    } @else {
                                        span { (c.money(l.planned)) }
                                    }
                                }
                                span class="cell num" data-col="spent" data-label=(t("col.spent")) { (c.money(l.spent)) }
                                span class=(if l.remaining.is_negative() { "cell num neg" } else { "cell num" }) data-col="remaining" data-label=(t("col.remaining")) { (c.money(l.remaining)) }
                                @if structure {
                                    div class="line-tools" {
                                        (move_btn(format!("/ui/lines/{}/move", l.id), "up", tf("line.move_up", &[("name", &l.name)])))
                                        (move_btn(format!("/ui/lines/{}/move", l.id), "down", tf("line.move_down", &[("name", &l.name)])))
                                        form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/lines/{}/delete", l.id))) {
                                            (view_input(&view))
                                            button type="submit" class="icon-btn danger" aria-label=(tf("line.delete", &[("name", &l.name)])) data-confirm=(tf("line.delete_confirm", &[("name", &l.name)])) { (icon("trash")) }
                                        }
                                    }
                                }
                                @if !m.paychecks.is_empty() {
                                    details class="line-funding" data-line-funding=(l.id) {
                                        summary { (tf("line.by_paycheck", &[("count", &l.funders.len().to_string())])) }
                                        ul class="funding-list" {
                                            @for p in m.paychecks_by_date() {
                                                @let this = m.allocation_for(&p.id, &l.id).map_or(Cents::ZERO, |a| a.amount);
                                                @let free = m.paycheck_unallocated(&p.id);
                                                @let pname = m.income_line(&p.income_line_id).map(|x| x.name.clone()).unwrap_or_default();
                                                li {
                                                    span class="fund-pc" { strong { (short_date(p.date)) } " · " (pname)
                                                        @if p.status == PaycheckStatus::Skipped { " · " (t("paycheck.skipped")) } }
                                                    @if alloc && p.status != PaycheckStatus::Skipped {
                                                        form data-on:submit__prevent=(post_form_guarded(&format!("/ui/paychecks/{}/lines/{}", p.id, l.id))) {
                                                            (view_input(&view))
                                                            (money_input("amount", Some(this), &tf("line.from_paycheck_label", &[("name", &l.name), ("date", &short_date(p.date))]), Some((this + free).get())))
                                                            span class="field-error" aria-live="polite" {}
                                                        }
                                                        span class="muted small" { (tf("line.paycheck_left", &[("amount", &c.money(free))])) }
                                                    } @else {
                                                        span class="num" { (c.money(this)) }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                @if l.is_debt {
                                    div class="debt" {
                                        @if structure {
                                            form class="debt-form" data-on:submit__prevent=(post_form(&format!("/ui/lines/{}/debt", l.id))) {
                                                (view_input(&view))
                                                label { (t("debt.balance")) " " input type="text" inputmode="decimal" name="current_balance" value=[l.current_balance.map(crate::money::plain)] data-on:change="el.form.requestSubmit()"; }
                                                label { (t("debt.minimum")) " " input type="text" inputmode="decimal" name="minimum_payment" value=[l.minimum_payment.map(crate::money::plain)] data-on:change="el.form.requestSubmit()"; }
                                            }
                                        } @else {
                                            span { (t("debt.balance")) " " (l.current_balance.map(|b| c.money(b)).unwrap_or_else(|| "—".into())) }
                                            span { (t("debt.minimum")) " " (l.minimum_payment.map(|b| c.money(b)).unwrap_or_else(|| "—".into())) }
                                        }
                                        @if let Some(min) = l.minimum_payment {
                                            @if l.planned < min {
                                                span class="warn-text" { (tf("debt.below_minimum", &[("amount", &c.money(min - l.planned))])) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    @if structure {
                        form class="add-line" id=(format!("add-line-{}", cat.id)) data-clear data-on:submit__prevent=(post_form_guarded(&format!("/ui/months/{}/lines", m.id))) {
                            (view_input(&view))
                            input type="hidden" name="category_id" value=(cat.id);
                            span class="add-icon" aria-hidden="true" { (icon("plus")) }
                            input type="text" name="name" required maxlength="100" placeholder=(t("line.add_placeholder")) aria-label=(tf("line.add_label", &[("category", &cat.name)]));
                            @if alloc && !m.paychecks.is_empty() {
                                span class="add-extra" {
                                    input type="text" inputmode="decimal" class="money" name="amount" placeholder="0.00" autocomplete="off"
                                        aria-label=(tf("line.add_amount_label", &[("category", &cat.name)]));
                                    select name="paycheck_id" aria-label=(tf("line.add_paycheck_label", &[("category", &cat.name)])) {
                                        @for p in m.paychecks_by_date().into_iter().filter(|p| p.status != PaycheckStatus::Skipped) {
                                            @let free = m.paycheck_unallocated(&p.id);
                                            option value=(p.id) selected[Some(&p.id) == default_fund_pc.as_ref()] {
                                                (tf("line.fund_option", &[("date", &short_date(p.date)), ("amount", &c.money(free))]))
                                            }
                                        }
                                    }
                                }
                            }
                            button type="submit" class="btn small" { (t("line.add")) }
                        }
                    }
                }))
            }
        }
        @if structure {
            form class="add-category" id="add-category" data-clear data-on:submit__prevent=(post_form(&format!("/ui/months/{}/categories", m.id))) {
                (view_input(&view))
                label for="new-category" { (t("category.add_label")) }
                input id="new-category" type="text" name="name" required maxlength="100";
                button type="submit" class="btn" { (t("category.add")) }
            }
        }
        section class="exports" aria-labelledby="export-h" {
            h2 id="export-h" class="h3" { (t("export.title")) }
            a class="btn" href=(format!("/months/{}/export.csv", m.id)) download { (t("export.csv")) }
            " "
            a class="btn" href=(format!("/months/{}/snapshot.json", m.id)) download { (t("export.snapshot")) }
        }
    }
}

// ----------------------------------------------------------------------
// Income lines (spec §2.2)
// ----------------------------------------------------------------------

fn describe_rule(r: Option<&Recurrence>) -> String {
    match r {
        None => t("schedule.one_off"),
        Some(Recurrence::Weekly { anchor }) => tf("schedule.weekly_desc", &[("day", &anchor.format("%A").to_string())]),
        Some(Recurrence::Biweekly { anchor }) => tf("schedule.biweekly_desc", &[("date", &anchor.format("%b %-d, %Y").to_string())]),
        Some(Recurrence::SemiMonthly { days }) => tf("schedule.semi_desc", &[("a", &days[0].to_string()), ("b", &days[1].to_string())]),
        Some(Recurrence::Monthly { days }) => {
            tf("schedule.monthly_desc", &[("days", &days.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))])
        }
    }
}

pub fn render_income(c: &Ctx, m: &Month, archived: bool, welcome: bool) -> Markup {
    let view = View::Income { month: m.id.clone(), welcome: false };
    let structure = !m.is_locked() && !archived;
    html! {
        h1 { (tf("income.title", &[("month", &month_label(m.year_month))])) }
        @if welcome || m.income_lines.is_empty() {
            div class="notice info welcome" {
                h2 class="h3" { (t("onboarding.title")) }
                p { (t("onboarding.body")) }
            }
        }
        (month_status(c, m, archived, &view))
        @if m.income_lines.is_empty() {
            (empty_state(&t("income.empty_title"), &t("income.empty_body"), None))
        }
        @for l in &m.income_lines {
            section class="income-line card" aria-label=(l.name) {
                div class="section-head" {
                    h2 class="h3" { (l.name) }
                    span class="muted" { (c.money(l.planned_amount)) " · " (describe_rule(l.recurrence_rule.as_ref())) }
                }
                table class="table" {
                    caption class="visually-hidden" { (tf("income.paychecks_of", &[("name", &l.name)])) }
                    thead { tr { th scope="col" { (t("col.date")) } th scope="col" { (t("col.planned")) } th scope="col" { (t("col.actual")) } th scope="col" { (t("col.status")) } } }
                    tbody {
                        @for p in m.paychecks_by_date().iter().filter(|p| p.income_line_id == l.id) {
                            tr {
                                td { a href=(format!("/months/{}/paychecks/{}", m.id, p.id)) { (short_date(p.date)) } }
                                td { (c.money(p.planned_amount)) }
                                td { (p.actual_amount.map(|a| c.money(a)).unwrap_or_else(|| "—".into())) }
                                td { (t(&format!("status.{}", p.status.as_str()))) }
                            }
                        }
                    }
                }
                @if structure {
                    details class="edit-income" {
                        summary { (t("income.edit")) }
                        (income_form(&view, &format!("/ui/income/{}", l.id), "edit", Some(l), m))
                        form class="inline" id=(format!("add-date-form-{}", l.id)) data-clear data-on:submit__prevent=(post_form(&format!("/ui/income/{}/paychecks", l.id))) {
                            (view_input(&view))
                            label for=(format!("add-date-{}", l.id)) { (t("income.add_date")) }
                            input id=(format!("add-date-{}", l.id)) type="date" name="date" required
                                min=(m.year_month.to_string()) max=(recurrence::last_of_month(m.year_month).to_string());
                            button type="submit" class="btn small" { (t("income.add_paycheck")) }
                        }
                        form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/income/{}/delete", l.id))) {
                            (view_input(&view))
                            button type="submit" class="btn small danger" data-confirm=(tf("income.delete_confirm", &[("name", &l.name)])) { (t("income.delete")) }
                        }
                    }
                }
            }
        }
        @if structure {
            section class="card add-income" aria-labelledby="add-income-h" {
                h2 id="add-income-h" class="h3" { (t("income.add")) }
                (income_form(&view, &format!("/ui/months/{}/income", m.id), "new", None, m))
            }
        }
    }
}

/// Income line form. The "new" form offers smart suggestions from history.
fn income_form(view: &View, url: &str, prefix: &str, line: Option<&IncomeLine>, m: &Month) -> Markup {
    let kind = match line.and_then(|l| l.recurrence_rule.as_ref()) {
        None => "one_off",
        Some(Recurrence::Weekly { .. }) => "weekly",
        Some(Recurrence::Biweekly { .. }) => "biweekly",
        Some(Recurrence::SemiMonthly { .. }) => "semi_monthly",
        Some(Recurrence::Monthly { .. }) => "monthly",
    };
    let (anchor, days) = match line.and_then(|l| l.recurrence_rule.as_ref()) {
        Some(Recurrence::Weekly { anchor } | Recurrence::Biweekly { anchor }) => (Some(anchor.to_string()), String::new()),
        Some(Recurrence::SemiMonthly { days }) => (None, format!("{}, {}", days[0], days[1])),
        Some(Recurrence::Monthly { days }) => (None, days.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")),
        None => (None, String::new()),
    };
    let first_date = line
        .and_then(|l| m.paychecks_by_date().iter().find(|p| p.income_line_id == l.id).map(|p| p.date.to_string()))
        .unwrap_or_else(|| m.year_month.to_string());
    let is_new = line.is_none();
    let sig = format!("_{prefix}kind");
    let signals = format!("{{{sig}: '{}', _sugoff: false}}", if is_new { "biweekly" } else { kind });
    let id = |f: &str| format!("{prefix}-{f}");
    let suggest_url = format!("/months/{}/income/suggestions", m.id);
    html! {
        form class="income-form" id=(id("form")) data-clear[is_new] data-signals=(signals) data-on:submit__prevent=(post_form(url)) {
            (view_input(view))
            div class="field" {
                label for=(id("name")) { (t("income.name")) }
                input id=(id("name")) type="text" name="name" required maxlength="100" autocomplete="off"
                    value=[line.map(|l| l.name.clone())]
                    "data-on:input__debounce.250ms"=[is_new.then(|| format!("if ($_sugoff !== el.value) {{ $_sugoff = false; @get('{suggest_url}?q=' + encodeURIComponent(el.value)) }}"))]
                    data-on:focus=[is_new.then(|| format!("$_sugoff === false && @get('{suggest_url}?q=' + encodeURIComponent(el.value))"))];
            }
            @if is_new {
                div id="income-suggestions" class="suggest-box" data-show="$_sugoff === false" {}
            }
            div class="field" {
                label for=(id("amount")) { (t("income.amount")) }
                input id=(id("amount")) type="text" inputmode="decimal" class="money" name="amount" required placeholder="0.00"
                    value=[line.map(|l| crate::money::plain(l.planned_amount))];
            }
            div class="field" {
                label for=(id("kind")) { (t("income.schedule")) }
                select id=(id("kind")) name="kind" data-bind=(sig) {
                    @for (k, label) in [("one_off", "schedule.one_off"), ("weekly", "schedule.weekly"), ("biweekly", "schedule.biweekly"), ("semi_monthly", "schedule.semi_monthly"), ("monthly", "schedule.monthly")] {
                        option value=(k) selected[(!is_new && k == kind) || (is_new && k == "biweekly")] { (t(label)) }
                    }
                }
            }
            div class="field" data-show=(format!("${sig} == 'one_off'")) {
                label for=(id("date")) { (t("income.date")) }
                input id=(id("date")) type="date" name="date" value=(first_date)
                    min=(m.year_month.to_string()) max=(recurrence::last_of_month(m.year_month).to_string());
            }
            div class="field" data-show=(format!("${sig} == 'weekly' || ${sig} == 'biweekly'")) {
                label for=(id("anchor")) { (t("income.anchor")) }
                input id=(id("anchor")) type="date" name="anchor" value=(anchor.unwrap_or_else(|| m.year_month.to_string())) aria-describedby=(id("anchor-hint"));
                p id=(id("anchor-hint")) class="hint" { (t("income.anchor_hint")) }
            }
            div class="field" data-show=(format!("${sig} == 'semi_monthly' || ${sig} == 'monthly'")) {
                label for=(id("days")) { (t("income.days")) }
                input id=(id("days")) type="text" name="days" value=(days) placeholder="1, 15" aria-describedby=(id("days-hint"));
                p id=(id("days-hint")) class="hint" { (t("income.days_hint")) }
            }
            button type="submit" class="btn primary" { @if is_new { (t("income.add")) } @else { (t("common.save")) } }
        }
    }
}

// ----------------------------------------------------------------------
// Transactions (spec §2.8)
// ----------------------------------------------------------------------

fn line_options(m: &Month, selected: Option<&Id>) -> Markup {
    html! {
        option value="" { (t("tx.no_line")) }
        @for cat in m.categories_sorted() {
            @let lines = m.lines_of(&cat.id);
            @if !lines.is_empty() {
                optgroup label=(cat.name) {
                    @for l in lines {
                        option value=(l.id) selected[selected == Some(&l.id)] { (l.name) }
                    }
                }
            }
        }
    }
}

fn paycheck_options(m: &Month, selected: Option<&Id>) -> Markup {
    html! {
        option value="" { (t("tx.no_paycheck")) }
        @for p in m.paychecks_by_date() {
            @let name = m.income_line(&p.income_line_id).map(|l| l.name.clone()).unwrap_or_default();
            option value=(p.id) selected[selected == Some(&p.id)] { (short_date(p.date)) " · " (name) }
        }
    }
}

/// One transaction form for both plain and split transactions. With one
/// part it reads as a normal transaction (Amount, Expense line, Paycheck);
/// "Split into parts" turns the same fields into part rows.
fn tx_fields(c: &Ctx, m: &Month, prefix: &str, parts: &[&Transaction], default_date: NaiveDate) -> Markup {
    let id = |f: &str| format!("{prefix}-{f}");
    let first = parts.first().copied();
    let is_income = first.is_some_and(|x| x.amount.is_positive());
    let split = parts.len() > 1;
    let total: Cents = parts.iter().map(|p| p.amount.abs()).sum();
    let rows: Vec<(Option<Cents>, Option<Id>, Option<Id>)> = if parts.is_empty() {
        vec![(None, None, None)]
    } else {
        parts.iter().map(|x| (Some(x.amount.abs()), x.expense_line_id.clone(), x.paycheck_id.clone())).collect()
    };
    html! {
        fieldset class="segmented field" {
            legend { (t("tx.kind")) }
            label { input type="radio" name="direction" value="expense" checked[!is_income]; span { (t("tx.expense")) } }
            label { input type="radio" name="direction" value="income" checked[is_income]; span { (t("tx.income")) } }
        }
        div class="field" {
            label for=(id("date")) { (t("tx.date")) }
            input id=(id("date")) type="date" name="date" required value=(first.map_or(default_date, |x| x.date).to_string())
                min=(m.year_month.to_string()) max=(recurrence::last_of_month(m.year_month).to_string());
        }
        div class="field" {
            label for=(id("amount")) { (t("tx.amount")) }
            input id=(id("amount")) type="text" inputmode="decimal" class="money" name="amount" required placeholder="0.00"
                value=[first.map(|_| crate::money::plain(total))] data-split-total;
        }
        div class="field" {
            label for=(id("payee")) { (t("tx.payee")) }
            input id=(id("payee")) type="text" name="payee" maxlength="200" value=[first.and_then(|x| x.payee.clone())];
        }
        div class={ "tx-parts wide" @if split { " is-split" } } data-split-editor
            data-label-line=(t("tx.line")) data-label-paycheck=(t("tx.paycheck"))
            data-label-part-line=(t("split.part_line")) data-label-part-paycheck=(t("split.part_paycheck"))
            data-label-part-amount=(t("split.part_amount")) data-label-remove=(t("split.remove_part")) {
            div class="parts-head" aria-hidden="true" {
                span { (t("tx.line")) } span { (t("tx.paycheck")) } span { (t("split.amount_col")) } span {}
            }
            ol class="parts" data-parts {
                @for (i, (amt, line, pc)) in rows.iter().enumerate() {
                    @let n = (i + 1).to_string();
                    li class="part" data-part {
                        div class="field" {
                            label class="part-label" for=(format!("{}-line-{i}", prefix)) { (t("tx.line")) }
                            select id=(format!("{}-line-{i}", prefix)) name=(format!("part_line_{i}"))
                                aria-label=(if split { tf("split.part_line", &[("n", &n)]) } else { t("tx.line") }) {
                                (line_options(m, line.as_ref()))
                            }
                        }
                        div class="field" {
                            label class="part-label" for=(format!("{}-pc-{i}", prefix)) { (t("tx.paycheck")) }
                            select id=(format!("{}-pc-{i}", prefix)) name=(format!("part_paycheck_{i}")) aria-describedby=(id("paycheck-hint"))
                                aria-label=(if split { tf("split.part_paycheck", &[("n", &n)]) } else { t("tx.paycheck") }) {
                                (paycheck_options(m, pc.as_ref()))
                            }
                        }
                        input type="text" inputmode="decimal" class="money part-amount" name=(format!("part_amount_{i}")) placeholder="0.00"
                            value=[amt.filter(|_| split).map(crate::money::plain)] aria-label=(tf("split.part_amount", &[("n", &n)])) data-part-amount;
                        button type="button" class="icon-btn danger part-remove" data-remove-part aria-label=(tf("split.remove_part", &[("n", &n)])) { (icon("trash")) }
                    }
                }
            }
            div class="split-foot" {
                button type="button" class="link" data-add-part {
                    span class="when-single" { (t("split.start")) }
                    span class="when-split" { (t("split.add_part")) }
                }
                span class="split-left when-split" data-split-left aria-live="polite" {
                    (t("split.remaining")) ": " span data-split-left-amt { (c.money(Cents::ZERO)) }
                }
            }
            span class="field-error" aria-live="polite" {}
        }
        div class="field wide" {
            label for=(id("notes")) { (t("tx.notes")) }
            input id=(id("notes")) type="text" name="notes" maxlength="2000" value=[first.and_then(|x| x.notes.clone())];
        }
        p id=(id("paycheck-hint")) class="hint wide" { (t("tx.paycheck_hint")) }
    }
}

enum TxItem<'a> {
    Single(&'a Transaction),
    Split(Id),
}

pub fn render_transactions(c: &Ctx, m: &Month, archived: bool) -> Markup {
    let view = View::Transactions { month: m.id.clone() };
    let default_date = if recurrence::in_month(c.today, m.year_month) { c.today } else { m.year_month };
    let mut txs: Vec<&Transaction> = m.transactions.iter().collect();
    txs.sort_by_key(|x| std::cmp::Reverse(x.date));
    // One row per payment: a split shows once, where its first part sorts.
    let mut items: Vec<TxItem> = Vec::new();
    for x in &txs {
        match &x.split_group {
            Some(g) => {
                if !items.iter().any(|i| matches!(i, TxItem::Split(h) if h == g)) {
                    items.push(TxItem::Split(g.clone()));
                }
            }
            None => items.push(TxItem::Single(x)),
        }
    }
    let line_name = |id: &Option<Id>| id.as_ref().and_then(|l| m.expense_line(l)).map(|l| l.name.clone());
    html! {
        h1 { (tf("tx.title", &[("month", &month_label(m.year_month))])) }
        (overspent_banner(c, m, &view))
        @if m.is_locked() {
            p class="notice info" { (t("tx.locked_ok")) }
        }
        @if !archived {
            section class="card" aria-labelledby="add-tx-h" {
                h2 id="add-tx-h" class="h3" { (t("tx.add")) }
                form id="add-tx" class="tx-form" data-clear data-offline="create_transaction" data-month=(m.id)
                    data-on:submit__prevent=(format!("pz.checkSplit(el) && @post('/ui/months/{}/transactions', {{contentType: 'form'}})", m.id)) {
                    (view_input(&view))
                    (tx_fields(c, m, "new-tx", &[], default_date))
                    div class="form-actions wide" {
                        button type="submit" class="btn primary" { (t("tx.save")) }
                    }
                }
            }
        }
        section aria-labelledby="tx-list-h" {
            h2 id="tx-list-h" class="h3" { (t("tx.list")) }
            @if txs.is_empty() {
                (empty_state(&t("tx.empty_title"), &t("tx.empty_body"), None))
            } @else {
                ul class="tx-list card" id="tx-list" {
                    @for it in &items {
 @match it {
 TxItem::Split(g) => {
                        @let parts = m.split_parts(g);
                        @let x = parts[0];
                        @let total: Cents = parts.iter().map(|p| p.amount).sum();
                        @let payee = x.payee.clone().unwrap_or_else(|| t("tx.no_payee"));
                        @let dlg = format!("edit-split-{g}");
                        li class="tx split" data-tx=(x.id) data-split=(g) {
                            span class="tx-date" { (short_date(x.date)) }
                            div class="tx-main" {
                                span class="tx-payee" { (payee) }
                                span class="tx-meta" {
                                    strong { (t("split.meta")) } " · "
                                    @for (i, p) in parts.iter().enumerate() {
                                        @if i > 0 { " · " }
                                        (line_name(&p.expense_line_id).unwrap_or_else(|| t("tx.uncategorized"))) " " (c.money(p.amount.abs()))
                                        @if let Some(pc) = p.paycheck_id.as_ref().and_then(|pc| m.paycheck(pc)) { " (" (short_date(pc.date)) ")" }
                                    }
                                }
                            }
                            span class=(if total.is_negative() { "tx-amt num" } else { "tx-amt num pos" }) { (c.money(total)) }
                            @if !archived {
                                button type="button" class="icon-btn" aria-label=(tf("tx.edit_label", &[("payee", &payee), ("date", &short_date(x.date))]))
                                    data-on:click=(format!("document.getElementById('{dlg}').showModal()")) { (icon("edit")) }
                                dialog id=(dlg) aria-labelledby=(format!("{dlg}-h")) {
                                    h2 id=(format!("{dlg}-h")) class="h3" { (t("split.edit_title")) }
                                    form class="tx-form" data-online-only
                                        data-on:submit__prevent=(format!("pz.checkSplit(el) && @post('/ui/transactions/{}', {{contentType: 'form'}})", x.id)) {
                                        (view_input(&view))
                                        (tx_fields(c, m, &format!("split-{g}"), &parts, default_date))
                                        div class="form-actions wide" {
                                            button type="button" class="btn" data-close-dialog { (t("common.cancel")) }
                                            button type="submit" class="btn primary" { (t("common.save")) }
                                        }
                                    }
                                    form class="dialog-danger" data-on:submit__prevent=(post_form(&format!("/ui/splits/{g}/delete"))) {
                                        (view_input(&view))
                                        input type="hidden" name="part_of" value=(x.id);
                                        button type="submit" class="btn small danger" data-confirm=(t("split.delete_confirm")) { (icon("trash")) " " (t("split.delete")) }
                                    }
                                }
                            }
                        }
                    }
 TxItem::Single(x) => {
                        @let base = serde_json::json!({
                            "date": x.date, "amount": x.amount.get(), "payee": x.payee, "notes": x.notes,
                            "expense_line_id": x.expense_line_id, "paycheck_id": x.paycheck_id,
                        }).to_string();
                        @let payee = x.payee.clone().unwrap_or_else(|| t("tx.no_payee"));
                        @let dlg = format!("edit-tx-{}", x.id);
                        li class="tx" data-tx=(x.id) {
                            span class="tx-date" { (short_date(x.date)) }
                            div class="tx-main" {
                                span class="tx-payee" { (payee) }
                                span class="tx-meta" {
                                    (line_name(&x.expense_line_id).unwrap_or_else(|| t("tx.uncategorized")))
                                    @if let Some(p) = x.paycheck_id.as_ref().and_then(|p| m.paycheck(p)) {
                                        " · " (tf("tx.from_paycheck", &[("date", &short_date(p.date))]))
                                    }
                                    @if let Some(n) = &x.notes { " · " (n) }
                                }
                            }
                            span class=(if x.amount.is_negative() { "tx-amt num" } else { "tx-amt num pos" }) { (c.money(x.amount)) }
                            @if !archived {
                                button type="button" class="icon-btn" aria-label=(tf("tx.edit_label", &[("payee", &payee), ("date", &short_date(x.date))]))
                                    data-on:click=(format!("document.getElementById('{dlg}').showModal()")) { (icon("edit")) }
                                dialog id=(dlg) aria-labelledby=(format!("{dlg}-h")) {
                                    h2 id=(format!("{dlg}-h")) class="h3" { (t("tx.edit_title")) }
                                    form class="tx-form" data-offline="update_transaction" data-month=(m.id) data-tx=(x.id) data-base=(base.clone())
                                        data-on:submit__prevent=(post_form(&format!("/ui/transactions/{}", x.id))) {
                                        (view_input(&view))
                                        (tx_fields(c, m, &format!("tx-{}", x.id), &[x], default_date))
                                        div class="form-actions wide" {
                                            button type="button" class="btn" data-close-dialog { (t("common.cancel")) }
                                            button type="submit" class="btn primary" { (t("common.save")) }
                                        }
                                    }
                                    form class="dialog-danger" data-offline="delete_transaction" data-month=(m.id) data-tx=(x.id) data-base=(base)
                                        data-on:submit__prevent=(post_form(&format!("/ui/transactions/{}/delete", x.id))) {
                                        (view_input(&view))
                                        button type="submit" class="btn small danger" data-confirm=(t("tx.delete_confirm")) { (icon("trash")) " " (t("tx.delete")) }
                                    }
                                }
                            }
                        }
                    }
 }
 }
                }
            }
        }
    }
}

// ----------------------------------------------------------------------
// Reports (spec §15)
// ----------------------------------------------------------------------

/// Line names of a category in either period, current period's order first.
fn union_line_names(a: &report::CategoryFigures, b: Option<&report::CategoryFigures>) -> Vec<String> {
    let mut names: Vec<String> = a.lines.iter().map(|l| l.name.clone()).collect();
    for l in b.map(|b| b.lines.as_slice()).unwrap_or_default() {
        if !names.contains(&l.name) {
            names.push(l.name.clone());
        }
    }
    names
}

#[allow(clippy::too_many_arguments)]
fn figures_table(c: &Ctx, id: &str, caption: &str, cur_label: &str, cur: &Figures, other_label: Option<&str>, other: Option<&Figures>, names: &[String]) -> Markup {
    let cat = |f: &Figures, n: &str| f.category(n).cloned().unwrap_or_default();
    html! {
        table class="table report" id=(id) {
            caption class="visually-hidden" { (caption) }
            thead {
                tr {
                    th scope="col" { (t("report.row")) }
                    th scope="col" class="num" { (cur_label) " " (t("report.planned")) }
                    th scope="col" class="num" { (cur_label) " " (t("report.actual")) }
                    @if let Some(ol) = other_label {
                        th scope="col" class="num" { (ol) " " (t("report.planned")) }
                        th scope="col" class="num" { (ol) " " (t("report.actual")) }
                    }
                }
            }
            tbody {
                tr data-row="income" {
                    th scope="row" { (t("report.income")) }
                    td class="num" { (c.money(cur.planned_income)) } td class="num" { (c.money(cur.actual_income)) }
                    @if other_label.is_some() {
                        @if let Some(o) = other { td class="num" { (c.money(o.planned_income)) } td class="num" { (c.money(o.actual_income)) } }
                        @else { td class="num" { "—" } td class="num" { "—" } }
                    }
                }
                tr data-row="expenses" {
                    th scope="row" { (t("report.expenses")) }
                    td class="num" { (c.money(cur.planned_expense)) } td class="num" { (c.money(cur.actual_expense)) }
                    @if other_label.is_some() {
                        @if let Some(o) = other { td class="num" { (c.money(o.planned_expense)) } td class="num" { (c.money(o.actual_expense)) } }
                        @else { td class="num" { "—" } td class="num" { "—" } }
                    }
                }
                @for n in names.iter().filter(|n| {
                    let a = cat(cur, n);
                    let b = other.map(|o| cat(o, n)).unwrap_or_default();
                    !(a.planned.is_zero() && a.actual.is_zero() && b.planned.is_zero() && b.actual.is_zero())
                }) {
                    @let a = cat(cur, n);
                    @let b = other.map(|o| cat(o, n));
                    @let line_names = union_line_names(&a, b.as_ref());
                    @let group = format!("{id}-{}", n.to_lowercase().replace(' ', "-"));
                    tr data-row=(format!("cat:{n}")) class="cat-row" {
                        th scope="row" {
                            @if line_names.is_empty() { (n) } @else {
                                button type="button" class="row-toggle" aria-expanded="false" data-toggle-rows=(group) { span class="caret" aria-hidden="true" {} (n) }
                            }
                        }
                        td class="num" { (c.money(a.planned)) } td class="num" { (c.money(a.actual)) }
                        @if other_label.is_some() {
                            @if let Some(b) = &b { td class="num" { (c.money(b.planned)) } td class="num" { (c.money(b.actual)) } }
                            @else { td class="num" { "—" } td class="num" { "—" } }
                        }
                    }
                    @for ln in &line_names {
                        @let la = a.lines.iter().find(|l| &l.name == ln).cloned().unwrap_or_default();
                        tr class="line-row" data-row=(format!("line:{n}:{ln}")) data-parent=(group) hidden {
                            th scope="row" { (ln) }
                            td class="num" { (c.money(la.planned)) } td class="num" { (c.money(la.actual)) }
                            @if other_label.is_some() {
                                @if let Some(b) = &b {
                                    @let lb = b.lines.iter().find(|l| &l.name == ln).cloned().unwrap_or_default();
                                    td class="num" { (c.money(lb.planned)) } td class="num" { (c.money(lb.actual)) }
                                } @else { td class="num" { "—" } td class="num" { "—" } }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn comparison(c: &Ctx, id: &str, title: &str, cmp: &Comparison) -> Markup {
    let cur = month_label(cmp.current_month);
    let oth = month_label(cmp.other_month);
    html! {
        section class="report-section" aria-labelledby=(format!("{id}-h")) {
            h2 id=(format!("{id}-h")) class="h3" { (title) }
            @if cmp.other.is_none() {
                p class="muted" { (tf("report.no_data", &[("month", &oth)])) }
            }
            div class="table-scroll" tabindex="0" role="region" aria-label=(title) {
                (figures_table(c, id, &tf("report.caption_vs", &[("a", &cur), ("b", &oth)]), &short_month(cmp.current_month), &cmp.current,
                    Some(&short_month(cmp.other_month)), cmp.other.as_ref(), &cmp.category_names))
            }
        }
    }
}

fn short_month(d: NaiveDate) -> String {
    d.format("%b %Y").to_string()
}

/// Part-to-whole donut: top five slices plus "Other", with a legend that
/// carries names, amounts and shares (identity is never colour alone).
fn donut(c: &Ctx, id: &str, title: &str, center_label: &str, items: &[(String, Cents)]) -> Markup {
    let mut items: Vec<(String, Cents)> = items.iter().filter(|(_, v)| v.is_positive()).cloned().collect();
    items.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
    if items.len() > 6 {
        let other: Cents = items[5..].iter().map(|(_, v)| *v).sum();
        items.truncate(5);
        items.push((t("chart.other"), other));
    }
    let total: Cents = items.iter().map(|(_, v)| *v).sum();
    let circ = 2.0 * std::f64::consts::PI * 40.0;
    let gap = if items.len() > 1 { 2.0 } else { 0.0 };
    let pct = |v: Cents| if total.is_positive() { v.get() as f64 * 100.0 / total.get() as f64 } else { 0.0 };
    // (name, value, slot class, dash length, offset) per slice.
    let mut offset = 0.0_f64;
    let segs: Vec<(String, Cents, String, f64, f64)> = items
        .iter()
        .enumerate()
        .map(|(i, (name, v))| {
            let len = if total.is_positive() { (v.get() as f64 / total.get() as f64) * circ } else { 0.0 };
            let slot = if name == &t("chart.other") { "other".to_string() } else { (i + 1).to_string() };
            let seg = (name.clone(), *v, slot, (len - gap).max(0.5), offset);
            offset += len;
            seg
        })
        .collect();
    html! {
        figure class="chart-card" id=(id) {
            figcaption class="chart-title" { (title) }
            @if items.is_empty() {
                p class="muted" { (t("chart.nothing")) }
            } @else {
                div class="donut-wrap" {
                    svg class="donut" viewBox="0 0 100 100" role="img" aria-label=(tf("chart.donut_label", &[("title", title), ("total", &c.money(total))])) {
                        circle class="donut-track" cx="50" cy="50" r="40" {}
                        @for (name, v, slot, dash, off) in &segs {
                            circle class=(format!("donut-seg s-{slot}")) cx="50" cy="50" r="40"
                                stroke-dasharray=(format!("{dash:.2} {:.2}", circ - dash)) stroke-dashoffset=(format!("{:.2}", -off))
                                transform="rotate(-90 50 50)" {
                                title { (name) ": " (c.money(*v)) " (" (format!("{:.0}", pct(*v))) "%)" }
                            }
                        }
                        text class="donut-total" x="50" y="48" text-anchor="middle" { (c.money(total)) }
                        text class="donut-sub" x="50" y="60" text-anchor="middle" { (center_label) }
                    }
                    ul class="legend" {
                        @for (name, v, slot, _, _) in &segs {
                            li data-slice=(name) {
                                span class=(format!("swatch s-{slot}")) aria-hidden="true" {}
                                span class="legend-name" { (name) }
                                span class="legend-val" { (c.money(*v)) " · " (format!("{:.0}", pct(*v))) "%" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A single ratio against guide thresholds, as a ring gauge with a status
/// label (never colour alone).
fn ratio_gauge(id: &str, title: &str, ratio_bp: Option<i64>, good_max: i64, ok_max: i64, higher_is_better: bool, detail: Markup) -> Markup {
    let circ = 2.0 * std::f64::consts::PI * 40.0;
    let (status, label) = match ratio_bp {
        None => ("none", t("chart.no_income")),
        Some(bp) => {
            let good = if higher_is_better { bp >= good_max } else { bp <= good_max };
            let ok = if higher_is_better { bp >= ok_max } else { bp <= ok_max };
            if good { ("good", t("chart.status_good")) } else if ok { ("ok", t("chart.status_ok")) } else { ("high", if higher_is_better { t("chart.status_low") } else { t("chart.status_high") }) }
        }
    };
    let pct = ratio_bp.map_or(0.0, |bp| (bp as f64 / 100.0).clamp(0.0, 100.0));
    let dash = circ * pct / 100.0;
    html! {
        figure class="chart-card" id=(id) {
            figcaption class="chart-title" { (title) }
            div class="donut-wrap" {
                svg class=(format!("gauge g-{status}")) viewBox="0 0 100 100" role="img"
                    aria-label=(format!("{title}: {}", ratio_bp.map_or_else(|| "—".to_string(), |bp| format!("{:.1}%", bp as f64 / 100.0)))) {
                    circle class="donut-track" cx="50" cy="50" r="40" {}
                    @if dash > 0.0 {
                        circle class="gauge-fill" cx="50" cy="50" r="40" stroke-dasharray=(format!("{dash:.2} {:.2}", circ - dash)) transform="rotate(-90 50 50)" {}
                    }
                    text class="donut-total" x="50" y="53" text-anchor="middle" {
                        (ratio_bp.map_or_else(|| "—".to_string(), |bp| format!("{:.0}%", bp as f64 / 100.0)))
                    }
                }
                div class="gauge-info" {
                    p class=(format!("gauge-status st-{status}")) { (icon(if status == "good" { "check" } else { "alert" })) " " (label) }
                    (detail)
                }
            }
        }
    }
}

/// Planned vs spent per category for the selected month (pure CSS bars).
fn category_chart(c: &Ctx, f: &Figures) -> Markup {
    let rows: Vec<&report::CategoryFigures> = f.categories.iter().filter(|x| x.planned.is_positive() || x.actual.is_positive()).collect();
    let max = rows.iter().map(|x| x.planned.max(x.actual).get()).max().unwrap_or(0).max(1);
    let w = |v: Cents| (v.get().saturating_mul(1000) / max) as f64 / 10.0;
    html! {
        section class="report-section card chart" aria-labelledby="chart-h" {
            h2 id="chart-h" class="h3" { (t("report.chart")) }
            @if rows.is_empty() {
                p class="muted" { (t("report.chart_empty")) }
            } @else {
                p class="chart-legend" aria-hidden="true" {
                    span class="swatch planned" {} (t("report.planned")) " "
                    span class="swatch actual" {} (t("report.spent"))
                }
                ul class="bars" {
                    @for r in &rows {
                        li class=(if r.actual > r.planned { "bar-row over" } else { "bar-row" }) data-bar=(r.name) {
                            span class="bar-name" { (r.name) }
                            span class="bar-track" aria-hidden="true" {
                                span class="bar planned" style=(format!("width: {:.1}%", w(r.planned))) {}
                                span class="bar actual" style=(format!("width: {:.1}%", w(r.actual))) {}
                            }
                            span class="bar-vals" { (tf("report.bar_vals", &[("actual", &c.money(r.actual)), ("planned", &c.money(r.planned))])) }
                        }
                    }
                }
            }
        }
    }
}

fn report_tabs(m: &Month, q: &ReportQuery) -> Markup {
    let tab = |key: &str, label: String| {
        let nq = ReportQuery { tab: Some(key.into()), ..ReportQuery::default() };
        html! {
            a href=(format!("/months/{}/reports{}", m.id, nq.query_string())) class=(if q.tab() == key { "rtab active" } else { "rtab" })
                aria-current=[(q.tab() == key).then_some("page")] { (label) }
        }
    };
    html! {
        nav class="report-tabs" aria-label=(t("report.tabs")) {
            (tab("summary", t("report.tab_summary")))
            (tab("trends", t("report.tab_trends")))
            (tab("payees", t("report.tab_payees")))
            (tab("export", t("report.tab_export")))
        }
    }
}

fn csv_link(m: &Month, kind: &str, q: &ReportQuery) -> Markup {
    html! {
        a class="btn small" href=(format!("/months/{}/reports/export/{kind}.csv{}", m.id, q.query_string())) download {
            (t("report.download_csv"))
        }
    }
}

/// Tiny inline bar chart for a trend row.
fn sparkline(values: &[Option<Cents>]) -> Markup {
    let max = values.iter().flatten().map(|v| v.get()).max().unwrap_or(0).max(1);
    let n = values.len().max(1);
    let w = 6 * n + 2 * (n - 1);
    html! {
        svg class="spark" width=(w) height="24" viewBox=(format!("0 0 {w} 24")) aria-hidden="true" {
            @for (i, v) in values.iter().enumerate() {
                @let h = v.map_or(0, |v| (v.get().saturating_mul(22) / max).clamp(1, 22));
                rect x=(i * 8) y=(24 - h) width="6" height=(h) rx="1" {}
            }
        }
    }
}

fn render_trends(c: &Ctx, m: &Month, all: &[Month], q: &ReportQuery) -> Markup {
    let n = q.months();
    let tr = report::trend(m.year_month, all, n);
    let pick = |f: &Figures, cat: &str, line: Option<&str>| -> Cents {
        let Some(cf) = f.category(cat) else { return Cents::ZERO };
        let (p, a) = match line {
            Some(l) => cf.lines.iter().find(|x| x.name == l).map_or((Cents::ZERO, Cents::ZERO), |x| (x.planned, x.actual)),
            None => (cf.planned, cf.actual),
        };
        if q.planned() { p } else { a }
    };
    let mut rows: Vec<(String, Option<String>, String)> = Vec::new();
    for cat in &tr.category_names {
        rows.push((cat.clone(), None, cat.clone()));
        if q.lines() {
            let mut lines: Vec<String> = Vec::new();
            for f in tr.figures.iter().flatten() {
                for l in f.category(cat).map(|c| c.lines.as_slice()).unwrap_or_default() {
                    if !lines.contains(&l.name) { lines.push(l.name.clone()); }
                }
            }
            for l in lines {
                rows.push((cat.clone(), Some(l.clone()), format!("{cat} › {l}")));
            }
        }
    }
    html! {
        section class="report-section" aria-labelledby="trend-h" {
            div class="section-head" {
                h2 id="trend-h" class="h3" { (tf("report.trend_title", &[("n", &n.to_string())])) }
                (csv_link(m, "trends", q))
            }
            form class="report-filters" method="get" action=(format!("/months/{}/reports", m.id)) {
                input type="hidden" name="tab" value="trends";
                div class="filter" {
                    label for="rf-n" { (t("report.window")) }
                    select id="rf-n" name="n" { @for k in [3usize, 6, 12] { option value=(k) selected[k == n] { (tf("report.months_n", &[("n", &k.to_string())])) } } }
                }
                div class="filter" {
                    label for="rf-level" { (t("report.rows")) }
                    select id="rf-level" name="level" {
                        option value="category" selected[!q.lines()] { (t("report.by_category")) }
                        option value="line" selected[q.lines()] { (t("report.by_line")) }
                    }
                }
                div class="filter" {
                    label for="rf-metric" { (t("report.values")) }
                    select id="rf-metric" name="metric" {
                        option value="spent" selected[!q.planned()] { (t("report.metric_spent")) }
                        option value="planned" selected[q.planned()] { (t("report.metric_planned")) }
                    }
                }
                button type="submit" class="btn small" { (t("report.apply")) }
            }
            div class="table-scroll" tabindex="0" role="region" aria-label=(t("report.tab_trends")) {
                table class="table report trend" id="trend" {
                    caption class="visually-hidden" { (tf("report.trend_title", &[("n", &n.to_string())])) }
                    thead { tr {
                        th scope="col" { (t("report.row")) }
                        @for ym in &tr.months { th scope="col" class="num" { (short_month(*ym)) } }
                        th scope="col" { span class="visually-hidden" { (t("report.shape")) } }
                    } }
                    tbody {
                        @let income: Vec<Option<Cents>> = tr.figures.iter().map(|f| f.as_ref().map(|f| if q.planned() { f.planned_income } else { f.actual_income })).collect();
                        @let spend: Vec<Option<Cents>> = tr.figures.iter().map(|f| f.as_ref().map(|f| if q.planned() { f.planned_expense } else { f.actual_expense })).collect();
                        @for (label, key, vals) in [(t("report.income"), "income", &income), (t("report.expenses"), "expenses", &spend)] {
                            tr data-row=(key) class="total-row" {
                                th scope="row" { (label) }
                                @for v in vals.iter() { td class="num" { (v.map_or_else(|| "—".to_string(), |v| c.money(v))) } }
                                td { (sparkline(vals)) }
                            }
                        }
                        @for (cat, line, label) in &rows {
                            @let vals: Vec<Option<Cents>> = tr.figures.iter().map(|f| f.as_ref().map(|f| pick(f, cat, line.as_deref()))).collect();
                            @if vals.iter().flatten().any(|v| !v.is_zero()) {
                                tr data-row=(format!("{}:{label}", if line.is_some() { "line" } else { "cat" })) class=(if line.is_some() { "line-row" } else { "cat-row" }) {
                                    th scope="row" { (label) }
                                    @for v in &vals { td class="num" { (v.map_or_else(|| "—".to_string(), |v| c.money(v))) } }
                                    td { (sparkline(&vals)) }
                                }
                            }
                        }
                    }
                }
            }
            p class="muted small" { (t("report.trend_note")) }
        }
    }
}

fn render_payees(c: &Ctx, m: &Month, all: &[Month], q: &ReportQuery) -> Markup {
    let (from, to) = q.range(m.year_month);
    let rows = report::payees(all, from, to);
    let rq = ReportQuery { tab: Some("payees".into()), from: Some(from.to_string()), to: Some(to.to_string()), ..ReportQuery::default() };
    html! {
        section class="report-section" aria-labelledby="payee-h" {
            div class="section-head" {
                h2 id="payee-h" class="h3" { (t("report.payee_title")) }
                (csv_link(m, "payees", &rq))
            }
            (range_form(m, "payees", from, to))
            @if rows.is_empty() {
                (empty_state(&t("report.payee_empty_title"), &t("report.payee_empty_body"), None))
            } @else {
                div class="table-scroll" tabindex="0" role="region" aria-label=(t("report.payee_title")) {
                    table class="table report" id="payees" {
                        caption class="visually-hidden" { (t("report.payee_title")) }
                        thead { tr {
                            th scope="col" { (t("tx.payee")) } th scope="col" class="num" { (t("report.count")) }
                            th scope="col" class="num" { (t("report.spent_col")) } th scope="col" class="num" { (t("report.received_col")) }
                        } }
                        tbody {
                            @for r in &rows {
                                tr data-row=(format!("payee:{}", r.payee)) {
                                    th scope="row" { (r.payee) }
                                    td class="num" { (r.count) }
                                    td class="num" { (c.money(r.spent)) }
                                    td class="num" { (c.money(r.received)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn range_form(m: &Month, tab: &str, from: NaiveDate, to: NaiveDate) -> Markup {
    html! {
        form class="report-filters" method="get" action=(format!("/months/{}/reports", m.id)) {
            input type="hidden" name="tab" value=(tab);
            div class="filter" { label for="rf-from" { (t("report.from")) } input id="rf-from" type="date" name="from" value=(from.to_string()) required; }
            div class="filter" { label for="rf-to" { (t("report.to")) } input id="rf-to" type="date" name="to" value=(to.to_string()) required; }
            button type="submit" class="btn small" { (t("report.apply")) }
        }
    }
}

fn render_export(m: &Month, q: &ReportQuery) -> Markup {
    let (from, to) = q.range(m.year_month);
    html! {
        section class="report-section card" aria-labelledby="exp-month-h" {
            h2 id="exp-month-h" class="h3" { (tf("report.export_month", &[("month", &month_label(m.year_month))])) }
            div class="row-actions" {
                a class="btn" href=(format!("/months/{}/export.csv", m.id)) download { (t("export.csv")) }
                a class="btn" href=(format!("/months/{}/snapshot.json", m.id)) download { (t("export.snapshot")) }
            }
        }
        section class="report-section card" aria-labelledby="exp-tx-h" {
            h2 id="exp-tx-h" class="h3" { (t("report.export_tx")) }
            p class="muted" { (t("report.export_tx_body")) }
            form class="report-filters" method="get" data-download-form action=(format!("/months/{}/reports/export/transactions.csv", m.id)) {
                div class="filter" { label for="ex-from" { (t("report.from")) } input id="ex-from" type="date" name="from" value=(from.to_string()) required; }
                div class="filter" { label for="ex-to" { (t("report.to")) } input id="ex-to" type="date" name="to" value=(to.to_string()) required; }
                button type="submit" class="btn" { (t("report.download_csv")) }
            }
        }
        section class="report-section card" aria-labelledby="exp-rep-h" {
            h2 id="exp-rep-h" class="h3" { (t("report.export_reports")) }
            div class="row-actions" {
                @for (kind, label) in [("mom", "report.mom"), ("ytd", "report.ytd"), ("yoy", "report.yoy"), ("trends", "report.tab_trends"), ("payees", "report.tab_payees")] {
                    a class="btn" href=(format!("/months/{}/reports/export/{kind}.csv", m.id)) download { (t(label)) " (CSV)" }
                }
            }
        }
    }
}

pub fn render_reports(c: &Ctx, m: &Month, all: &[Month], q: &ReportQuery) -> Markup {
    html! {
        h1 { (tf("report.title", &[("month", &month_label(m.year_month))])) }
        (report_tabs(m, q))
        @match q.tab() {
            "trends" => { (render_trends(c, m, all, q)) },
            "payees" => { (render_payees(c, m, all, q)) },
            "export" => { (render_export(m, q)) },
            _ => { (render_summary(c, m, all, q)) },
        }
    }
}

fn render_summary(c: &Ctx, m: &Month, all: &[Month], q: &ReportQuery) -> Markup {
    let mom = report::month_over_month(m, all);
    let yoy = report::year_over_year(m, all);
    let ytd = report::year_to_date(m, all);
    let names: Vec<String> = ytd.figures.categories.iter().map(|c| c.name.clone()).collect();
    let fig = report::month_figures(m);
    let planned_items: Vec<(String, Cents)> = fig.categories.iter().map(|x| (x.name.clone(), x.planned)).collect();
    let spent_items: Vec<(String, Cents)> = fig.categories.iter().map(|x| (x.name.clone(), x.actual)).collect();
    let debt_lines: Vec<&ExpenseLine> = m.expense_lines.iter().filter(|l| m.is_debt_line(&l.id)).collect();
    let debt_pay: Cents = debt_lines.iter().map(|l| m.line_planned(&l.id)).sum();
    let debt_min: Cents = debt_lines.iter().filter_map(|l| l.minimum_payment).sum();
    let debt_bal: Cents = debt_lines.iter().filter_map(|l| l.current_balance).sum();
    let income = fig.planned_income;
    let bp = |part: Cents| (income.is_positive()).then(|| part.get().saturating_mul(10_000) / income.get());
    let saving: Cents = m.categories.iter().filter(|x| x.name.eq_ignore_ascii_case("Saving")).flat_map(|x| m.lines_of(&x.id)).map(|l| m.line_planned(&l.id)).sum();
    html! {
        (summary_cards(c, m))
        section class="chart-grid" aria-label=(t("chart.section")) {
            (donut(c, "donut-planned", &t("chart.planned_title"), &t("chart.planned_center"), &planned_items))
            (donut(c, "donut-spent", &t("chart.spent_title"), &t("chart.spent_center"), &spent_items))
            (ratio_gauge("gauge-dti", &t("chart.dti_title"), bp(debt_pay), 1_500, 3_600, false, html! {
                p class="small" { (tf("chart.dti_detail", &[("pay", &c.money(debt_pay)), ("income", &c.money(income))])) }
                @if debt_min.is_positive() { p class="small muted" { (tf("chart.dti_min", &[("amount", &c.money(debt_min))])) } }
                @if debt_bal.is_positive() {
                    p class="small muted" { (tf("chart.debt_balance", &[
                        ("amount", &c.money(debt_bal)),
                        ("months", &if income.is_positive() { format!("{:.1}", debt_bal.get() as f64 / income.get() as f64) } else { "—".into() }),
                    ])) }
                }
                p class="small muted" { (t("chart.dti_guide")) }
            }))
            (ratio_gauge("gauge-savings", &t("chart.savings_title"), bp(saving), 2_000, 1_000, true, html! {
                p class="small" { (tf("chart.savings_detail", &[("amount", &c.money(saving)), ("income", &c.money(income))])) }
                p class="small muted" { (t("chart.savings_guide")) }
            }))
        }
        (category_chart(c, &fig))
        div class="section-head" { span {} (csv_link(m, "mom", q)) }
        (comparison(c, "mom", &t("report.mom"), &mom))
        section class="report-section" aria-labelledby="ytd-h" {
            div class="section-head" {
                h2 id="ytd-h" class="h3" { (t("report.ytd")) }
                (csv_link(m, "ytd", q))
            }
            p class="muted" { (tf("report.ytd_range", &[("from", &ytd.from.format("%b %-d, %Y").to_string()), ("through", &month_label(ytd.through)), ("n", &ytd.months_included.to_string())])) }
            div class="table-scroll" tabindex="0" role="region" aria-label=(t("report.ytd")) {
                (figures_table(c, "ytd", &t("report.ytd"), &t("report.ytd_short"), &ytd.figures, None, None, &names))
            }
        }
        div class="section-head" { span {} (csv_link(m, "yoy", q)) }
        (comparison(c, "yoy", &t("report.yoy"), &yoy))
        p class="muted small" { (t("report.drill_hint")) }
    }
}

/// CSV downloads for every report (user request #11).
pub async fn report_csv(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path((id, kind)): Path<(Id, String)>, Query(q): Query<ReportQuery>) -> Page {
    let m = st.load(&user.0, &id).await?.month;
    let all = st.all_months(&user.0).await?;
    let ym = m.year_month.format("%Y-%m").to_string();
    let (rows, name): (Vec<Vec<String>>, String) = match kind.as_str() {
        "mom.csv" | "yoy.csv" => {
            let cmp = if kind == "mom.csv" { report::month_over_month(&m, &all) } else { report::year_over_year(&m, &all) };
            let periods = vec![
                (short_month(cmp.current_month), Some(&cmp.current)),
                (short_month(cmp.other_month), cmp.other.as_ref()),
            ];
            (export::figures_rows(&periods), format!("{}-{}.csv", kind.trim_end_matches(".csv"), ym))
        }
        "ytd.csv" => {
            let y = report::year_to_date(&m, &all);
            (export::figures_rows(&[(format!("YTD {}", y.through.format("%Y")), Some(&y.figures))]), format!("ytd-{ym}.csv"))
        }
        "trends.csv" => {
            let tr = report::trend(m.year_month, &all, q.months());
            let periods: Vec<(String, Option<&Figures>)> = tr.months.iter().zip(&tr.figures).map(|(d, f)| (short_month(*d), f.as_ref())).collect();
            (export::figures_rows(&periods), format!("trends-{}m-{ym}.csv", q.months()))
        }
        "payees.csv" => {
            let (from, to) = q.range(m.year_month);
            let mut rows = vec![vec!["payee".to_string(), "payments".into(), "spent".into(), "received".into()]];
            for r in report::payees(&all, from, to) {
                rows.push(vec![r.payee, r.count.to_string(), crate::money::plain(r.spent), crate::money::plain(r.received)]);
            }
            (rows, format!("payees-{from}-to-{to}.csv"))
        }
        "transactions.csv" => {
            let (from, to) = q.range(m.year_month);
            let mut rows = vec![vec!["date".to_string(), "month".into(), "payee".into(), "category".into(), "line".into(), "paycheck".into(), "amount".into(), "notes".into(), "split_group".into()]];
            for (mo, t) in report::transactions_between(&all, from, to) {
                let line = t.expense_line_id.as_ref().and_then(|l| mo.expense_line(l));
                rows.push(vec![
                    t.date.to_string(),
                    mo.year_month.format("%Y-%m").to_string(),
                    t.payee.clone().unwrap_or_default(),
                    line.and_then(|l| mo.category(&l.category_id)).map(|c| c.name.clone()).unwrap_or_default(),
                    line.map(|l| l.name.clone()).unwrap_or_default(),
                    t.paycheck_id.as_ref().and_then(|p| mo.paycheck(p)).map(|p| p.date.to_string()).unwrap_or_default(),
                    crate::money::plain(t.amount),
                    t.notes.clone().unwrap_or_default(),
                    t.split_group.as_ref().map(ToString::to_string).unwrap_or_default(),
                ]);
            }
            (rows, format!("transactions-{from}-to-{to}.csv"))
        }
        _ => return Err(AppError::NotFound),
    };
    Ok(export::csv_download(&name, export::csv_rows(&rows)))
}

// ----------------------------------------------------------------------
// Months list (spec §2.1, §2.11, §2.12)
// ----------------------------------------------------------------------

pub fn render_months(c: &Ctx, metas: &[MonthMeta], months: &[Month], archived: bool) -> Markup {
    let view = View::Months { archived };
    let default_ym = recurrence::first_of_month(c.today);
    let prev = report::previous_month(default_ym);
    let default_source = months.iter().find(|x| x.year_month == prev).or_else(|| months.last()).map(|x| x.id.clone());
    html! {
        div class="section-head" {
            h1 { (t("months.title")) }
            button type="button" class="btn primary" id="new-month-btn" data-on:click="document.getElementById('new-month').showModal()" { "+ " (t("months.new")) }
        }
        p class="muted" { (t("months.intro")) }
        p {
            @if archived { a href="/months" { (t("months.hide_archived")) } }
            @else { a href="/months?archived=1" { (t("months.show_archived")) } }
        }
        @if metas.is_empty() {
            (empty_state(&t("months.empty_title"), &t("months.empty_body"),
                Some(html! { button type="button" class="btn primary" data-on:click="document.getElementById('new-month').showModal()" { (t("months.new")) } })))
        }
        ul class="month-list" {
            @for meta in metas {
                @let full = months.iter().find(|x| x.id == meta.id);
                @let label = month_label(meta.year_month);
                li class={ "month-card" @if meta.archived { " archived" } } data-month=(meta.year_month.format("%Y-%m")) {
                    a class="month-link" href=(format!("/months/{}", meta.id)) {
                        span class="month-title" { (label) }
                        span class="muted" {
                            (t(&format!("status.month_{}", meta.status.as_str())))
                            @if meta.reassigning { " · " (t("status.reassigning_short")) }
                            @if meta.archived { " · " (t("status.archived_short")) }
                            @if let Some(f) = full {
                                " · "
                                @if f.is_zero() { (t("status.zero_short")) }
                                @else { (tf("status.left_short", &[("amount", &c.money(f.zero_difference()))])) }
                            }
                        }
                    }
                    div class="month-actions" {
                        @if meta.archived {
                            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/months/{}/restore", meta.id))) {
                                (view_input(&view))
                                button type="submit" class="btn small" { (t("months.restore")) span class="visually-hidden" { " " (label) } }
                            }
                        } @else {
                            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/months/{}/archive", meta.id))) {
                                (view_input(&view))
                                button type="submit" class="btn small" { (t("months.archive")) span class="visually-hidden" { " " (label) } }
                            }
                        }
                        details class="danger-zone" {
                            summary { (t("months.delete")) span class="visually-hidden" { " " (label) } }
                            form data-on:submit__prevent=(post_form(&format!("/ui/months/{}/delete", meta.id))) {
                                (view_input(&view))
                                p class="warn-text" { (tf("months.delete_warning", &[("month", &label)])) }
                                label for=(format!("confirm-{}", meta.id)) { (tf("months.delete_type", &[("code", &meta.year_month.format("%Y-%m").to_string())])) }
                                input id=(format!("confirm-{}", meta.id)) type="text" name="confirm" required autocomplete="off";
                                button type="submit" class="btn small danger" { (t("months.delete_forever")) }
                            }
                        }
                    }
                }
            }
        }
        section class="card" aria-labelledby="restore-h" {
            h2 id="restore-h" class="h3" { (t("restore.title")) }
            p class="muted" { (t("restore.body")) }
            form id="restore-form" data-restore {
                label for="restore-file" { (t("restore.file")) }
                input id="restore-file" type="file" accept="application/json,.json" required;
                label class="check" { input type="checkbox" name="replace" value="1"; " " (t("restore.replace")) }
                button type="submit" class="btn" { (t("restore.submit")) }
            }
        }
        dialog id="new-month" aria-labelledby="new-month-h" data-signals="{_copymode: 'blank'}" {
            form data-on:submit__prevent=(post_form("/ui/months")) {
                (view_input(&view))
                h2 id="new-month-h" { (t("months.new")) }
                label for="new-month-ym" { (t("months.which")) }
                input id="new-month-ym" type="month" name="year_month" required value=(default_ym.format("%Y-%m")) data-new-month;
                fieldset {
                    legend { (t("months.start_with")) }
                    label class="radio" { input type="radio" name="copy_mode" value="blank" data-bind:_copymode checked; " " span { strong { (t("copy.blank")) } br; span class="muted small" { (t("copy.blank_desc")) } } }
                    label class="radio" { input type="radio" name="copy_mode" value="structure" data-bind:_copymode disabled[months.is_empty()]; " " span { strong { (t("copy.structure")) } br; span class="muted small" { (t("copy.structure_desc")) } } }
                    label class="radio" { input type="radio" name="copy_mode" value="structure_and_planned" data-bind:_copymode disabled[months.is_empty()]; " " span { strong { (t("copy.planned")) } br; span class="muted small" { (t("copy.planned_desc")) } } }
                }
                @if !months.is_empty() {
                    div class="field" data-show="$_copymode != 'blank'" {
                        label for="new-month-source" { (t("months.source")) }
                        select id="new-month-source" name="source_month_id" {
                            @for x in months.iter().rev() {
                                option value=(x.id) selected[Some(&x.id) == default_source.as_ref()] { (month_label(x.year_month)) }
                            }
                        }
                    }
                }
                div class="dialog-actions" {
                    button type="button" class="btn" data-on:click="el.closest('dialog').close()" { (t("common.cancel")) }
                    button type="submit" class="btn primary" { (t("months.create")) }
                }
            }
        }
    }
}

// ----------------------------------------------------------------------
// Settings (spec §13.5, §13.9)
// ----------------------------------------------------------------------

/// IANA zones grouped by region (e.g. "America" → ["America/Chicago", …]).
fn timezone_groups() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut groups: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for tz in chrono_tz::TZ_VARIANTS.iter() {
        let name = tz.name();
        let Some((region, _)) = name.split_once('/') else { continue };
        if matches!(region, "Etc" | "SystemV" | "US" | "Canada" | "Brazil" | "Chile" | "Mexico") {
            continue;
        }
        match groups.iter_mut().find(|(r, _)| *r == region) {
            Some((_, v)) => v.push(name),
            None => groups.push((region, vec![name])),
        }
    }
    groups.push(("UTC", vec!["UTC"]));
    groups
}

pub fn render_settings(c: &Ctx) -> Markup {
    let view = View::Settings;
    html! {
        h1 { (t("settings.title")) }
        section class="card" aria-labelledby="acct-h" {
            h2 id="acct-h" class="h3" { (t("settings.account")) }
            p { (tf("settings.signed_in_as", &[("email", &c.user.email)])) }
            div class="row-actions" {
                form class="inline" data-on:submit__prevent=(post_form("/ui/logout")) {
                    button type="submit" class="btn" { (t("settings.logout")) }
                }
                form class="inline" data-on:submit__prevent=(post_form("/ui/logout-all")) {
                    button type="submit" class="btn danger" data-confirm=(t("settings.logout_all_confirm")) { (t("settings.logout_all")) }
                }
            }
        }
        section class="card" aria-labelledby="cur-h" {
            h2 id="cur-h" class="h3" { (t("settings.currency")) }
            p class="muted" { (tf("settings.currency_now", &[("code", &c.user.currency)])) " " (t("settings.currency_explain")) }
            form data-on:submit__prevent=(post_form("/ui/settings/currency")) {
                (view_input(&view))
                div class="field" {
                    label for="currency" { (t("settings.new_currency")) }
                    select id="currency" name="currency" {
                        @for cur in crate::money::CURRENCIES {
                            option value=(cur.code) selected[cur.code == c.user.currency] { (cur.code) " — " (cur.name) }
                        }
                    }
                }
                div class="field" {
                    label for="rate" { (tf("settings.rate", &[("from", &c.user.currency)])) }
                    div class="inline-field" {
                        input id="rate" type="text" inputmode="decimal" name="rate" required placeholder="0.92" aria-describedby="rate-hint";
                        button type="submit" class="btn primary" data-confirm=(t("settings.currency_confirm")) { (t("settings.convert")) }
                    }
                    p id="rate-hint" class="hint" { (t("settings.rate_hint")) }
                }
            }
        }
        section class="card" aria-labelledby="tz-h" {
            h2 id="tz-h" class="h3" { (t("settings.timezone")) }
            p class="muted" { (tf("settings.today_is", &[("date", &c.today.format("%A, %B %-d, %Y").to_string())])) }
            form data-on:submit__prevent=(post_form("/ui/settings/timezone")) {
                (view_input(&view))
                label for="timezone" { (t("settings.timezone")) }
                div class="inline-field" {
                    select id="timezone" name="timezone" {
                        @if !timezone_groups().iter().any(|(_, z)| z.contains(&c.user.timezone.as_str())) {
                            option value=(c.user.timezone) selected { (c.user.timezone) }
                        }
                        @for (region, zones) in &timezone_groups() {
                            optgroup label=(region) {
                                @for z in zones {
                                    option value=(z) selected[*z == c.user.timezone] { (z.split_once('/').map_or(*z, |(_, city)| city).replace('_', " ").replace('/', " / ")) }
                                }
                            }
                        }
                    }
                    button type="submit" class="btn" { (t("common.save")) }
                }
            }
        }
    }
}
