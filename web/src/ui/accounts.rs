//! Accounts, credit cards, transfers and goals: the Accounts screen, the
//! cards they add to Budget, their sheets and their actions.
//!
//! These live across months (a checking account doesn't reset on the 1st),
//! so they are stored in the user's wallet rather than in a month. Balances
//! are computed from every month's transactions.

use super::actions::{done, failed, field, money_field, month_action, opt_id, opt_money_field, overs, overspend_toasts, view_of, F};
use super::pages::{account_options, ctx, line_options, paycheck_options};
use super::plan::default_tx_date;
use super::*;
use crate::auth::AuthUser;
use crate::error::{AppError, AppResult};
use crate::sse::Sse;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::{Extension, Form};
use chrono::Datelike;
use paycheckzero_core::*;
use paycheckzero_storage::{Owner, UserRecord};
use serde::Deserialize;
use std::collections::HashMap;

/// Days after which a balance is worth checking against the bank again.
const RECONCILE_AFTER_DAYS: i64 = 30;

fn kind_label(k: AccountKind) -> String {
    t(&format!("accounts.kind_{}", k.as_str()))
}

fn bar(p: i64, class: &str, label: &str) -> Markup {
    html! {
        span class=(format!("meter {class}")) role="meter" aria-label=(label) aria-valuemin="0" aria-valuemax="100" aria-valuenow=(p) {
            span style=(format!("width: {p}%")) {}
        }
    }
}

/// Totals for the summary.
struct Totals {
    cash: Cents,
    invested: Cents,
    owed: Cents,
}

impl Totals {
    fn net(&self) -> Cents {
        self.cash + self.invested - self.owed
    }
}

fn totals(w: &Wallet, all: &[Month]) -> Totals {
    let mut t = Totals { cash: Cents::ZERO, invested: Cents::ZERO, owed: Cents::ZERO };
    for a in w.accounts_sorted() {
        match a.kind.group() {
            AccountGroup::Card => t.owed += w.owed(&a.id, all),
            AccountGroup::Invested => t.invested += w.balance(&a.id, all),
            AccountGroup::Cash => t.cash += w.balance(&a.id, all),
        }
    }
    t
}

fn signed(c: &Ctx, v: Cents) -> String {
    if v.is_positive() { format!("+{}", c.money(v)) } else { c.money(v) }
}

fn reconciled_meta(c: &Ctx, a: &Account) -> Markup {
    html! {
        @match a.reconciled_on {
            Some(d) if (c.today - d).num_days() > RECONCILE_AFTER_DAYS => {
                span class="warn-text" { (tf("accounts.reconcile_due", &[("date", &short_date(d))])) }
            }
            Some(d) => { (tf("accounts.checked_on", &[("date", &short_date(d))])) }
            None => { (t("accounts.never_checked")) }
        }
    }
}

fn account_row(c: &Ctx, m: &Month, w: &Wallet, all: &[Month], a: &Account, enc: &str) -> Markup {
    let open = open_sheet(&format!("/ui/sheet/account/{}?view={enc}", a.id));
    html! {
        @if a.kind.is_card() {
            @let s = w.card_summary(&a.id, all, m);
            li class="row account card-account" data-account=(a.name) {
                button type="button" class="row-main" data-on:click=(open) aria-label=(tf("accounts.details", &[("name", &a.name)])) {
                    span class="row-name" { span class="acct-icon" { (icon("card")) } (a.name) }
                    span class="row-meta" {
                        @if s.ready_to_pay.is_positive() {
                            span class="strong" data-col="ready" { (c.money(s.ready_to_pay)) } " " (t("cards.ready_short"))
                            @if s.carried.is_positive() { " · " }
                        }
                        @if s.carried.is_positive() {
                            span data-col="carried" { (c.money(s.carried)) } " " (t("cards.carried_short"))
                        }
                        @if s.owed.is_zero() { (t("cards.paid_off")) }
                        @if s.unbudgeted_spending.is_positive() {
                            " · " span class="warn-text" { (tf("cards.needs_line_short", &[("amount", &c.money(s.unbudgeted_spending))])) }
                        }
                    }
                    @if let Some(u) = s.utilization {
                        (bar(u.min(100), if u >= 30 { "over" } else { "" }, &tf("cards.utilization_label", &[("pct", &u.to_string())])))
                    }
                }
                div class="row-amount" { span class="num neg" data-col="owed" { (c.money(s.owed)) } }
            }
        } @else if a.kind.is_invested() {
            @let s = w.invested_summary(&a.id, all, m.year_month.year());
            li class="row account invested-account" data-account=(a.name) {
                button type="button" class="row-main" data-on:click=(open) aria-label=(tf("accounts.details", &[("name", &a.name)])) {
                    span class="row-name" { span class="acct-icon" { (icon("growth")) } (a.name) }
                    span class="row-meta" {
                        (kind_label(a.kind))
                        @if s.contributed.is_positive() { " · " (tf("invest.contributed_short", &[("amount", &c.money(s.contributed))])) }
                        @if !s.growth.is_zero() {
                            " · " span class=(if s.growth.is_negative() { "neg" } else { "pos" }) data-col="growth" { (tf("invest.growth_short", &[("amount", &signed(c, s.growth))])) }
                        }
                    }
                }
                div class="row-amount" { span class="num" data-col="balance" { (c.money(s.balance)) } }
            }
        } @else {
            @let bal = w.balance(&a.id, all);
            li class="row account" data-account=(a.name) {
                button type="button" class="row-main" data-on:click=(open) aria-label=(tf("accounts.details", &[("name", &a.name)])) {
                    span class="row-name" { span class="acct-icon" { (icon(if a.kind == AccountKind::Cash { "wallet" } else { "bank" })) } (a.name) }
                    span class="row-meta" {
                        (kind_label(a.kind)) " · "
                        @if a.link_id.is_some() { span class="synced" { (icon("check")) " " (tf("bank.synced_on", &[("date", &a.reconciled_on.map_or_else(|| "—".into(), short_date))])) } }
                        @else { (reconciled_meta(c, a)) }
                    }
                }
                div class="row-amount" { span class=(if bal.is_negative() { "num neg" } else { "num" }) data-col="balance" { (c.money(bal)) } }
            }
        }
    }
}

/// The Accounts screen.
pub fn render_accounts(c: &Ctx, m: &Month, archived: bool, w: &Wallet, all: &[Month]) -> Markup {
    let view = View::Accounts { month: m.id.clone() };
    let enc = view.encode();
    let accts = w.accounts_sorted();
    let group = |g: AccountGroup| -> Vec<&Account> { accts.iter().copied().filter(|a| a.kind.group() == g).collect() };
    let (cash, invested, cards) = (group(AccountGroup::Cash), group(AccountGroup::Invested), group(AccountGroup::Card));
    let tot = totals(w, all);
    html! {
        div class="split-layout" {
        div class="side" {
            h1 { (t("accounts.title")) span class="visually-hidden" { " · " (month_label(m.year_month)) } }
            @if !accts.is_empty() {
                section class="summary card" aria-label=(t("accounts.summary")) {
                    div class="net-worth" data-card="net" {
                        span class="stat-label" { (t("accounts.net_worth")) }
                        span class=(if tot.net().is_negative() { "net-value neg" } else { "net-value" }) { (c.money(tot.net())) }
                    }
                    div class="summary-stats" {
                        div data-card="cash" { span class="stat-label" { (t("accounts.cash_total")) } span class="stat-value" { (c.money(tot.cash)) } }
                        div data-card="invested" { span class="stat-label" { (t("accounts.invested_total")) } span class="stat-value" { (c.money(tot.invested)) } }
                        div data-card="owed" { span class="stat-label" { (t("accounts.owed_total")) } span class="stat-value" { (c.money(tot.owed)) } }
                    }
                    p class="summary-note" { (t("accounts.summary_note")) }
                }
            }
            div class="action-row" {
                @if !archived {
                    button type="button" class="btn primary" data-on:click=(open_sheet(&format!("/ui/sheet/bank/connect?view={enc}"))) { (icon("bank")) " " (t("bank.connect")) }
                }
                button type="button" class="btn" data-on:click=(open_sheet(&format!("/ui/sheet/account/new?view={enc}"))) { (icon("plus")) " " (t("accounts.add")) }
                @if accts.len() >= 2 && !archived {
                    button type="button" class="btn" data-on:click=(open_sheet(&format!("/ui/sheet/transfer/new/{}?view={enc}", m.id))) { (icon("transfer")) " " (t("transfer.button")) }
                }
            }
        }
        div class="main-col" {
            (super::bank::links_section(c, w, &enc))
            @if accts.is_empty() {
                (empty_state(&t("accounts.empty_title"), &t("accounts.empty_body"), Some(html! {
                    button type="button" class="btn primary" data-on:click=(open_sheet(&format!("/ui/sheet/account/new?view={enc}"))) { (t("accounts.add_first")) }
                })))
            }
            @if !cash.is_empty() {
                section aria-labelledby="cash-h" id="cash-accounts" {
                    div class="section-head" { h2 id="cash-h" { (t("accounts.group_cash")) } }
                    ul class="rows card" { @for a in &cash { (account_row(c, m, w, all, a, &enc)) } }
                }
            }
            @if !invested.is_empty() {
                section aria-labelledby="inv-h" id="invested-accounts" {
                    div class="section-head" { h2 id="inv-h" { (t("accounts.group_invested")) } }
                    ul class="rows card" { @for a in &invested { (account_row(c, m, w, all, a, &enc)) } }
                    p class="muted small" { (t("invest.explain")) }
                }
            }
            @if !cards.is_empty() {
                section aria-labelledby="cards-h" id="card-accounts" {
                    div class="section-head" { h2 id="cards-h" { (t("accounts.group_cards")) } }
                    ul class="rows card" { @for a in &cards { (account_row(c, m, w, all, a, &enc)) } }
                    p class="muted small" { (t("cards.explain")) }
                }
            }
        }
        }
    }
}

// ----------------------------------------------------------------------
// Pieces of the Budget screen
// ----------------------------------------------------------------------

/// Account balances at a glance, for Budget.
pub fn accounts_card(c: &Ctx, m: &Month, w: &Wallet, all: &[Month]) -> Markup {
    let accts = w.accounts_sorted();
    let href = format!("/months/{}/accounts", m.id);
    html! {
        section class="card accounts-card" aria-labelledby="acc-card-h" {
            div class="section-head" {
                h2 id="acc-card-h" class="h3" { (t("accounts.title")) }
                a href=(href) class="small" { (t("accounts.see_all")) }
            }
            @if accts.is_empty() {
                p class="muted small" { (t("accounts.prompt")) }
                a class="btn small" href=(href) { (t("accounts.add_first")) }
            } @else {
                @let tot = totals(w, all);
                ul class="mini-list acct-mini" {
                    @for a in accts.iter().take(6) {
                        li data-account=(a.name) {
                            span { (a.name) }
                            @if a.kind.is_card() {
                                span class="num neg" { "−" (c.money(w.owed(&a.id, all))) }
                            } @else {
                                span class="num" { (c.money(w.balance(&a.id, all))) }
                            }
                        }
                    }
                }
                p class="acct-net" {
                    span { (t("accounts.net_worth")) }
                    strong class=(if tot.net().is_negative() { "num neg" } else { "num" }) data-card="net" { (c.money(tot.net())) }
                }
            }
        }
    }
}

/// Month-over-month bars of a goal's progress.
fn goal_bars(c: &Ctx, p: &GoalProgress) -> Markup {
    let max = p.history.iter().map(|h| h.change.get().abs()).max().unwrap_or(0).max(1);
    html! {
        div class="goal-bars" role="img" aria-label=(t("goal.bars_label")) {
            @for h in &p.history {
                @let height = (h.change.get().abs().saturating_mul(100) / max).clamp(4, 100);
                span class=(if h.change.is_negative() { "gbar neg" } else { "gbar" }) style=(format!("height: {height}%"))
                    title=(format!("{} {}", h.month.format("%b"), c.money(h.change))) {}
            }
        }
        ul class="visually-hidden" {
            @for h in &p.history { li { (h.month.format("%B %Y").to_string()) ": " (c.money(h.change)) } }
        }
    }
}

fn status_pill(s: GoalStatus) -> Markup {
    let (class, key, ic) = match s {
        GoalStatus::Done => ("pill ok", "goal.status_done", "check"),
        GoalStatus::OnTrack => ("pill ok", "goal.status_on_track", "check"),
        GoalStatus::Behind => ("pill warn", "goal.status_behind", "alert"),
        GoalStatus::NoDate => ("pill neutral", "goal.status_no_date", "flag"),
    };
    html! { span class=(class) data-goal-status=(s.as_str()) { (icon(ic)) " " (t(key)) } }
}

fn goal_card(c: &Ctx, g: &Goal, p: &GoalProgress, enc: &str) -> Markup {
    let target_word = if g.kind == GoalKind::Payoff { t("goal.paid_of") } else { t("goal.saved_of") };
    html! {
        li class="goal" data-goal=(g.name) {
            button type="button" class="goal-card" data-on:click=(open_sheet(&format!("/ui/sheet/goal/{}?view={enc}", g.id)))
                aria-label=(tf("goal.details", &[("name", &g.name)])) {
                span class="goal-top" {
                    span class="goal-name" { (icon("flag")) " " (g.name) }
                    (status_pill(p.status))
                }
                span class="goal-amounts" {
                    strong class="num" data-col="current" { (c.money(p.current)) } " " (target_word) " "
                    span class="num" data-col="target" { (c.money(p.target)) } " · " (p.percent) "%"
                }
                (bar(p.percent, if p.status == GoalStatus::Behind { "warn" } else { "ok" }, &tf("goal.meter", &[("name", &g.name)])))
                span class="goal-month" {
                    @match (p.status, p.needed_this_month) {
                        (GoalStatus::Done, _) => { (t("goal.done_line")) },
                        (_, Some(need)) => {
                            (tf("goal.this_month_of", &[("amount", &c.money(p.this_month)), ("need", &c.money(need))]))
                            @if let Some(tm) = g.target_month { " · " (tf("goal.by", &[("month", &tm.format("%b %Y").to_string())])) }
                        },
                        (_, None) => { (tf("goal.this_month", &[("amount", &c.money(p.this_month))])) },
                    }
                }
                @if p.history.len() > 1 { (goal_bars(c, p)) }
            }
        }
    }
}

/// Goals with where each stands this month, for Budget.
pub fn goals_section(c: &Ctx, m: &Month, w: &Wallet, all: &[Month], view: &View) -> Markup {
    let enc = view.encode();
    let goals = w.goals_sorted();
    html! {
        section class="goals" aria-labelledby="goals-h" id="goals" {
            div class="section-head" {
                h2 id="goals-h" { (t("goal.title")) }
                button type="button" class="chip-toggle" data-on:click=(open_sheet(&format!("/ui/sheet/goal/new?view={enc}"))) { (icon("plus")) " " (t("goal.new")) }
            }
            @if goals.is_empty() {
                div class="empty soft small-empty" {
                    p { (t("goal.empty")) }
                }
            } @else {
                ul class="goal-list" {
                    @for g in &goals {
                        @let p = w.goal_progress(g, all, m.year_month);
                        (goal_card(c, g, &p, &enc))
                    }
                }
            }
        }
    }
}

/// Goals that follow a budget line, shown in that line's sheet, with a
/// one-tap way to plan what the goal needs.
pub fn line_goals(c: &Ctx, m: &Month, w: &Wallet, all: &[Month], line: &LineView, view: &View, editable: bool) -> Markup {
    let goals = w.goals_for_line(&line.name);
    html! {
        @for g in goals {
            @let p = w.goal_progress(g, all, m.year_month);
            div class="line-goal" data-goal=(g.name) {
                p { (icon("flag")) " " strong { (g.name) } " " (status_pill(p.status)) }
                p class="small muted" {
                    (c.money(p.current)) " " (if g.kind == GoalKind::Payoff { t("goal.paid_of") } else { t("goal.saved_of") }) " " (c.money(p.target))
                    @if let Some(need) = p.needed_this_month { " · " (tf("goal.needs_this_month", &[("amount", &c.money(need))])) }
                }
                (plan_more(c, g, &p, line, view, editable))
            }
        }
    }
}

/// "Plan $50 more" on the goal's line, when this month is short of pace.
fn plan_more(c: &Ctx, g: &Goal, p: &GoalProgress, line: &LineView, view: &View, editable: bool) -> Markup {
    html! {
        @if let Some(need) = p.needed_this_month {
            @let short = need - p.this_month;
            @if g.kind == GoalKind::Save && short.is_positive() && editable {
                form class="inline plan-more" data-on:submit__prevent=(post_form(&format!("/ui/lines/{}/planned", line.id))) {
                    (view_input(view))
                    input type="hidden" name="amount" value=(crate::money::plain(line.planned + short));
                    button type="submit" class="btn small primary" { (tf("goal.plan_more", &[("amount", &c.money(short))])) }
                }
            }
        }
    }
}

/// A short list of recent transactions (Plan, account sheets).
pub fn recent_list(c: &Ctx, m: &Month, w: &Wallet, txs: &[&Transaction], view: &View, account: Option<&Id>) -> Markup {
    let enc = view.encode();
    html! {
        ul class="mini-list tx-mini" {
            @for x in txs {
                @let payee = x.payee.clone().unwrap_or_else(|| if x.is_transfer() { t("transfer.title") } else { t("tx.no_payee") });
                @let effect = match account {
                    Some(a) if x.transfer_account_id.as_ref() == Some(a) => -x.amount,
                    _ => x.amount,
                };
                li {
                    button type="button" class="mini-open" data-on:click=(open_sheet(&format!("/ui/sheet/tx/{}?view={enc}", x.id)))
                        aria-label=(tf("tx.edit_label", &[("payee", &payee), ("date", &short_date(x.date))])) {
                        span {
                            span class="mini-title" { (payee) }
                            span class="mini-sub" {
                                (short_date(x.date)) " · "
                                @if x.is_transfer() {
                                    (transfer_route(w, x))
                                } @else if let Some(l) = x.expense_line_id.as_ref().and_then(|l| m.expense_line(l)) {
                                    (l.name)
                                } @else if x.needs_line() {
                                    span class="warn-text" { (t("tx.needs_line")) }
                                } @else {
                                    (t("tx.uncategorized"))
                                }
                            }
                        }
                        span class=(if effect.is_positive() { "num pos" } else { "num" }) { (c.money(effect)) }
                    }
                }
            }
        }
    }
}

/// "Checking → Visa".
pub fn transfer_route(w: &Wallet, x: &Transaction) -> String {
    let name = |id: &Option<Id>| id.as_ref().and_then(|i| w.account(i)).map_or_else(|| t("accounts.unknown"), |a| a.name.clone());
    format!("{} → {}", name(&x.account_id), name(&x.transfer_account_id))
}

// ----------------------------------------------------------------------
// Sheets
// ----------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct AccountQuery {
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    amount: Option<i64>,
    #[serde(default)]
    kind: Option<String>,
}

fn view_or(q: &Option<String>, user: &UserRecord) -> View {
    q.as_deref().and_then(View::decode).unwrap_or_else(|| match &user.last_month_id {
        Some(m) => View::Accounts { month: m.clone() },
        None => View::Months { archived: false },
    })
}

fn sheet_error(e: &AppError, currency: &str) -> Sse {
    Sse::new().patch(sheet(&t("error.load_title"), None, html! { p { (e.human(currency, None)) } }))
}

async fn month_for_view(st: &Shared, user: &UserRecord, view: &View) -> AppResult<Month> {
    match view.month() {
        Some(mid) => Ok(st.load(user, mid).await?.month),
        None => Err(AppError::NotFound),
    }
}

pub async fn account_new_sheet(State(_st): State<Shared>, Extension(user): Extension<AuthUser>, Query(q): Query<AccountQuery>) -> Sse {
    let user = user.0;
    let view = view_or(&q.view, &user);
    let kind = q.kind.as_deref().and_then(AccountKind::parse).unwrap_or(AccountKind::Checking);
    Sse::new().patch(sheet(&t("accounts.add"), Some(&t("accounts.add_sub")), html! {
        form class="stack" id="account-form" data-signals=(format!("{{_akind: '{}'}}", kind.as_str()))
            data-on:submit__prevent=(post_form_guarded("/ui/accounts")) {
            (view_input(&view))
            fieldset class="segmented field kind-choice" {
                legend { (t("accounts.kind")) }
                @for k in AccountKind::ALL {
                    label { input type="radio" name="kind" value=(k.as_str()) checked[k == kind] data-bind:_akind; span { (kind_label(k)) } }
                }
            }
            div class="field" {
                label for="acct-name" { (t("accounts.name")) }
                input id="acct-name" type="text" name="name" required maxlength="100" autocomplete="off" placeholder=(t("accounts.name_placeholder"));
            }
            div class="field" data-show="$_akind != 'credit_card'" {
                label for="acct-balance" { (t("accounts.balance_now")) }
                input id="acct-balance" type="text" inputmode="decimal" class="money" name="balance" placeholder="0.00" autocomplete="off";
                p class="hint" { (t("accounts.balance_hint")) }
            }
            div class="field" data-show="$_akind == 'credit_card'" {
                label for="acct-owed" { (t("cards.owed_now")) }
                input id="acct-owed" type="text" inputmode="decimal" class="money" name="owed" placeholder="0.00" autocomplete="off";
            }
            div class="field" data-show="$_akind == 'credit_card'" {
                label for="acct-limit" { (t("cards.limit")) }
                input id="acct-limit" type="text" inputmode="decimal" class="money" name="credit_limit" placeholder="0.00" autocomplete="off";
            }
            span class="field-error" aria-live="polite" {}
            button type="submit" class="btn primary block" { (t("accounts.add")) }
        }
    }))
}

pub async fn account_sheet(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Query(q): Query<AccountQuery>) -> Sse {
    let user = user.0;
    let c = ctx(&st, &user, &headers);
    let view = view_or(&q.view, &user);
    let (w, all) = match (st.wallet(&user).await, st.all_months(&user).await) {
        (Ok(w), Ok(a)) => (w, a),
        (Err(e), _) | (_, Err(e)) => return sheet_error(&e, &user.currency),
    };
    let Some(a) = w.account(&id).cloned() else { return sheet_error(&AppError::NotFound, &user.currency) };
    let month = month_for_view(&st, &user, &view).await.ok();
    let txs = w.account_transactions(&id, &all);
    let enc = view.encode();
    let card = a.kind.is_card();
    let bal = w.balance(&id, &all);
    let summary = month.as_ref().filter(|_| card).map(|m| w.card_summary(&id, &all, m));
    let first_cash = w.accounts_sorted().into_iter().find(|x| x.kind.group() == AccountGroup::Cash).map(|x| x.id.clone());
    if a.kind.is_invested() {
        return Sse::new().patch(sheet(&a.name, Some(&kind_label(a.kind)), invested_sheet(&c, &view, &w, &all, &a, month.as_ref(), first_cash.as_ref())));
    }
    Sse::new().patch(sheet(&a.name, Some(&kind_label(a.kind)), html! {
        div class="sheet-stats" {
            @if let Some(s) = &summary {
                div class="sheet-stat" { span class="stat-label" { (t("cards.owed")) } span class="stat-value neg" data-col="owed" { (c.money(s.owed)) } }
                div class="sheet-stat" { span class="stat-label" { (t("cards.ready")) } span class="stat-value" data-col="ready" { (c.money(s.ready_to_pay)) } }
                div class="sheet-stat" { span class="stat-label" { (t("cards.carried")) } span class="stat-value" data-col="carried" { (c.money(s.carried)) } }
            } @else {
                div class="sheet-stat" { span class="stat-label" { (t("accounts.balance")) } span class=(if bal.is_negative() { "stat-value neg" } else { "stat-value" }) data-col="balance" { (c.money(bal)) } }
                div class="sheet-stat" { span class="stat-label" { (t("accounts.checked")) } span class="stat-value small-stat" { (a.reconciled_on.map_or_else(|| "—".into(), short_date)) } }
            }
        }
        @if let (Some(s), Some(m)) = (&summary, &month) {
            div class="card-explain" {
                @if s.ready_to_pay.is_positive() {
                    p { (tf("cards.ready_explain", &[("amount", &c.money(s.ready_to_pay)), ("month", &month_label(m.year_month))])) }
                }
                @if s.carried.is_positive() {
                    p class="muted small" { (tf("cards.carried_explain", &[("amount", &c.money(s.carried))])) }
                }
                @if s.unbudgeted_spending.is_positive() {
                    p class="warn-text" { (tf("cards.needs_line_explain", &[("amount", &c.money(s.unbudgeted_spending))])) }
                }
                @if let Some(min) = a.minimum_payment {
                    @if s.paid < min && s.owed.is_positive() {
                        p class="warn-text" { (tf("cards.minimum_due", &[("amount", &c.money(min - s.paid))])) }
                    }
                }
                @if let Some(u) = s.utilization {
                    p class="small muted" { (tf("cards.utilization", &[("pct", &u.to_string())])) }
                    (bar(u.min(100), if u >= 30 { "over" } else { "" }, &tf("cards.utilization_label", &[("pct", &u.to_string())])))
                }
                @if s.owed.is_positive() && first_cash.is_some() {
                    @let amt = if s.ready_to_pay.is_positive() { s.ready_to_pay } else { s.owed };
                    button type="button" class="btn primary block"
                        data-on:click=(open_sheet(&format!("/ui/sheet/transfer/new/{}?view={enc}&to={id}&from={}&amount={}", m.id, first_cash.clone().unwrap_or_default(), amt.get()))) {
                        (tf("cards.pay", &[("amount", &c.money(amt))]))
                    }
                }
            }
        }

        section class="sheet-section" aria-labelledby="rec-h" {
            h3 id="rec-h" { (t("accounts.reconcile_title")) }
            form class="stack" data-on:submit__prevent=(post_form_guarded(&format!("/ui/accounts/{id}/reconcile"))) {
                (view_input(&view))
                label for="rec-amount" { (if card { t("cards.reconcile_label") } else { t("accounts.reconcile_label") }) }
                div class="inline-field" {
                    input id="rec-amount" type="text" inputmode="decimal" class="money" name="balance" required autocomplete="off"
                        value=(crate::money::plain(if card { (-bal).max(Cents::ZERO) } else { bal }));
                    button type="submit" class="btn primary" { (t("accounts.reconcile")) }
                }
                p class="hint" { (t("accounts.reconcile_hint")) }
            }
        }

        section class="sheet-section" aria-labelledby="acct-tx-h" {
            h3 id="acct-tx-h" { (t("accounts.recent")) }
            @if txs.is_empty() { p class="muted small" { (t("accounts.no_tx")) } }
            @if let Some(m) = &month {
                (recent_list(&c, m, &w, &txs.iter().take(8).copied().collect::<Vec<_>>(), &view, Some(&id)))
            }
        }

        (edit_section(&a, &view, !txs.is_empty()))
    }))
}

fn adjustment_label(k: AdjustmentKind) -> String {
    t(&format!("invest.adj_{}", k.as_str()))
}

/// A retirement or investment account: its value, this year's
/// contributions and growth, and ways to record both.
fn invested_sheet(c: &Ctx, view: &View, w: &Wallet, all: &[Month], a: &Account, month: Option<&Month>, first_cash: Option<&Id>) -> Markup {
    let id = &a.id;
    let year = month.map_or(c.today.year(), |m| m.year_month.year());
    let s = w.invested_summary(id, all, year);
    let activity = w.account_activity(id, all);
    let enc = view.encode();
    html! {
        div class="sheet-stats" {
            div class="sheet-stat" { span class="stat-label" { (t("accounts.balance")) } span class="stat-value" data-col="balance" { (c.money(s.balance)) } }
            div class="sheet-stat" { span class="stat-label" { (tf("invest.contributed_year", &[("year", &year.to_string())])) } span class="stat-value" data-col="contributed" { (c.money(s.contributed)) } }
            div class="sheet-stat" { span class="stat-label" { (tf("invest.growth_year", &[("year", &year.to_string())])) }
                span class=(if s.growth.is_negative() { "stat-value neg" } else if s.growth.is_positive() { "stat-value pos" } else { "stat-value" }) data-col="growth" { (signed(c, s.growth)) } }
        }
        @if let (Some(m), Some(from)) = (month, first_cash) {
            button type="button" class="btn block"
                data-on:click=(open_sheet(&format!("/ui/sheet/transfer/new/{}?view={enc}&to={id}&from={from}", m.id))) {
                (icon("transfer")) " " (t("invest.contribute_transfer"))
            }
        }

        section class="sheet-section" aria-labelledby="inv-upd-h" {
            h3 id="inv-upd-h" { (t("invest.update_title")) }
            form class="stack" data-on:submit__prevent=(post_form_guarded(&format!("/ui/accounts/{id}/reconcile"))) {
                (view_input(view))
                label for="inv-balance" { (t("invest.update_label")) }
                div class="inline-field" {
                    input id="inv-balance" type="text" inputmode="decimal" class="money" name="balance" required autocomplete="off" value=(crate::money::plain(s.balance));
                    button type="submit" class="btn primary" { (t("accounts.reconcile")) }
                }
                p class="hint" { (t("invest.update_hint")) }
            }
        }

        section class="sheet-section" aria-labelledby="inv-con-h" {
            h3 id="inv-con-h" { (t("invest.contribution_title")) }
            form class="stack" id="contribution-form" data-clear data-on:submit__prevent=(post_form_guarded(&format!("/ui/accounts/{id}/contribution"))) {
                (view_input(view))
                div class="two-col" {
                    div class="field" {
                        label for="inv-amount" { (t("tx.amount")) }
                        input id="inv-amount" type="text" inputmode="decimal" class="money" name="amount" required placeholder="0.00" autocomplete="off";
                    }
                    div class="field" {
                        label for="inv-date" { (t("tx.date")) }
                        input id="inv-date" type="date" name="date" required value=(c.today.to_string());
                    }
                }
                p class="hint" { (t("invest.contribution_hint")) }
                span class="field-error" aria-live="polite" {}
                button type="submit" class="btn primary" { (t("invest.contribution_add")) }
            }
        }

        section class="sheet-section" aria-labelledby="inv-act-h" {
            h3 id="inv-act-h" { (t("invest.activity")) }
            @if activity.is_empty() { p class="muted small" { (t("accounts.no_tx")) } }
            ul class="mini-list activity" {
                @for item in activity.iter().take(12) {
                    @match item {
                        Activity::Tx(x) => {
                            @let effect = if x.transfer_account_id.as_ref() == Some(id) { -x.amount } else { x.amount };
                            li {
                                span { span class="mini-title" { (x.payee.clone().unwrap_or_else(|| t("transfer.title"))) }
                                    span class="mini-sub" { (short_date(x.date)) " · " (transfer_route(w, x)) } }
                                span class=(if effect.is_positive() { "num pos" } else { "num" }) { (signed(c, effect)) }
                            }
                        },
                        Activity::Adjustment(adj) => {
                            li data-adjustment=(adj.kind.as_str()) {
                                span { span class="mini-title" { (adjustment_label(adj.kind)) } span class="mini-sub" { (short_date(adj.date)) } }
                                span class="adj-end" {
                                    span class=(if adj.amount.is_negative() { "num neg" } else { "num pos" }) { (signed(c, adj.amount)) }
                                    @if matches!(adj.kind, AdjustmentKind::Contribution | AdjustmentKind::Growth) {
                                        form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/adjustments/{}/delete", adj.id))) {
                                            (view_input(view))
                                            button type="submit" class="icon-btn small-icon" data-confirm=(t("invest.delete_confirm"))
                                                aria-label=(tf("invest.delete_label", &[("what", &adjustment_label(adj.kind)), ("date", &short_date(adj.date))])) { (icon("trash")) }
                                        }
                                    }
                                }
                            }
                        },
                    }
                }
            }
        }
        (edit_section(a, view, w.is_used(id, all)))
    }
}

/// Rename, card details, reorder, archive or delete.
fn edit_section(a: &Account, view: &View, has_history: bool) -> Markup {
    html! {
        section class="sheet-section" aria-labelledby="acct-edit-h" {
            h3 id="acct-edit-h" { (t("accounts.edit")) }
            form class="stack" data-on:submit__prevent=(post_form(&format!("/ui/accounts/{}/edit", a.id))) {
                (view_input(view))
                div class="field" {
                    label for="ae-name" { (t("accounts.name")) }
                    input id="ae-name" type="text" name="name" value=(a.name) required maxlength="100";
                }
                @if a.kind.is_card() {
                    div class="two-col" {
                        div class="field" {
                            label for="ae-limit" { (t("cards.limit")) }
                            input id="ae-limit" type="text" inputmode="decimal" class="money" name="credit_limit" value=[a.credit_limit.map(crate::money::plain)];
                        }
                        div class="field" {
                            label for="ae-min" { (t("cards.minimum")) }
                            input id="ae-min" type="text" inputmode="decimal" class="money" name="minimum_payment" value=[a.minimum_payment.map(crate::money::plain)];
                        }
                    }
                    div class="field" {
                        label for="ae-apr" { (t("cards.apr")) }
                        input id="ae-apr" type="text" inputmode="decimal" name="apr" value=[a.apr_bp.map(|b| format!("{}.{:02}", b / 100, b % 100))] placeholder="19.99";
                    }
                }
                button type="submit" class="btn primary" { (t("common.save")) }
            }
            div class="sheet-actions" {
                form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/accounts/{}/move", a.id))) {
                    (view_input(view)) input type="hidden" name="direction" value="up";
                    button type="submit" class="btn small" aria-label=(tf("accounts.move_up", &[("name", &a.name)])) { (icon("up")) " " (t("common.move_up")) }
                }
                form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/accounts/{}/move", a.id))) {
                    (view_input(view)) input type="hidden" name="direction" value="down";
                    button type="submit" class="btn small" aria-label=(tf("accounts.move_down", &[("name", &a.name)])) { (icon("down")) " " (t("common.move_down")) }
                }
                @if !has_history {
                    form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/accounts/{}/delete", a.id))) {
                        (view_input(view))
                        button type="submit" class="btn small danger" aria-label=(tf("accounts.delete", &[("name", &a.name)])) data-confirm=(tf("accounts.delete_confirm", &[("name", &a.name)])) {
                            (icon("trash")) " " (t("common.delete"))
                        }
                    }
                } @else {
                    form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/accounts/{}/archive", a.id))) {
                        (view_input(view))
                        button type="submit" class="btn small" data-confirm=(tf("accounts.archive_confirm", &[("name", &a.name)])) { (t("accounts.archive")) }
                    }
                }
            }
        }
    }
}

/// Prefill for a new transfer (e.g. "Pay $120 to Visa from Checking").
#[derive(Default)]
pub struct TransferPrefill {
    pub from: Option<Id>,
    pub to: Option<Id>,
    pub amount: Option<Cents>,
}

/// The transfer form, new or editing `existing`.
pub fn transfer_form(c: &Ctx, m: &Month, view: &View, w: &Wallet, existing: Option<&Transaction>, pre: &TransferPrefill) -> Markup {
    let url = match existing {
        Some(x) => format!("/ui/transfers/{}", x.id),
        None => format!("/ui/months/{}/transfers", m.id),
    };
    let from = existing.and_then(|x| x.account_id.clone()).or_else(|| pre.from.clone());
    let to = existing.and_then(|x| x.transfer_account_id.clone()).or_else(|| pre.to.clone());
    let amount = existing.map(|x| x.amount.abs()).or(pre.amount);
    let date = existing.map_or_else(|| default_tx_date(c, m), |x| x.date);
    html! {
        form class="stack transfer-form" id="transfer-form" data-clear
            data-on:submit__prevent=(post_form_guarded(&url)) {
            (view_input(view))
            div class="two-col" {
                div class="field" {
                    label for="tr-from" { (t("transfer.from")) }
                    select id="tr-from" name="from_account" required { option value="" { (t("transfer.choose")) } (account_options(w, from.as_ref(), false)) }
                }
                div class="field" {
                    label for="tr-to" { (t("transfer.to")) }
                    select id="tr-to" name="to_account" required { option value="" { (t("transfer.choose")) } (account_options(w, to.as_ref(), false)) }
                }
            }
            div class="field big-amount" {
                label for="tr-amount" { (t("tx.amount")) }
                input id="tr-amount" type="text" inputmode="decimal" class="money" name="amount" required placeholder="0.00" autocomplete="off"
                    value=[amount.map(crate::money::plain)];
            }
            div class="two-col" {
                div class="field" {
                    label for="tr-date" { (t("tx.date")) }
                    input id="tr-date" type="date" name="date" required value=(date.to_string())
                        min=(m.year_month.to_string()) max=(recurrence::last_of_month(m.year_month).to_string());
                }
                div class="field" {
                    label for="tr-notes" { (t("tx.notes")) }
                    input id="tr-notes" type="text" name="notes" maxlength="2000" value=[existing.and_then(|x| x.notes.clone())];
                }
            }
            details class="more-options" open[existing.is_some_and(|x| x.expense_line_id.is_some() || x.paycheck_id.is_some())] {
                summary { (t("transfer.more")) }
                div class="field" {
                    label for="tr-line" { (t("transfer.line")) }
                    select id="tr-line" name="expense_line_id" { (line_options(m, existing.and_then(|x| x.expense_line_id.as_ref()))) }
                    p class="hint" { (t("transfer.line_hint")) }
                }
                div class="field" {
                    label for="tr-pc" { (t("tx.paycheck")) }
                    select id="tr-pc" name="paycheck_id" { (paycheck_options(m, existing.and_then(|x| x.paycheck_id.as_ref()))) }
                }
            }
            span class="field-error" aria-live="polite" {}
            button type="submit" class="btn primary block" { (if existing.is_some() { t("common.save") } else { t("transfer.save") }) }
        }
        @if let Some(x) = existing {
            form class="dialog-danger" data-on:submit__prevent=(post_form(&format!("/ui/transactions/{}/delete", x.id))) {
                (view_input(view))
                button type="submit" class="btn small danger" data-confirm=(t("transfer.delete_confirm")) { (icon("trash")) " " (t("transfer.delete")) }
            }
        }
    }
}

pub async fn transfer_new_sheet(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(mid): Path<Id>, Query(q): Query<AccountQuery>) -> Sse {
    let user = user.0;
    let c = ctx(&st, &user, &headers);
    let view = q.view.as_deref().and_then(View::decode).unwrap_or(View::Accounts { month: mid.clone() });
    let (m, w) = match (st.load(&user, &mid).await, st.wallet(&user).await) {
        (Ok(l), Ok(w)) => (l.month, w),
        (Err(e), _) | (_, Err(e)) => return sheet_error(&e, &user.currency),
    };
    let pre = TransferPrefill {
        from: q.from.as_deref().filter(|s| !s.is_empty()).map(Id::new),
        to: q.to.as_deref().filter(|s| !s.is_empty()).map(Id::new),
        amount: q.amount.filter(|a| *a > 0).map(Cents::new),
    };
    Sse::new().patch(sheet(&t("transfer.title"), Some(&t("transfer.sub")), transfer_form(&c, &m, &view, &w, None, &pre)))
}

/// A goal's money field, shown formatted ("$5,000.00") and re-formatted as
/// the user leaves it.
fn goal_money(c: &Ctx, id: &str, name: &str, value: Option<Cents>) -> Markup {
    html! {
        input id=(id) type="text" inputmode="decimal" autocomplete="off" class="money" data-format="money" name=(name)
            placeholder=(c.money(Cents::ZERO)) value=[value.map(|v| c.money(v))];
    }
}

fn goal_kind_fields(c: &Ctx, w: &Wallet, m: &Month, all: &[Month], g: Option<&Goal>) -> Markup {
    let kind = g.map_or(GoalKind::Save, |g| g.kind);
    let owed = |track: GoalTrack| w.debt_now(&track, all, m.year_month);
    let owed_label = |name: &str, debt: Cents| tf("goal.owed_option", &[("name", name), ("amount", &c.money(debt))]);
    let sel_line = |name: &str| matches!(g.map(|g| &g.track), Some(GoalTrack::Line { name: n }) if n.eq_ignore_ascii_case(name));
    let sel_acct = |id: &Id| matches!(g.map(|g| &g.track), Some(GoalTrack::Account { id: a }) if a == id);
    let accts = w.accounts_sorted();
    let debt_lines: Vec<&ExpenseLine> = m.expense_lines.iter().filter(|l| m.is_debt_line(&l.id)).collect();
    html! {
        fieldset class="segmented field" {
            legend { (t("goal.kind")) }
            label { input type="radio" name="kind" value="save" checked[kind == GoalKind::Save] data-bind:_gkind; span { (t("goal.kind_save")) } }
            label { input type="radio" name="kind" value="payoff" checked[kind == GoalKind::Payoff] data-bind:_gkind; span { (t("goal.kind_payoff")) } }
        }
        div class="field" {
            label for="goal-name" { (t("goal.name")) }
            input id="goal-name" type="text" name="name" required maxlength="100" value=[g.map(|g| g.name.clone())] placeholder=(t("goal.name_placeholder"));
        }
        div class="field" data-show="$_gkind == 'save'" {
            label for="goal-target" { (t("goal.target")) }
            (goal_money(c, "goal-target", "target_amount", g.filter(|g| g.kind == GoalKind::Save).map(|g| g.target_amount)))
        }
        div class="field" data-show="$_gkind == 'save'" {
            label for="goal-track-save" { (t("goal.track_save")) }
            select id="goal-track-save" name="track_save" {
                option value=(NEW_LINE) selected[g.is_none()] { (t("goal.track_new")) }
                optgroup label=(t("goal.track_lines")) {
                    @for l in m.expense_lines.iter().filter(|l| !m.is_debt_line(&l.id)) {
                        option value=(format!("line:{}", l.name)) selected[sel_line(&l.name)] { (l.name) }
                    }
                }
                @let savings: Vec<&&Account> = accts.iter().filter(|a| !a.kind.is_card()).collect();
                @if !savings.is_empty() {
                    optgroup label=(t("goal.track_accounts")) {
                        @for a in savings { option value=(format!("account:{}", a.id)) selected[sel_acct(&a.id)] { (a.name) " (" (t("goal.balance_word")) ")" } }
                    }
                }
            }
            p class="hint" { (t("goal.track_save_hint")) }
        }
        @let cards: Vec<&&Account> = accts.iter().filter(|a| a.kind.is_card()).collect();
        @if cards.is_empty() && debt_lines.is_empty() {
            p class="hint" data-show="$_gkind == 'payoff'" data-goal-no-debts { (t("goal.no_debts")) }
        } @else {
            div class="field" data-show="$_gkind == 'payoff'" {
                label for="goal-track-payoff" { (t("goal.track_payoff")) }
                select id="goal-track-payoff" name="track_payoff" {
                    @if !cards.is_empty() {
                        optgroup label=(t("accounts.group_cards")) {
                            @for a in &cards {
                                option value=(format!("account:{}", a.id)) selected[sel_acct(&a.id)] { (owed_label(&a.name, owed(GoalTrack::Account { id: a.id.clone() }))) }
                            }
                        }
                    }
                    @if !debt_lines.is_empty() {
                        optgroup label=(t("goal.track_debt_lines")) {
                            @for l in &debt_lines {
                                option value=(format!("line:{}", l.name)) selected[sel_line(&l.name)] { (owed_label(&l.name, owed(GoalTrack::Line { name: l.name.clone() }))) }
                            }
                        }
                    }
                }
                p class="hint" { (t("goal.track_payoff_hint")) }
            }
            div class="field" data-show="$_gkind == 'payoff'" {
                label for="goal-payoff" { (t("goal.payoff_amount")) }
                // Blank means all of it.
                (goal_money(c, "goal-payoff", "payoff_amount", g.filter(|g| g.kind == GoalKind::Payoff && g.target_amount < g.starting_amount).map(|g| g.target_amount)))
                p class="hint" { (t("goal.payoff_amount_hint")) }
            }
        }
        div class="two-col" {
            div class="field" {
                label for="goal-by" { (t("goal.by_label")) }
                input id="goal-by" type="month" name="target_month" placeholder="2027-06"
                    value=[g.and_then(|g| g.target_month).map(|d| d.format("%Y-%m").to_string())];
            }
            div class="field" data-show="$_gkind == 'save'" {
                label for="goal-start" { (t("goal.starting")) }
                (goal_money(c, "goal-start", "starting_amount", g.filter(|g| g.kind == GoalKind::Save && !g.starting_amount.is_zero()).map(|g| g.starting_amount)))
            }
        }
    }
}

pub async fn goal_new_sheet(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Query(q): Query<AccountQuery>) -> Sse {
    let user = user.0;
    let c = ctx(&st, &user, &headers);
    let view = view_or(&q.view, &user);
    let (m, w, all) = match (month_for_view(&st, &user, &view).await, st.wallet(&user).await, st.all_months(&user).await) {
        (Ok(m), Ok(w), Ok(a)) => (m, w, a),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return sheet_error(&e, &user.currency),
    };
    Sse::new().patch(sheet(&t("goal.new_title"), Some(&t("goal.new_sub")), html! {
        form class="stack" id="goal-form" data-signals="{_gkind: 'save'}" data-on:submit__prevent=(post_form_guarded("/ui/goals")) {
            (view_input(&view))
            (goal_kind_fields(&c, &w, &m, &all, None))
            span class="field-error" aria-live="polite" {}
            button type="submit" class="btn primary block" { (t("goal.create")) }
        }
    }))
}

pub async fn goal_sheet(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Query(q): Query<AccountQuery>) -> Sse {
    let user = user.0;
    let c = ctx(&st, &user, &headers);
    let view = view_or(&q.view, &user);
    let (m, w, all) = match (month_for_view(&st, &user, &view).await, st.wallet(&user).await, st.all_months(&user).await) {
        (Ok(m), Ok(w), Ok(a)) => (m, w, a),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return sheet_error(&e, &user.currency),
    };
    let Some(g) = w.goal(&id).cloned() else { return sheet_error(&AppError::NotFound, &user.currency) };
    let p = w.goal_progress(&g, &all, m.year_month);
    let payoff = g.kind == GoalKind::Payoff;
    let tracked = match &g.track {
        GoalTrack::Line { name } => tf("goal.tracked_line", &[("name", name)]),
        GoalTrack::Account { id } => tf("goal.tracked_account", &[("name", &w.account(id).map_or_else(|| t("accounts.unknown"), |a| a.name.clone()))]),
    };
    Sse::new().patch(sheet(&g.name, Some(&tracked), html! {
        div class="sheet-stats" {
            div class="sheet-stat" { span class="stat-label" { (if payoff { t("goal.paid_down") } else { t("goal.saved") }) } span class="stat-value" data-col="current" { (c.money(p.current)) } }
            div class="sheet-stat" { span class="stat-label" { (if payoff { t("goal.still_owed") } else { t("goal.to_go") }) } span class="stat-value" data-col="remaining" { (c.money(p.remaining)) } }
            div class="sheet-stat" { span class="stat-label" { (t("goal.progress")) } span class="stat-value" data-col="percent" { (p.percent) "%" } }
        }
        p { (status_pill(p.status)) " "
            @match (p.status, p.needed_this_month, p.months_left) {
                (GoalStatus::Done, _, _) => { (t("goal.done_explain")) },
                (GoalStatus::OnTrack, Some(need), Some(n)) => { (tf("goal.on_track_explain", &[("amount", &c.money(p.this_month)), ("need", &c.money(need)), ("n", &n.to_string())])) },
                (GoalStatus::Behind, Some(need), Some(n)) => { (tf("goal.behind_explain", &[("amount", &c.money(p.this_month)), ("need", &c.money(need)), ("n", &n.to_string())])) },
                _ => { (tf("goal.no_date_explain", &[("amount", &c.money(p.this_month))])) },
            }
        }
        @if let GoalTrack::Line { name } = &g.track {
            @if let Some(l) = m.category_views().into_iter().flat_map(|c| c.lines).find(|l| l.name.eq_ignore_ascii_case(name)) {
                (plan_more(&c, &g, &p, &l, &view, m.allocations_editable()))
            }
        }
        section class="sheet-section" aria-labelledby="gh-h" {
            h3 id="gh-h" { (t("goal.history")) }
            table class="mini-table" {
                thead { tr { th scope="col" { (t("goal.month_col")) } th scope="col" class="num" { (t("goal.change_col")) } th scope="col" class="num" { (t("goal.total_col")) } } }
                tbody {
                    @for h in p.history.iter().rev() {
                        tr { td { (h.month.format("%b %Y").to_string()) } td class="num" { (c.money(h.change)) } td class="num" { (c.money(h.value)) } }
                    }
                }
            }
        }
        section class="sheet-section" aria-labelledby="ge-h" {
            h3 id="ge-h" { (t("goal.edit")) }
            form class="stack" data-signals=(format!("{{_gkind: '{}'}}", g.kind.as_str())) data-on:submit__prevent=(post_form_guarded(&format!("/ui/goals/{id}"))) {
                (view_input(&view))
                (goal_kind_fields(&c, &w, &m, &all, Some(&g)))
                span class="field-error" aria-live="polite" {}
                button type="submit" class="btn primary" { (t("common.save")) }
            }
            div class="sheet-actions" {
                form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/goals/{id}/delete"))) {
                    (view_input(&view))
                    button type="submit" class="btn small danger" aria-label=(tf("goal.delete", &[("name", &g.name)])) data-confirm=(tf("goal.delete_confirm", &[("name", &g.name)])) {
                        (icon("trash")) " " (t("common.delete"))
                    }
                }
            }
        }
    }))
}

// ----------------------------------------------------------------------
// Actions
// ----------------------------------------------------------------------

async fn wallet_action<T>(
    st: &Shared,
    user: &UserRecord,
    headers: &HeaderMap,
    view: View,
    op: impl FnOnce(&mut Wallet, &[Month]) -> AppResult<T>,
    toasts: impl FnOnce(&T, &Wallet) -> Vec<Markup>,
) -> Sse {
    match st.mutate_wallet(user, op).await {
        Ok((v, w)) => {
            let ts = toasts(&v, &w);
            done(st, user, headers, &view, ts).await
        }
        Err(e) => failed(st, user, headers, &view, &e).await,
    }
}

fn fallback_view(user: &UserRecord) -> View {
    match &user.last_month_id {
        Some(m) => View::Accounts { month: m.clone() },
        None => View::Months { archived: false },
    }
}

pub async fn add_account(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    let kind = AccountKind::parse(field(&f, "kind")).unwrap_or(AccountKind::Checking);
    let parsed = if kind.is_card() {
        opt_money_field(&f, "owed").and_then(|o| opt_money_field(&f, "credit_limit").map(|l| (o, l)))
    } else {
        signed_money(&f, "balance").map(|b| (b, None))
    };
    let (balance, limit) = match parsed {
        Ok((b, l)) => (b.unwrap_or(Cents::ZERO), l),
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let today = st.today_for(&user);
    let name = field(&f, "name").to_string();
    wallet_action(&st, &user, &headers, view, move |w, _| Ok(w.add_account(&name, kind, balance, today, limit)?), |_, _| {
        vec![toast(ToastKind::Success, &t("accounts.added"), None)]
    })
    .await
}

/// A money field that may be negative (an overdrawn account): "-45.10".
fn signed_money(f: &HashMap<String, String>, k: &str) -> AppResult<Option<Cents>> {
    let raw = field(f, k);
    match raw.strip_prefix('-').or_else(|| raw.strip_prefix('−')) {
        Some(rest) => {
            let mut g = HashMap::new();
            g.insert(k.to_string(), rest.to_string());
            Ok(opt_money_field(&g, k)?.map(|c| -c))
        }
        None => opt_money_field(f, k),
    }
}

pub async fn edit_account(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    let parsed = (|| -> AppResult<(Option<Cents>, Option<Cents>, Option<i64>)> {
        let apr = opt_money_field(&f, "apr")?.map(Cents::get);
        Ok((opt_money_field(&f, "credit_limit")?, opt_money_field(&f, "minimum_payment")?, apr))
    })();
    let (limit, min, apr) = match parsed {
        Ok(v) => v,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let name = field(&f, "name").to_string();
    wallet_action(&st, &user, &headers, view, move |w, _| {
        w.rename_account(&id, &name)?;
        w.set_card_details(&id, limit, apr, min)?;
        Ok(())
    }, |_, _| vec![toast(ToastKind::Success, &t("accounts.saved"), None)])
    .await
}

pub async fn reconcile_account(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    let actual = match signed_money(&f, "balance") {
        Ok(Some(a)) => a,
        Ok(None) => return failed(&st, &user, &headers, &view, &AppError::bad(t("err.amount_required"))).await,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let today = st.today_for(&user);
    let currency = user.currency.clone();
    wallet_action(&st, &user, &headers, view, move |w, all| {
        let invested = w.account(&id).is_some_and(|a| a.kind.is_invested());
        Ok((w.reconcile(&id, actual, all, today)?, invested))
    }, move |(delta, invested), _| {
        let delta = *delta;
        if *invested {
            let msg = if delta.is_zero() { t("invest.updated_same") } else { tf("invest.updated", &[("amount", &{
                let s = crate::money::format(delta, &currency);
                if delta.is_positive() { format!("+{s}") } else { s }
            })]) };
            vec![toast(ToastKind::Success, &msg, None)]
        } else if delta.is_zero() {
            vec![toast(ToastKind::Success, &t("accounts.reconciled_match"), None)]
        } else {
            vec![toast(ToastKind::Success, &tf("accounts.reconciled_adjusted", &[("amount", &crate::money::format(delta, &currency))]), None)]
        }
    })
    .await
}

pub async fn archive_account(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    wallet_action(&st, &user, &headers, view, move |w, _| Ok(w.set_archived(&id, true)?), |_, _| vec![toast(ToastKind::Success, &t("accounts.archived"), None)]).await
}

pub async fn move_account(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    let up = field(&f, "direction") == "up";
    wallet_action(&st, &user, &headers, view, move |w, _| Ok(w.move_account(&id, up)?), |_, _| Vec::new()).await
}

pub async fn delete_account(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    wallet_action(&st, &user, &headers, view, move |w, all| {
        if w.is_used(&id, all) {
            return Err(AppError::bad(t("err.account_in_use")));
        }
        Ok(w.delete_account(&id)?)
    }, |_, _| vec![toast(ToastKind::Success, &t("accounts.deleted"), None)])
    .await
}

pub async fn add_contribution(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    let parsed = money_field(&f, "amount").and_then(|a| {
        chrono::NaiveDate::parse_from_str(field(&f, "date"), "%Y-%m-%d").map(|d| (a, d)).map_err(|_| AppError::bad(t("err.date")))
    });
    let (amount, date) = match parsed {
        Ok(v) => v,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    wallet_action(&st, &user, &headers, view, move |w, _| Ok(w.add_contribution(&id, amount, date)?), |_, _| {
        vec![toast(ToastKind::Success, &t("invest.contribution_saved"), None)]
    })
    .await
}

pub async fn delete_adjustment(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    wallet_action(&st, &user, &headers, view, move |w, _| Ok(w.delete_adjustment(&id)?), |_, _| vec![toast(ToastKind::Success, &t("invest.deleted"), None)]).await
}

fn transfer_from_form(f: &HashMap<String, String>, id: Id) -> AppResult<Transaction> {
    let date = chrono::NaiveDate::parse_from_str(field(f, "date"), "%Y-%m-%d").map_err(|_| AppError::bad(t("err.date")))?;
    let amount = money_field(f, "amount")?;
    if !amount.is_positive() {
        return Err(AppError::Domain(DomainError::NonPositiveAmount));
    }
    let from = opt_id(f, "from_account").ok_or_else(|| AppError::bad(t("err.transfer_accounts")))?;
    let to = opt_id(f, "to_account").ok_or_else(|| AppError::bad(t("err.transfer_accounts")))?;
    if from == to {
        return Err(AppError::bad(t("err.transfer_same")));
    }
    let notes = field(f, "notes");
    Ok(Transaction {
        id,
        date,
        amount: -amount,
        payee: None,
        notes: (!notes.is_empty()).then(|| notes.to_string()),
        expense_line_id: opt_id(f, "expense_line_id"),
        paycheck_id: opt_id(f, "paycheck_id"),
        split_group: None,
        account_id: Some(from),
        transfer_account_id: Some(to),
        external_id: None,
    })
}

pub async fn add_transfer(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(mid): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, View::Accounts { month: mid.clone() });
    let tx = match transfer_from_form(&f, Id::generate()) {
        Ok(t) => t,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    if let Err(e) = st.check_accounts(&user, &[tx.account_id.as_ref(), tx.transfer_account_id.as_ref()]).await {
        return failed(&st, &user, &headers, &view, &e).await;
    }
    let u = user.clone();
    month_action(&st, &user, &headers, view, Ok(mid), |m| {
        let b = overs(m);
        m.add_transaction(tx).map(|_| b)
    }, move |b, m| {
        let mut v = vec![toast(ToastKind::Success, &t("transfer.saved"), None)];
        v.extend(overspend_toasts(&u, b, m));
        v
    })
    .await
}

pub async fn update_transfer(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let mid = st.resolve(&user, Owner::Transaction, &id).await;
    let view = view_of(&f, View::Transactions { month: mid.as_ref().cloned().unwrap_or_default(), filter: TxFilter::All });
    let tx = match transfer_from_form(&f, id) {
        Ok(t) => t,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    if let Err(e) = st.check_accounts(&user, &[tx.account_id.as_ref(), tx.transfer_account_id.as_ref()]).await {
        return failed(&st, &user, &headers, &view, &e).await;
    }
    month_action(&st, &user, &headers, view, mid, |m| m.update_transaction(tx), |_, _| vec![toast(ToastKind::Success, &t("transfer.saved"), None)]).await
}

fn parse_month(s: &str) -> Option<chrono::NaiveDate> {
    let s = s.trim();
    chrono::NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d")
        .or_else(|_| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d"))
        .ok()
        .and_then(|d| d.with_day(1))
}

fn parse_track(v: &str) -> Option<GoalTrack> {
    if let Some(name) = v.strip_prefix("line:") {
        return (!name.trim().is_empty()).then(|| GoalTrack::Line { name: name.trim().to_string() });
    }
    v.strip_prefix("account:").filter(|s| !s.is_empty()).map(|id| GoalTrack::Account { id: Id::new(id) })
}

/// The saving goal's "Counts" choice that makes a budget line for it.
const NEW_LINE: &str = "new";

/// A goal read from its form.
struct GoalForm {
    goal: Goal,
    /// Payoff: how much to pay off, or `None` for all of it. The goal's
    /// amounts are filled in from the debt when it is saved.
    payoff: Option<Option<Cents>>,
    /// The goal follows a line named after it, to be added if missing.
    new_line: bool,
}

/// Reads the goal form. `start` is the month progress counts from.
fn goal_from_form(f: &HashMap<String, String>, id: Id, start: chrono::NaiveDate) -> AppResult<GoalForm> {
    let kind = GoalKind::parse(field(f, "kind")).unwrap_or(GoalKind::Save);
    let raw_track = field(f, if kind == GoalKind::Save { "track_save" } else { "track_payoff" });
    let new_line = kind == GoalKind::Save && raw_track == NEW_LINE;
    let track = if new_line { Some(GoalTrack::Line { name: field(f, "name").trim().to_string() }) } else { parse_track(raw_track) }
        .ok_or_else(|| AppError::bad(t("err.goal_track")))?;
    let target_month = match field(f, "target_month") {
        "" => None,
        v => Some(parse_month(v).ok_or_else(|| AppError::bad(t("err.goal_month")))?),
    };
    if target_month.is_some_and(|t| t < start) {
        return Err(AppError::bad(t("err.goal_past")));
    }
    let target = if kind == GoalKind::Save { money_field(f, "target_amount")? } else { Cents::ZERO };
    let starting = if kind == GoalKind::Save && matches!(track, GoalTrack::Line { .. }) { opt_money_field(f, "starting_amount")?.unwrap_or(Cents::ZERO) } else { Cents::ZERO };
    Ok(GoalForm {
        goal: Goal { id, name: field(f, "name").to_string(), kind, target_amount: target, target_month, track, start_month: start, starting_amount: starting, sort_order: 0 },
        payoff: if kind == GoalKind::Payoff { Some(opt_money_field(f, "payoff_amount")?) } else { None },
        new_line,
    })
}

/// Adds a budget line named after a new saving goal to the viewed month,
/// under Saving, unless a line by that name is already there. Returns
/// whether a line was added. The goal is checked first so a bad form
/// doesn't leave a stray line behind.
async fn ensure_goal_line(st: &Shared, user: &UserRecord, view: &View, goal: &Goal) -> AppResult<bool> {
    st.wallet(user).await?.validate_goal(goal)?;
    let GoalTrack::Line { name } = &goal.track else { return Ok(false) };
    let mid = view.month().ok_or(AppError::NotFound)?.clone();
    let name = name.clone();
    let (added, _) = st
        .mutate(user, &mid, move |m| {
            if m.expense_lines.iter().any(|l| l.name.eq_ignore_ascii_case(&name)) {
                return Ok(false);
            }
            let cat = match m.categories.iter().find(|c| c.kind == CategoryKind::Standard && c.name.eq_ignore_ascii_case("Saving")) {
                Some(c) => c.id.clone(),
                None => m.add_category("Saving", CategoryKind::Standard)?,
            };
            m.add_expense_line(&cat, &name)?;
            Ok(true)
        })
        .await?;
    Ok(added)
}

/// Fills in a payoff goal from the debt it starts at: pay off `amount`, or
/// all of it when blank.
fn set_payoff(goal: &mut Goal, debt: Cents, amount: Option<Cents>, currency: &str) -> AppResult<()> {
    if debt.is_zero() {
        return Err(AppError::bad(t("err.goal_no_debt")));
    }
    let target = amount.unwrap_or(debt);
    if target > debt {
        return Err(AppError::bad(tf("err.goal_over_debt", &[("amount", &crate::money::format(debt, currency))])));
    }
    goal.starting_amount = debt;
    goal.target_amount = target;
    Ok(())
}

async fn goal_start(st: &Shared, user: &UserRecord, view: &View) -> chrono::NaiveDate {
    match month_for_view(st, user, view).await {
        Ok(m) => m.year_month,
        Err(_) => st.today_for(user).with_day(1).unwrap_or_else(|| st.today_for(user)),
    }
}

pub async fn add_goal(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    let start = goal_start(&st, &user, &view).await;
    let GoalForm { goal, payoff, new_line } = match goal_from_form(&f, Id::generate(), start) {
        Ok(v) => v,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let added = if new_line {
        match ensure_goal_line(&st, &user, &view, &goal).await {
            Ok(a) => a,
            Err(e) => return failed(&st, &user, &headers, &view, &e).await,
        }
    } else {
        false
    };
    let line_name = goal.name.trim().to_string();
    let currency = user.currency.clone();
    wallet_action(&st, &user, &headers, view, move |w, all| {
        let mut goal = goal;
        if let Some(amount) = payoff {
            let debt = w.debt_now(&goal.track, all, start);
            set_payoff(&mut goal, debt, amount, &currency)?;
        }
        Ok(w.add_goal(goal)?)
    }, move |_, _| {
        let msg = if added { tf("goal.created_line", &[("name", &line_name)]) } else { t("goal.created") };
        vec![toast(ToastKind::Success, &msg, None)]
    })
    .await
}

pub async fn update_goal(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    let old = st.wallet(&user).await.ok().and_then(|w| w.goal(&id).cloned());
    let Some(old) = old else { return failed(&st, &user, &headers, &view, &AppError::NotFound).await };
    let currency = user.currency.clone();
    let GoalForm { goal, payoff, new_line } = match goal_from_form(&f, id, old.start_month) {
        Ok(v) => v,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    if new_line {
        if let Err(e) = ensure_goal_line(&st, &user, &view, &goal).await {
            return failed(&st, &user, &headers, &view, &e).await;
        }
    }
    wallet_action(&st, &user, &headers, view, move |w, all| {
        let mut goal = goal;
        if let Some(amount) = payoff {
            // Keep the starting debt unless the goal now follows another debt.
            let debt = if old.kind == GoalKind::Payoff && old.track == goal.track { old.starting_amount } else { w.debt_now(&goal.track, all, old.start_month) };
            set_payoff(&mut goal, debt, amount, &currency)?;
        }
        Ok(w.update_goal(goal)?)
    }, |_, _| vec![toast(ToastKind::Success, &t("goal.saved_toast"), None)])
    .await
}

pub async fn delete_goal(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, fallback_view(&user));
    wallet_action(&st, &user, &headers, view, move |w, _| Ok(w.delete_goal(&id)?), |_, _| vec![toast(ToastKind::Success, &t("goal.deleted"), None)]).await
}
