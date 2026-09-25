//! Datastar web UI (spec §3, §14). Pages render a shell with a skeleton
//! loader; the content is then fetched and morphed in over SSE. Every
//! mutation answers with SSE that re-renders the current view, so derived
//! numbers (Safe-to-Spend, category totals, zero status) update live.

pub mod actions;
pub mod pages;

use crate::auth;
use crate::i18n::{client_bundle, t, tf};
use crate::money;
use crate::Shared;
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use chrono::NaiveDate;
use maud::{html, Markup, PreEscaped, DOCTYPE};
use paycheckzero_core::{Cents, Id};
use paycheckzero_storage::UserRecord;

pub fn routes(state: Shared) -> Router<Shared> {
    let protected = Router::new()
        .route("/", get(pages::home))
        .route("/months", get(pages::months_page))
        .route("/months/content", get(pages::months_content))
        .route("/months/{id}", get(pages::month_home))
        .route("/months/{id}/paychecks/{pid}", get(pages::paycheck_page))
        .route("/months/{id}/paychecks/{pid}/content", get(pages::paycheck_content))
        .route("/months/{id}/overview", get(pages::overview_page))
        .route("/months/{id}/overview/content", get(pages::overview_content))
        .route("/months/{id}/income", get(pages::income_page))
        .route("/months/{id}/income/content", get(pages::income_content))
        .route("/months/{id}/income/suggestions", get(actions::income_suggestions))
        .route("/months/{id}/transactions", get(pages::transactions_page))
        .route("/months/{id}/transactions/content", get(pages::transactions_content))
        .route("/months/{id}/reports", get(pages::reports_page))
        .route("/months/{id}/reports/content", get(pages::reports_content))
        .route("/months/{id}/reports/export/{kind}", get(pages::report_csv))
        .route("/months/{id}/export.csv", get(pages::export_csv))
        .route("/months/{id}/snapshot.json", get(pages::export_snapshot))
        .route("/settings", get(pages::settings_page))
        .route("/settings/content", get(pages::settings_content))
        .route("/ui/logout", post(actions::logout))
        .route("/ui/logout-all", post(actions::logout_all))
        .route("/ui/settings/currency", post(actions::change_currency))
        .route("/ui/settings/timezone", post(actions::change_timezone))
        .route("/ui/months", post(actions::create_month))
        .route("/ui/months/import", post(actions::import_month))
        .route("/ui/months/{id}/archive", post(actions::archive_month))
        .route("/ui/months/{id}/restore", post(actions::restore_month))
        .route("/ui/months/{id}/delete", post(actions::delete_month))
        .route("/ui/months/{id}/lock", post(actions::lock_month))
        .route("/ui/months/{id}/reassign/begin", post(actions::begin_reassign))
        .route("/ui/months/{id}/reassign/finish", post(actions::finish_reassign))
        .route("/ui/months/{id}/income", post(actions::add_income))
        .route("/ui/months/{id}/categories", post(actions::add_category))
        .route("/ui/months/{id}/lines", post(actions::add_line))
        .route("/ui/months/{id}/fund", post(actions::fund))
        .route("/ui/months/{id}/place", post(actions::place))
        .route("/ui/months/{id}/transactions", post(actions::add_transaction))
        .route("/ui/income/{id}", post(actions::update_income))
        .route("/ui/income/{id}/delete", post(actions::delete_income))
        .route("/ui/income/{id}/paychecks", post(actions::add_paycheck))
        .route("/ui/paychecks/{id}/planned", post(actions::paycheck_planned))
        .route("/ui/paychecks/{id}/actual", post(actions::paycheck_actual))
        .route("/ui/paychecks/{id}/status", post(actions::paycheck_status))
        .route("/ui/paychecks/{id}/apply-actual", post(actions::apply_actual))
        .route("/ui/paychecks/{id}/delete", post(actions::delete_paycheck))
        .route("/ui/paychecks/{id}/give", post(actions::give))
        .route("/ui/paychecks/{pid}/lines/{lid}", post(actions::set_allocation))
        .route("/ui/categories/{id}/rename", post(actions::rename_category))
        .route("/ui/categories/{id}/move", post(actions::move_category))
        .route("/ui/categories/{id}/delete", post(actions::delete_category))
        .route("/ui/lines/{id}/rename", post(actions::rename_line))
        .route("/ui/lines/{id}/planned", post(actions::line_planned))
        .route("/ui/lines/{id}/debt", post(actions::line_debt))
        .route("/ui/lines/{id}/move", post(actions::move_line))
        .route("/ui/lines/{id}/delete", post(actions::delete_line))
        .route("/ui/splits/{group}/delete", post(actions::delete_split))
        .route("/ui/transactions/{id}", post(actions::update_transaction))
        .route("/ui/transactions/{id}/delete", post(actions::delete_transaction))
        .route("/sync", post(crate::sync::sync))
        .layer(middleware::from_fn(auth::require_datastar_header))
        .layer(middleware::from_fn_with_state(state, auth::require_session));

    Router::new()
        .route("/login", get(pages::login_page))
        .route("/register", get(pages::register_page))
        .route("/offline", get(pages::offline_page))
        .route("/ui/login", post(actions::login).layer(middleware::from_fn(auth::same_origin_or_datastar)))
        .route("/ui/register", post(actions::register).layer(middleware::from_fn(auth::same_origin_or_datastar)))
        .merge(protected)
}

// ----------------------------------------------------------------------
// Views
// ----------------------------------------------------------------------

/// Which screen a request is rendering; carried by forms as `view` so a
/// mutation can re-render the screen it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Months { archived: bool },
    Paycheck { month: Id, paycheck: Id },
    Overview { month: Id },
    Income { month: Id, welcome: bool },
    Transactions { month: Id },
    Reports { month: Id, q: ReportQuery },
    Settings,
}

impl View {
    #[must_use]
    pub fn encode(&self) -> String {
        match self {
            View::Months { archived } => format!("months:{}", u8::from(*archived)),
            View::Paycheck { month, paycheck } => format!("paycheck:{month}:{paycheck}"),
            View::Overview { month } => format!("overview:{month}"),
            View::Income { month, .. } => format!("income:{month}"),
            View::Transactions { month } => format!("transactions:{month}"),
            View::Reports { month, .. } => format!("reports:{month}"),
            View::Settings => "settings".into(),
        }
    }

    #[must_use]
    pub fn decode(s: &str) -> Option<View> {
        let mut it = s.split(':');
        let kind = it.next()?;
        let a = it.next().map(Id::new);
        let b = it.next().map(Id::new);
        Some(match kind {
            "months" => View::Months { archived: a.is_some_and(|x| x.as_str() == "1") },
            "paycheck" => View::Paycheck { month: a?, paycheck: b? },
            "overview" => View::Overview { month: a? },
            "income" => View::Income { month: a?, welcome: false },
            "transactions" => View::Transactions { month: a? },
            "reports" => View::Reports { month: a?, q: ReportQuery::default() },
            "settings" => View::Settings,
            _ => return None,
        })
    }

    #[must_use]
    pub fn month(&self) -> Option<&Id> {
        match self {
            View::Paycheck { month, .. }
            | View::Overview { month }
            | View::Income { month, .. }
            | View::Transactions { month }
            | View::Reports { month, .. } => Some(month),
            View::Months { .. } | View::Settings => None,
        }
    }

    #[must_use]
    pub fn url(&self) -> String {
        match self {
            View::Months { archived } => {
                if *archived {
                    "/months?archived=1".into()
                } else {
                    "/months".into()
                }
            }
            View::Paycheck { month, paycheck } => format!("/months/{month}/paychecks/{paycheck}"),
            View::Overview { month } => format!("/months/{month}/overview"),
            View::Income { month, welcome } => {
                if *welcome {
                    format!("/months/{month}/income?welcome=1")
                } else {
                    format!("/months/{month}/income")
                }
            }
            View::Transactions { month } => format!("/months/{month}/transactions"),
            View::Reports { month, q } => format!("/months/{month}/reports{}", q.query_string()),
            View::Settings => "/settings".into(),
        }
    }

    #[must_use]
    pub fn content_url(&self) -> String {
        match self {
            View::Months { archived } => format!("/months/content{}", if *archived { "?archived=1" } else { "" }),
            View::Income { month, welcome } => format!("/months/{month}/income/content{}", if *welcome { "?welcome=1" } else { "" }),
            View::Reports { month, q } => format!("/months/{month}/reports/content{}", q.query_string()),
            other => format!("{}/content", other.url()),
        }
    }
}

/// Reports page state, carried in the URL so it can be bookmarked.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ReportQuery {
    #[serde(default)]
    pub tab: Option<String>,
    /// Trend window in months (3, 6 or 12).
    #[serde(default)]
    pub n: Option<usize>,
    /// Trend rows: "category" (default) or "line".
    #[serde(default)]
    pub level: Option<String>,
    /// Trend values: "spent" (default) or "planned".
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

impl ReportQuery {
    #[must_use]
    pub fn tab(&self) -> &str {
        match self.tab.as_deref() {
            Some(t @ ("trends" | "payees" | "export")) => t,
            _ => "summary",
        }
    }

    #[must_use]
    pub fn months(&self) -> usize {
        match self.n {
            Some(3) => 3,
            Some(12) => 12,
            _ => 6,
        }
    }

    #[must_use]
    pub fn lines(&self) -> bool {
        self.level.as_deref() == Some("line")
    }

    #[must_use]
    pub fn planned(&self) -> bool {
        self.metric.as_deref() == Some("planned")
    }

    /// Date range, defaulting to the month.
    #[must_use]
    pub fn range(&self, ym: NaiveDate) -> (NaiveDate, NaiveDate) {
        let parse = |s: &Option<String>| s.as_deref().and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
        let from = parse(&self.from).unwrap_or(ym);
        let to = parse(&self.to).unwrap_or_else(|| paycheckzero_core::recurrence::last_of_month(ym));
        if to < from { (to, from) } else { (from, to) }
    }

    #[must_use]
    pub fn query_string(&self) -> String {
        let mut parts = Vec::new();
        if self.tab() != "summary" {
            parts.push(format!("tab={}", self.tab()));
        }
        if let Some(n) = self.n {
            parts.push(format!("n={n}"));
        }
        for (k, v) in [("level", &self.level), ("metric", &self.metric), ("from", &self.from), ("to", &self.to)] {
            if let Some(v) = v {
                let clean: String = v.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
                parts.push(format!("{k}={clean}"));
            }
        }
        if parts.is_empty() { String::new() } else { format!("?{}", parts.join("&")) }
    }
}

/// Per-request rendering context.
pub struct Ctx {
    pub user: UserRecord,
    pub today: NaiveDate,
    /// Category names the user has collapsed (remembered per session, §14.7).
    pub collapsed: Vec<String>,
    /// Paycheck view: show only lines this paycheck funds.
    pub only_funded: bool,
}

impl Ctx {
    #[must_use]
    pub fn money(&self, c: Cents) -> String {
        money::format(c, &self.user.currency)
    }

    #[must_use]
    pub fn is_open(&self, category: &str) -> bool {
        !self.collapsed.iter().any(|c| c == category)
    }
}

pub const COLLAPSE_COOKIE: &str = "pz_collapsed";

#[must_use]
pub fn collapsed_from(headers: &axum::http::HeaderMap) -> Vec<String> {
    auth::cookie_value(headers, COLLAPSE_COOKIE)
        .map(|v| {
            v.split('|')
                .filter(|s| !s.is_empty())
                .map(percent_decode)
                .collect()
        })
        .unwrap_or_default()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ----------------------------------------------------------------------
// Shared components
// ----------------------------------------------------------------------

#[must_use]
pub fn month_label(ym: NaiveDate) -> String {
    ym.format("%B %Y").to_string()
}

#[must_use]
pub fn short_date(d: NaiveDate) -> String {
    d.format("%b %-d").to_string()
}

/// Hidden `view` field so the server can re-render the originating screen.
#[must_use]
pub fn view_input(view: &View) -> Markup {
    html! { input type="hidden" name="view" value=(view.encode()); }
}

/// Datastar attribute value posting the enclosing form.
#[must_use]
pub fn post_form(url: &str) -> String {
    format!("@post('{url}', {{contentType: 'form'}})")
}

/// Same, but only after the client-side money guard accepts the input.
#[must_use]
pub fn post_form_guarded(url: &str) -> String {
    format!("pz.guard(el) && @post('{url}', {{contentType: 'form'}})")
}

#[must_use]
pub fn skeleton() -> Markup {
    html! {
        div class="skeleton" aria-hidden="true" {
            div class="sk sk-title" {}
            div class="sk sk-hero" {}
            div class="sk sk-row" {}
            div class="sk sk-row" {}
            div class="sk sk-row" {}
        }
        p class="visually-hidden" { (t("common.loading")) }
    }
}

#[must_use]
pub fn empty_state(title: &str, body: &str, action: Option<Markup>) -> Markup {
    html! {
        div class="empty" {
            h2 { (title) }
            p { (body) }
            @if let Some(a) = action { (a) }
        }
    }
}

pub enum ToastKind {
    Success,
    Warning,
    Error,
}

/// A toast; errors carry collapsible technical details (spec §13.3).
#[must_use]
pub fn toast(kind: ToastKind, message: &str, technical: Option<&str>) -> Markup {
    let (class, role) = match kind {
        ToastKind::Success => ("toast success", "status"),
        ToastKind::Warning => ("toast warning", "status"),
        ToastKind::Error => ("toast error", "alert"),
    };
    let auto = matches!(kind, ToastKind::Success);
    html! {
        div class=(class) role=(role) data-toast
            data-init=[auto.then_some("setTimeout(() => el.remove(), 5000)")] {
            p class="toast-msg" { (message) }
            @if let Some(tech) = technical {
                details class="tech" {
                    summary { (t("common.technical_details")) }
                    code { (tech) }
                }
            }
            button type="button" class="toast-close" aria-label=(t("common.dismiss")) data-on:click="el.closest('[data-toast]').remove()" { "×" }
        }
    }
}

#[must_use]
pub fn money_input(name: &str, value: Option<Cents>, label: &str, max_cents: Option<i64>) -> Markup {
    html! {
        input type="text" inputmode="decimal" autocomplete="off" class="money" name=(name)
            value=[value.map(money::plain)] aria-label=(label)
            data-max-cents=[max_cents] placeholder="0.00";
    }
}

/// Inline SVG icons (decorative; always paired with visible or aria text).
#[must_use]
pub fn icon(name: &str) -> Markup {
    let path = match name {
        "paychecks" => "M3 6h18v12H3z M3 10h18 M7 15h4",
        "overview" => "M4 5h16 M4 12h16 M4 19h10",
        "income" => "M12 19V5 M6 11l6-6 6 6",
        "transactions" => "M4 7h13l-3-3 M20 17H7l3 3",
        "reports" => "M5 20V10 M12 20V4 M19 20v-7",
        "trash" => "M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13",
        "up" => "M12 19V5 M6 11l6-6 6 6",
        "down" => "M12 5v14 M6 13l6 6 6-6",
        "grip" => "M9 6h.01 M15 6h.01 M9 12h.01 M15 12h.01 M9 18h.01 M15 18h.01",
        "edit" => "M4 20h4L19 9l-4-4L4 16z M13 7l4 4",
        "lock" => "M6 11h12v9H6z M8 11V8a4 4 0 0 1 8 0v3",
        "check" => "M5 12l5 5 9-10",
        "plus" => "M12 5v14 M5 12h14",
        "alert" => "M12 4l9 16H3z M12 10v4 M12 17h.01",
        _ => "",
    };
    let width = if name == "grip" { "3" } else { "2" };
    html! {
        svg class=(format!("icon icon-{name}")) viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor"
            stroke-width=(width) stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false" {
            path d=(path) {}
        }
    }
}

struct NavTab {
    key: &'static str,
    label: String,
    short: String,
    href: String,
}

/// Section navigation within a month: tabs on wide screens, a bottom tab
/// bar on phones.
fn month_nav(month: &Id, active: &str, default_paycheck: Option<&Id>) -> Markup {
    let tabs = [
        NavTab {
            key: "paycheck",
            label: t("nav.paychecks"),
            short: t("nav.paychecks_short"),
            href: default_paycheck.map_or_else(|| format!("/months/{month}"), |p| format!("/months/{month}/paychecks/{p}")),
        },
        NavTab { key: "overview", label: t("nav.overview"), short: t("nav.overview_short"), href: format!("/months/{month}/overview") },
        NavTab { key: "income", label: t("nav.income"), short: t("nav.income_short"), href: format!("/months/{month}/income") },
        NavTab { key: "transactions", label: t("nav.transactions"), short: t("nav.transactions_short"), href: format!("/months/{month}/transactions") },
        NavTab { key: "reports", label: t("nav.reports"), short: t("nav.reports_short"), href: format!("/months/{month}/reports") },
    ];
    let icon_of = |k: &str| icon(if k == "paycheck" { "paychecks" } else { k });
    html! {
        nav class="tabs" aria-label=(t("nav.sections")) {
            @for tab in &tabs {
                a href=(tab.href) class=(if tab.key == active { "tab active" } else { "tab" })
                    aria-current=[(tab.key == active).then_some("page")] { (tab.label) }
            }
        }
        nav class="bottom-tabs" aria-label=(t("nav.sections_mobile")) {
            @for tab in &tabs {
                a href=(tab.href) class=(if tab.key == active { "btab active" } else { "btab" })
                    aria-current=[(tab.key == active).then_some("page")] aria-label=(tab.label) {
                    (icon_of(tab.key))
                    span { (tab.short) }
                }
            }
        }
    }
}

/// Header navigation data for month-scoped pages.
pub struct MonthHeader {
    pub id: Id,
    pub year_month: NaiveDate,
    pub prev: Option<Id>,
    pub next: Option<Id>,
    pub default_paycheck: Option<Id>,
}

fn month_switcher(h: &MonthHeader, active: &str) -> Markup {
    let prev_ym = paycheckzero_core::report::previous_month(h.year_month);
    let next_ym = paycheckzero_core::recurrence::last_of_month(h.year_month).succ_opt().unwrap_or(h.year_month);
    let target = |id: &Option<Id>, ym: NaiveDate| match id {
        Some(id) => format!("/months/{id}"),
        None => format!("/months?new={}", ym.format("%Y-%m")),
    };
    html! {
        div class="month-switcher" {
            a class="icon-btn" href=(target(&h.prev, prev_ym)) aria-label=(tf("nav.prev_month", &[("month", &month_label(prev_ym))])) { "‹" }
            a class="month-name" href="/months" aria-label=(tf("nav.all_months_current", &[("month", &month_label(h.year_month))])) { (month_label(h.year_month)) }
            a class="icon-btn" href=(target(&h.next, next_ym)) aria-label=(tf("nav.next_month", &[("month", &month_label(next_ym))])) { "›" }
        }
        (month_nav(&h.id, active, h.default_paycheck.as_ref()))
    }
}

/// The page shell: header, offline/sync banners, skeleton content that
/// loads itself over SSE, and the toast region.
#[must_use]
pub fn layout(user: Option<&UserRecord>, title: &str, header: Option<(&MonthHeader, &str)>, view: Option<&View>, body: Option<Markup>) -> Markup {
    let currency = user.map_or("USD", |u| u.currency.as_str());
    html! {
        (DOCTYPE)
        html lang="en" data-currency=(currency) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover";
                meta name="theme-color" content="#0f766e";
                meta name="description" content=(t("app.tagline"));
                title { (title) " · " (t("app.name")) }
                link rel="manifest" href="/manifest.webmanifest";
                link rel="icon" href="/static/icon.svg" type="image/svg+xml";
                link rel="apple-touch-icon" href="/static/icon-192.png";
                link rel="stylesheet" href="/static/app.css";
                script id="pz-i18n" type="application/json" { (PreEscaped(client_bundle().replace("</", "<\\/"))) }
                script src="/static/app.js" {}
                script type="module" src="/static/datastar.js" {}
            }
            body {
                a class="skip-link" href="#content" { (t("common.skip")) }
                div id="offline-banner" class="banner offline" role="status" hidden { (t("offline.banner")) }
                div id="script-banner" class="banner offline" role="alert" hidden { (t("error.no_scripts")) }
                div id="sync-banner" class="banner sync" role="status" hidden {
                    span id="sync-text" {}
                    button type="button" class="link" id="sync-review" hidden { (t("sync.review")) }
                }
                header class="topbar" {
                    a class="brand" href="/" {
                        img src="/static/icon.svg" alt="" width="28" height="28";
                        span { (t("app.name")) }
                    }
                    @if user.is_some() {
                        a class="settings-link" href="/settings" aria-label=(t("nav.settings")) { (t("nav.settings")) }
                    }
                }
                @if let Some((h, active)) = header {
                    div class="subbar" { (month_switcher(h, active)) }
                }
                @match (view, body) {
                    (_, Some(b)) => { main id="content" tabindex="-1" { (b) } },
                    (Some(v), None) => {
                        main id="content" tabindex="-1" aria-busy="true" data-init=(format!("@get('{}')", v.content_url())) {
                            (skeleton())
                            div class="load-error" hidden {
                                h2 { (t("error.load_title")) }
                                p { (t("error.load_body")) }
                                button type="button" class="btn" data-retry { (t("common.retry")) }
                            }
                        }
                    },
                    (None, None) => { main id="content" {} },
                }
                div id="toasts" class="toasts" aria-live="polite" {}
                dialog id="conflict-dialog" aria-labelledby="conflict-title" {
                    h2 id="conflict-title" { (t("sync.conflict_title")) }
                    div id="conflict-list" {}
                    button type="button" class="btn" data-close-dialog { (t("common.close")) }
                }
            }
        }
    }
}

/// Wraps rendered content in the `#content` element for an SSE morph.
#[must_use]
pub fn content(inner: Markup) -> Markup {
    html! { main id="content" tabindex="-1" aria-busy="false" { (inner) } }
}
