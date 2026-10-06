//! The three everyday screens, designed mobile-first: Plan (one paycheck),
//! Budget (the whole month) and Spending (transactions).
//!
//! Each screen shows only what matters at a glance: a headline number, one
//! obvious next step, and calm rows (name, progress, what's left). Amounts
//! are edited in place; everything else lives one tap away in a sheet.

use super::pages::{over_badge, overspent_lines, variance_panel};
use super::*;
use paycheckzero_core::*;

fn pct(part: Cents, whole: Cents) -> i64 {
    if whole.is_positive() {
        (part.get().saturating_mul(100) / whole.get()).clamp(0, 100)
    } else {
        0
    }
}

fn meter(part: Cents, whole: Cents, over: bool, label: &str) -> Markup {
    let p = pct(part, whole);
    html! {
        span class=(if over { "meter over" } else { "meter" }) role="meter" aria-label=(label)
            aria-valuemin="0" aria-valuemax="100" aria-valuenow=(p) {
            span style=(format!("width: {p}%")) {}
        }
    }
}

/// What a debt line still owes once this month's plan is paid, and roughly
/// how long the rest takes at this pace (before interest). `full` spells it
/// out (the line's sheet); otherwise it's short enough for a budget row.
pub(super) fn debt_outlook(c: &Ctx, l: &LineView, full: bool) -> Markup {
    html! {
        @match l.current_balance {
            None => { span class="row-debt muted" data-debt="unknown" { (t("debt.no_balance")) } },
            Some(b) if b.is_zero() => { span class="row-debt" data-debt="paid" { (icon("check")) " " (t("debt.paid_off")) } },
            Some(b) => {
                span class="row-debt" data-debt="owed" {
                    strong data-col="owed" { (c.money(b)) } " " (t("debt.owed_word"))
                    @if l.planned.is_positive() {
                        " · " (tf("debt.after_month", &[("amount", &c.money((b - l.planned).max(Cents::ZERO)))]))
                        @let months = (b.get() + l.planned.get() - 1) / l.planned.get();
                        @if months > 1 { " · " (tf(if full { "debt.payoff_months_full" } else { "debt.payoff_months" }, &[("n", &months.to_string())])) }
                    } @else {
                        " · " (t("debt.nothing_planned"))
                    }
                }
            },
        }
    }
}

/// "Spent $85.20 of $600.00 · $514.80 left" (Planned / Spent / Remaining).
fn line_meta(c: &Ctx, l: &LineView) -> Markup {
    html! {
        span class="row-meta" {
            @if l.planned.is_zero() && l.spent.is_zero() {
                (t("row.not_planned"))
                span class="visually-hidden" { span data-col="planned" { (c.money(l.planned)) } span data-col="spent" { (c.money(l.spent)) } span data-col="remaining" { (c.money(l.remaining)) } }
            } @else {
                span class=(if l.remaining.is_negative() { "neg strong" } else { "strong" }) data-col="remaining" { (c.money(l.remaining.abs())) }
                " " (if l.remaining.is_negative() { t("row.over") } else { t("row.left") }) " " (t("row.of")) " "
                span data-col="planned" { (c.money(l.planned)) }
                @if l.spent.is_positive() {
                    " · " span data-col="spent" { (c.money(l.spent)) } " " (t("cat.spent"))
                } @else {
                    span class="visually-hidden" { " · " span data-col="spent" { (c.money(l.spent)) } }
                }
            }
        }
    }
}

/// A budget line: tap the name for details, edit the amount in place.
fn line_row(c: &Ctx, l: &LineView, sheet_url: &str, amount: Option<Markup>, fallback_amount: Cents, dim: bool) -> Markup {
    let over = l.spent > l.planned;
    html! {
        li class={ "row line" @if over { " over" } @if dim { " unfunded" } } id=(format!("line-{}", l.id)) data-line=(l.name) {
            button type="button" class="row-main" data-on:click=(open_sheet(sheet_url))
                aria-label=(tf("row.details", &[("name", &l.name)])) {
                span class="row-name" { (l.name) (over_badge(c, l)) }
                (line_meta(c, l))
                @if l.is_debt { (debt_outlook(c, l, false)) }
                @if l.spent.is_positive() { (meter(l.spent, l.planned, over, &tf("row.meter", &[("name", &l.name)]))) }
            }
            div class="row-amount" {
                @match amount {
                    Some(form) => { (form) },
                    None => { span class="num" { (c.money(fallback_amount)) } },
                }
            }
        }
    }
}

fn amount_form(c: &Ctx, url: &str, view: &View, value: Cents, label: &str, max: Option<i64>) -> Markup {
    html! {
        form data-on:submit__prevent=(post_form_guarded(url)) {
            (view_input(view))
            (money_input(c, "amount", Some(value), label, max))
            span class="field-error" aria-live="polite" {}
        }
    }
}

fn add_line_row(m: &Month, view: &View, cat: &CategoryView, pid: Option<&Id>, max: Option<i64>) -> Markup {
    html! {
        form class="add-line" id=(format!("add-line-{}", cat.id)) data-clear data-on:submit__prevent=(post_form_guarded(&format!("/ui/months/{}/lines", m.id))) {
            (view_input(view))
            input type="hidden" name="category_id" value=(cat.id);
            @if let Some(p) = pid { input type="hidden" name="paycheck_id" value=(p); }
            span class="add-icon" aria-hidden="true" { (icon("plus")) }
            input type="text" name="name" required maxlength="100" placeholder=(t("line.add_placeholder")) aria-label=(tf("line.add_label", &[("category", &cat.name)]));
            @if pid.is_some() {
                span class="add-extra" {
                    @let debt = cat.kind == CategoryKind::Debt;
                    input type="text" inputmode="decimal" class="money" name="amount" autocomplete="off" required data-max-cents=[max]
                        placeholder=(if debt { t("line.add_payment_placeholder") } else { "0.00".into() })
                        aria-label=(tf(if debt { "line.add_payment_label" } else { "line.add_from_this_label" }, &[("category", &cat.name)]));
                }
            }
            button type="submit" class="btn small" { (t("line.add")) }
        }
    }
}

/// Categories with no lines yet, as one row of "+ Name" chips that open the
/// new-line sheet for that category.
fn empty_cats(m: &Month, empty: &[&CategoryView], query: &str) -> Markup {
    html! {
        @if !empty.is_empty() {
            div class="empty-cats" {
                p class="muted small" { (t("budget.empty_cats")) }
                div class="chip-row" {
                    @for cat in empty {
                        button type="button" class="chip-toggle" data-category-empty=(cat.name)
                            data-on:click=(open_sheet(&format!("/ui/sheet/new-line/{}?{query}&cat={}", m.id, cat.id))) {
                            (icon("plus")) " " (cat.name)
                        }
                    }
                }
            }
        }
    }
}

/// Compact alerts shared by Plan and Budget: month zero status, overspending,
/// archived/locked state and the variance flow.
pub(super) fn month_alerts(c: &Ctx, m: &Month, archived: bool, view: &View, offer_lock: bool) -> Markup {
    let diff = m.zero_difference();
    let overs = overspent_lines(m);
    html! {
        div class="alerts" {
            @if archived {
                p class="alert warn" { (icon("alert")) " " (t("status.archived")) }
            }
            @if m.is_locked() && !m.reassigning {
                p class="alert neutral pill locked" { (icon("lock")) " " (t("status.locked")) }
            }
            div id="zero-status" class={ "alert " @if m.paychecks.is_empty() { "todo" } @else if diff.is_zero() && m.has_variance() { "caution" } @else if diff.is_zero() { "ok" } @else { "todo" } } aria-live="polite" {
                span class="alert-text" {
                    @if m.paychecks.is_empty() { (t("status.no_income")) }
                    @else if diff.is_zero() && m.has_variance() { (t("status.zero_with_variance")) }
                    @else if diff.is_zero() { (icon("check")) " " (t("status.zero")) }
                    @else if diff.is_positive() { (tf("status.left_to_assign", &[("amount", &c.money(diff))])) }
                    @else { (tf("status.over_assigned", &[("amount", &c.money(diff.abs()))])) }
                }
                @if !m.is_locked() && !archived && !m.paychecks.is_empty() && (offer_lock || diff.is_zero()) {
                    form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/months/{}/lock", m.id))) {
                        (view_input(view))
                        button type="submit" class=(if diff.is_zero() { "btn small primary" } else { "btn small" }) aria-describedby="zero-status" { (icon("lock")) " " (t("status.lock")) }
                    }
                }
            }
            @if !overs.is_empty() {
                div class="alert danger" id="overspent-banner" role="status" {
                    span class="alert-text" {
                        (icon("alert")) " "
                        strong { (tf("over.banner", &[("n", &overs.len().to_string())])) } " "
                        @for (i, (name, amt)) in overs.iter().enumerate() {
                            @if i > 0 { ", " }
                            (tf("over.item", &[("name", name), ("amount", &c.money(*amt))]))
                        }
                    }
                    @if !matches!(view, View::Overview { .. }) {
                        a class="alert-link" href=(format!("/months/{}/overview", m.id)) { (t("over.review")) }
                    }
                }
            }
            @if (m.has_variance() || m.reassigning) && !archived {
                (variance_panel(c, m, view))
            }
        }
    }
}

/// Plan's alerts are about this paycheck; month-wide status lives on
/// Budget. The month's zero status only shows here while re-assigning a
/// variance (it holds the re-lock button).
fn paycheck_alerts(c: &Ctx, m: &Month, archived: bool, view: &View) -> Markup {
    html! {
        @if archived || (m.is_locked() && !m.reassigning) || m.reassigning || m.has_variance() {
            (month_alerts(c, m, archived, view, false))
        }
    }
}

/// "3 transactions need a line" with a way to fix them.
fn needs_line_alert(c: &Ctx, m: &Month) -> Markup {
    let needing: Vec<&Transaction> = m.transactions.iter().filter(|t| t.needs_line()).collect();
    let total: Cents = needing.iter().map(|t| t.amount.abs()).sum();
    html! {
        @if !needing.is_empty() {
            div class="alert caution" id="needs-line-alert" role="status" {
                span class="alert-text" {
                    (icon("alert")) " "
                    (tf("tx.needs_line_alert", &[("n", &needing.len().to_string()), ("amount", &c.money(total))]))
                }
                a class="alert-link" href=(format!("/months/{}/transactions?show=needs-line", m.id)) { (t("tx.needs_line_review")) }
            }
        }
    }
}

/// The latest spending tagged to this paycheck, so the plan and what
/// actually happened sit side by side.
fn recent_from_paycheck(c: &Ctx, m: &Month, wallet: &Wallet, pid: &Id, view: &View) -> Markup {
    let mut txs: Vec<&Transaction> = m.transactions.iter().filter(|t| t.paycheck_id.as_ref() == Some(pid)).collect();
    txs.sort_by_key(|t| std::cmp::Reverse(t.date));
    html! {
        section class="card recent" aria-labelledby="recent-h" {
            div class="section-head" {
                h2 id="recent-h" class="h3" { (t("plan.recent")) }
                a class="small" href=(format!("/months/{}/transactions", m.id)) { (t("plan.recent_all")) }
            }
            @if txs.is_empty() {
                p class="muted small" { (t("plan.recent_empty")) }
            } @else {
                (super::accounts::recent_list(c, m, wallet, &txs.iter().take(4).copied().collect::<Vec<_>>(), view, None))
            }
        }
    }
}

// ----------------------------------------------------------------------
// Plan: one paycheck
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
                    span class="chip-amt" { (c.money(v.planned_amount)) }
                    span class="chip-state" {
                        @if v.status == PaycheckStatus::Skipped { (t("paycheck.skipped")) }
                        @else if v.fully_allocated { (icon("check")) " " (t("paycheck.fully_assigned")) }
                        @else { (tf("paycheck.left", &[("amount", &c.money(v.unallocated))])) }
                    }
                    span class="visually-hidden" { " · " (v.income_line_name) }
                }
            }
            a class="chip add" href=(format!("/months/{}/income", m.id)) { (icon("plus")) " " (t("paycheck.add")) }
        }
    }
}

pub fn render_paycheck(c: &Ctx, m: &Month, archived: bool, pid: &Id, wallet: &Wallet) -> Markup {
    let Some(p) = m.paycheck(pid) else { return html! {} };
    let v = m.paycheck_view(p);
    let view = View::Paycheck { month: m.id.clone(), paycheck: pid.clone() };
    let enc = view.encode();
    let editable = m.allocations_editable() && !archived && p.status != PaycheckStatus::Skipped;
    let structure = !m.is_locked() && !archived;
    // This paycheck on its own: only the lines it funds, with its share,
    // its spending and what's left of it. Budget shows the whole month.
    let shown = m.funding_views(pid);
    let has_lines = !shown.is_empty();
    html! {
        h1 class="visually-hidden" { (tf("paycheck.heading", &[("date", &short_date(p.date)), ("name", &v.income_line_name)])) }
        div class="split-layout has-extras" {
        div class="side" {
        (paycheck_strip(c, m, pid))

        section class="hero" aria-labelledby="sts-label" {
            div class="hero-top" {
                span class="hero-label" { (tf("paycheck.hero_sub", &[("date", &short_date(p.date)), ("name", &v.income_line_name)])) }
                button type="button" class="icon-btn on-dark" aria-label=(t("paycheck.details"))
                    data-on:click=(open_sheet(&format!("/ui/sheet/paycheck/{pid}?view={enc}"))) { (icon("more")) }
            }
            p id="sts-label" class="hero-kicker" { (t("paycheck.safe_to_spend")) }
            p id="safe-to-spend" class=(if v.safe_to_spend.is_negative() { "hero-amount neg" } else { "hero-amount" }) aria-live="polite" {
                (c.money(v.safe_to_spend))
            }
            @if v.safe_to_spend.is_negative() {
                p class="hero-warn" role="status" { (icon("alert")) " " (tf("over.sts", &[("amount", &c.money(v.safe_to_spend.abs()))])) }
            }
            (meter(v.allocated, v.planned_amount, false, &t("paycheck.assigned_meter")))
            p class="hero-progress" {
                span data-stat="assigned" { (c.money(v.allocated)) } " " (t("row.of")) " "
                span data-stat="planned" { (c.money(v.planned_amount)) } " " (t("paycheck.assigned_word"))
                span class="visually-hidden" { " · " span data-stat="unassigned" { (c.money(v.unallocated)) } " " (t("paycheck.unassigned")) }
            }
            @if v.status == PaycheckStatus::Skipped {
                p class="hero-note" { (t("paycheck.skipped_notice")) }
            } @else if v.unallocated.is_positive() {
                div class="hero-cta" {
                    p id="unassigned-nudge" class="hero-note" role="status" { (tf("paycheck.nudge", &[("amount", &c.money(v.unallocated))])) }
                    @if editable {
                        button type="button" class="btn cta" data-on:click=(open_sheet(&format!("/ui/sheet/assign/{pid}?view={enc}"))) {
                            (tf("assign.button", &[("amount", &c.money(v.unallocated))]))
                        }
                    }
                }
            } @else {
                p class="hero-note ok" id="unassigned-nudge" {
                    (icon("check")) " " (t("paycheck.all_assigned"))
                    @if editable { " " span class="hero-hint" { (t("paycheck.all_assigned_hint")) } }
                }
            }
            dl class="hero-stats" {
                div { dt { (t("paycheck.budget_left")) } dd data-stat="budget-left" { (c.money(v.budget_left)) } }
                div { dt { (t("paycheck.tagged")) } dd data-stat="tagged" { (c.money(v.tagged_expense)) } }
            }
        }

        (paycheck_alerts(c, m, archived, &view))
        }
        div class="main-col" {
        section class="funding" id="funding" aria-labelledby="funding-h" {
            div class="section-head" {
                h2 id="funding-h" { (t("paycheck.funds")) }
                a class="small" href=(format!("/months/{}/overview", m.id)) { (t("plan.whole_month")) }
            }
            @if !has_lines {
                div class="empty soft" {
                    h3 { (t("plan.empty_title")) }
                    p { (t("plan.empty_body")) }
                }
            }
            @for cat in &shown {
                div class="cat-wrap" {
                    details class="category" data-category=(cat.name) data-cat-id=(cat.id) open[c.is_open(&cat.name)] {
                        summary {
                            span class="cat-name" { (cat.name)
                                @if cat.lines.iter().any(|l| l.spent > l.planned) {
                                    span class="cat-over" title=(t("over.category")) { (icon("alert")) span class="visually-hidden" { (t("over.category")) } }
                                }
                            }
                            span class="cat-sum" {
                                span class="num" data-col="this" { (c.money(cat.this_paycheck)) }
                                span class="cat-sub" {
                                    span class=(if cat.remaining.is_negative() { "neg" } else { "" }) data-col="remaining" { (c.money(cat.remaining.abs())) }
                                    " " (if cat.remaining.is_negative() { t("row.over") } else { t("row.left") })
                                    @if cat.spent.is_positive() { " · " span data-col="spent" { (c.money(cat.spent)) } " " (t("cat.spent")) }
                                }
                            }
                        }
                        ul class="rows" {
                            @for l in &cat.lines {
                                @let form = editable.then(|| amount_form(c,
                                    &format!("/ui/paychecks/{}/lines/{}", pid, l.id), &view, l.this_paycheck,
                                    &tf("line.planned_label", &[("name", &l.name)]), Some((l.this_paycheck + v.unallocated).get())));
                                (line_row(c, l, &format!("/ui/sheet/line/{}?view={enc}&pid={pid}", l.id), form, l.this_paycheck, false))
                            }
                        }
                        @if structure {
                            (add_line_row(m, &view, cat, Some(pid), Some(v.unallocated.get())))
                        }
                    }
                }
            }
            @if structure {
                button type="button" class="btn ghost wide-btn" data-on:click=(open_sheet(&format!("/ui/sheet/new-line/{}?view={enc}&pid={pid}", m.id))) {
                    (icon("plus")) " " (t("plan.new_line"))
                }
            }
        }
        p class="page-links" {
            a href=(format!("/months/{}/income", m.id)) { (t("plan.manage_income")) }
        }
        }
        div class="extras" { (recent_from_paycheck(c, m, wallet, pid, &view)) }
        }
    }
}

// ----------------------------------------------------------------------
// Budget: the whole month
// ----------------------------------------------------------------------

pub fn render_overview(c: &Ctx, m: &Month, archived: bool, wallet: &Wallet, all: &[Month]) -> Markup {
    let view = View::Overview { month: m.id.clone() };
    let enc = view.encode();
    let alloc = m.allocations_editable() && !archived;
    let structure = !m.is_locked() && !archived;
    let cats = m.category_views();
    let (shown, empty): (Vec<&CategoryView>, Vec<&CategoryView>) = cats.iter().partition(|c| !c.lines.is_empty());
    let free: Cents = m.paychecks.iter().map(|p| m.paycheck_unallocated(&p.id)).sum();
    let income = m.total_planned_income();
    let planned = m.total_planned_expense();
    let spent: Cents = m.expense_lines.iter().map(|l| m.line_spent(&l.id)).sum();
    html! {
        div class="split-layout has-extras" {
        div class="side" {
        h1 { (t("budget.title")) span class="visually-hidden" { " · " (month_label(m.year_month)) } }
        section class="summary card" aria-label=(t("cards.label")) {
            div class="summary-stats" {
                div data-card="income" { span class="stat-label" { (t("budget.income")) } span class="stat-value" { (c.money(income)) } }
                div data-card="expenses" { span class="stat-label" { (t("budget.planned")) } span class="stat-value" { (c.money(planned)) } }
                div data-card="spent" { span class="stat-label" { (t("budget.spent")) } span class="stat-value" { (c.money(spent)) } }
            }
            (meter(spent, planned, spent > planned, &t("cards.spent_meter")))
            p class="summary-note" { (tf("budget.spent_of", &[("pct", &pct(spent, planned).to_string())])) }
        }
        (month_alerts(c, m, archived, &view, true))
        (needs_line_alert(c, m))
        }
        div class="main-col" {
        @if m.paychecks.is_empty() {
            (empty_state(&t("overview.no_income_title"), &t("overview.no_income_body"),
                Some(html! { a class="btn primary" href=(format!("/months/{}/income", m.id)) { (t("income.add")) } })))
        }
        section class="overview" aria-label=(t("overview.categories")) {
            @for cat in &shown {
                details class={ "category" @if structure { " has-more" } } data-category=(cat.name) data-cat-id=(cat.id) open[c.is_open(&cat.name)] {
                    summary {
                        span class="cat-name" { (cat.name)
                            @if cat.lines.iter().any(|l| l.spent > l.planned) {
                                span class="cat-over" title=(t("over.category")) { (icon("alert")) span class="visually-hidden" { (t("over.category")) } }
                            }
                        }
                        span class="cat-sum" {
                            span class="num" data-col="planned" { (c.money(cat.planned)) }
                            span class="cat-sub" {
                                span data-col="spent" { (c.money(cat.spent)) } " " (t("cat.spent")) " · "
                                span class=(if cat.remaining.is_negative() { "neg" } else { "" }) data-col="remaining" { (c.money(cat.remaining)) } " " (t("row.left"))
                            }
                            @if cat.kind == CategoryKind::Debt {
                                @let owed: Cents = cat.lines.iter().filter_map(|l| l.current_balance).sum();
                                @let after: Cents = cat.lines.iter().filter_map(|l| l.current_balance.map(|b| (b - l.planned).max(Cents::ZERO))).sum();
                                @if owed.is_positive() {
                                    span class="cat-sub cat-debt" data-col="cat-owed" { (tf("debt.cat_owed", &[("amount", &c.money(owed)), ("after", &c.money(after))])) }
                                }
                            }
                        }
                    }
                    @if structure {
                        button type="button" class="icon-btn cat-more" aria-label=(tf("category.edit", &[("name", &cat.name)]))
                            data-on:click=(open_sheet(&format!("/ui/sheet/category/{}?view={enc}", cat.id))) { (icon("more")) }
                    }
                    ul class="rows" {
                        @for l in &cat.lines {
                            @let form = alloc.then(|| amount_form(c,
                                &format!("/ui/lines/{}/planned", l.id), &view, l.planned,
                                &tf("line.total_planned_label", &[("name", &l.name)]), Some((l.planned + free).get())));
                            (line_row(c, l, &format!("/ui/sheet/line/{}?view={enc}", l.id), form, l.planned, false))
                        }
                    }
                    @if structure { (add_line_row(m, &view, cat, None, None)) }
                }
            }
            @if structure { (empty_cats(m, &empty, &format!("view={enc}"))) }
            @if structure {
                button type="button" class="btn ghost wide-btn" data-on:click=(open_sheet(&format!("/ui/sheet/new-line/{}?view={enc}", m.id))) {
                    (icon("plus")) " " (t("plan.new_line"))
                }
            }
        }
        @if structure {
            form class="add-category" id="add-category" data-clear data-on:submit__prevent=(post_form(&format!("/ui/months/{}/categories", m.id))) {
                (view_input(&view))
                label for="new-category" { (t("category.add_label")) }
                div class="inline-field" {
                    input id="new-category" type="text" name="name" required maxlength="100";
                    button type="submit" class="btn" { (t("category.add")) }
                }
            }
        }
        }
        // Goals and balances: beside the plan on wide screens, after it on phones.
        div class="extras" {
            (super::accounts::goals_section(c, m, wallet, all, &view))
            (super::accounts::accounts_card(c, m, wallet, all))
        }
        }
    }
}

// ----------------------------------------------------------------------
// Spending: transactions
// ----------------------------------------------------------------------

enum TxItem<'a> {
    Single(&'a Transaction),
    Split(Id),
}

pub fn render_transactions(c: &Ctx, m: &Month, archived: bool, filter: TxFilter, wallet: &Wallet) -> Markup {
    let view = View::Transactions { month: m.id.clone(), filter };
    let enc = view.encode();
    let needing: Vec<&Transaction> = m.transactions.iter().filter(|t| t.needs_line()).collect();
    let mut txs: Vec<&Transaction> = m.transactions.iter().filter(|t| filter == TxFilter::All || t.needs_line()).collect();
    txs.sort_by_key(|x| std::cmp::Reverse(x.date));
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
    let spent: Cents = m.transactions.iter().filter(|t| t.is_spending()).map(|t| t.amount.abs()).sum();
    let received: Cents = m.transactions.iter().filter(|t| t.amount.is_positive()).map(|t| t.amount).sum();
    // (show a day header?, first part, total, parts) per payment.
    let mut last_day: Option<NaiveDate> = None;
    let rows: Vec<(bool, &Transaction, Cents, Vec<&Transaction>)> = items
        .iter()
        .map(|it| {
            let (first, total, parts): (&Transaction, Cents, Vec<&Transaction>) = match it {
                TxItem::Single(x) => (*x, x.amount, vec![*x]),
                TxItem::Split(g) => {
                    let ps = m.split_parts(g);
                    (ps[0], ps.iter().map(|p| p.amount).sum(), ps)
                }
            };
            let show = last_day != Some(first.date);
            last_day = Some(first.date);
            (show, first, total, parts)
        })
        .collect();
    html! {
        div class="split-layout" {
        div class="side" {
        h1 { (t("spend.title")) span class="visually-hidden" { " · " (month_label(m.year_month)) } }
        section class="summary card slim" {
            div class="summary-stats two" {
                div { span class="stat-label" { (t("spend.out")) } span class="stat-value" { (c.money(spent)) } }
                div { span class="stat-label" { (t("spend.in")) } span class="stat-value pos" { (c.money(received)) } }
            }
        }
        @if m.is_locked() {
            p class="alert neutral" { (t("tx.locked_ok")) }
        }
        (super::pages::overspent_banner_pub(c, m, &view))
        }
        div class="main-col" {
        div class="filter-chips" role="group" aria-label=(t("tx.filter_label")) {
            a class=(if filter == TxFilter::All { "chip-toggle on" } else { "chip-toggle" }) href=(format!("/months/{}/transactions", m.id))
                aria-current=[(filter == TxFilter::All).then_some("page")] { (t("tx.filter_all")) }
            a class={ "chip-toggle" @if filter == TxFilter::NeedsLine { " on" } @if !needing.is_empty() { " attention" } }
                href=(format!("/months/{}/transactions?show=needs-line", m.id)) data-filter="needs-line"
                aria-current=[(filter == TxFilter::NeedsLine).then_some("page")] {
                (t("tx.filter_needs_line")) @if !needing.is_empty() { " " span class="count" { (needing.len()) } }
            }
        }
        div class="toolbar" {
            div class="search" {
                span class="search-icon" aria-hidden="true" { (icon("search")) }
                input type="search" id="tx-search" data-filter-list="tx-list" placeholder=(t("spend.search_placeholder")) aria-label=(t("spend.search"));
            }
        }
        section aria-labelledby="tx-list-h" {
            h2 id="tx-list-h" class="visually-hidden" { (t("tx.list")) }
            @if items.is_empty() && filter == TxFilter::NeedsLine {
                div class="empty soft" { h3 { (icon("check")) " " (t("tx.all_have_lines")) } p { (t("tx.all_have_lines_body")) } }
            } @else if items.is_empty() {
                (empty_state(&t("tx.empty_title"), &t("tx.empty_body"), (!archived).then(|| html! {
                    button type="button" class="btn primary" data-on:click=(open_sheet(&format!("/ui/sheet/tx/new/{}?view={enc}", m.id))) { (t("tx.add")) }
                })))
            } @else {
                ul class="tx-list card" id="tx-list" {
                    @for (show_day, first, total, parts) in &rows {
                        @let (first, total) = (*first, *total);
                        @if *show_day {
                            li class="tx-day" aria-hidden="true" { (first.date.format("%a, %b %-d").to_string()) }
                        }
                        @let payee = first.payee.clone().unwrap_or_else(|| if first.is_transfer() { t("transfer.title") } else { t("tx.no_payee") });
                        @let is_split = parts.len() > 1;
                        @let account = first.account_id.as_ref().and_then(|a| wallet.account(a)).map(|a| a.name.clone());
                        @let base = (!is_split && !first.is_transfer()).then(|| serde_json::json!({
                            "date": first.date, "amount": first.amount.get(), "payee": first.payee, "notes": first.notes,
                            "expense_line_id": first.expense_line_id, "paycheck_id": first.paycheck_id, "account_id": first.account_id,
                        }).to_string());
                        li class={ "tx" @if is_split { " split" } @if first.is_transfer() { " transfer" } @if first.needs_line() { " needs-line" } } data-tx=(first.id) data-split=[first.split_group.as_ref()] data-base=[base]
                            data-search=(format!("{} {}", payee, parts.iter().filter_map(|p| line_name(&p.expense_line_id)).collect::<Vec<_>>().join(" ")).to_lowercase()) {
                            button type="button" class="tx-open" disabled[archived]
                                aria-label=(tf("tx.edit_label", &[("payee", &payee), ("date", &short_date(first.date))]))
                                data-on:click=(open_sheet(&format!("/ui/sheet/tx/{}?view={enc}", first.id))) {
                                span class="tx-main" {
                                    span class="tx-payee" {
                                        @if first.is_transfer() { span class="tx-icon" { (icon("transfer")) } }
                                        (payee)
                                        @if first.needs_line() { " " span class="pill warn tiny" { (t("tx.needs_line")) } }
                                    }
                                    span class="tx-meta" {
                                        @if first.is_transfer() {
                                            (super::accounts::transfer_route(wallet, first))
                                            @if let Some(l) = line_name(&first.expense_line_id) { " · " (l) }
                                            @if let Some(n) = &first.notes { " · " (n) }
                                        } @else if is_split {
                                            strong { (t("split.meta")) } " · "
                                            @for (i, p) in parts.iter().enumerate() {
                                                @if i > 0 { " · " }
                                                (line_name(&p.expense_line_id).unwrap_or_else(|| t("tx.uncategorized"))) " " (c.money(p.amount.abs()))
                                                @if let Some(pc) = p.paycheck_id.as_ref().and_then(|pc| m.paycheck(pc)) { " (" (short_date(pc.date)) ")" }
                                            }
                                        } @else {
                                            @if !first.needs_line() { (line_name(&first.expense_line_id).unwrap_or_else(|| t("tx.uncategorized"))) }
                                            @else { (t("tx.tap_to_add_line")) }
                                            @if let Some(p) = first.paycheck_id.as_ref().and_then(|p| m.paycheck(p)) {
                                                " · " (tf("tx.from_paycheck", &[("date", &short_date(p.date))]))
                                            }
                                            @if let Some(n) = &first.notes { " · " (n) }
                                        }
                                        @if let (Some(a), false) = (&account, first.is_transfer()) { " · " span class="tx-acct" { (a) } }
                                    }
                                }
                                span class={ "tx-amt num" @if total.is_positive() { " pos" } @if first.is_transfer() { " neutral" } } {
                                    (c.money(if first.is_transfer() { total.abs() } else { total }))
                                }
                            }
                        }
                    }
                }
                p class="muted small empty-search" hidden data-empty-for="tx-list" { (t("spend.no_match")) }
            }
        }
        }
        }
    }
}

/// Used by the new-transaction sheet to preselect today's date inside the month.
#[must_use]
pub fn default_tx_date(c: &Ctx, m: &Month) -> NaiveDate {
    if recurrence::in_month(c.today, m.year_month) { c.today } else { m.year_month }
}
