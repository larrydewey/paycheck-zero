//! Bottom sheets: everything that isn't needed at a glance lives here, one
//! tap away. Each handler renders into `#sheet-body` over SSE; forms inside
//! post to the regular actions, which close the sheet on success.

use super::pages::{ctx, fund_form, tx_fields};
use super::plan::default_tx_date;
use super::*;
use crate::auth::AuthUser;
use crate::error::AppError;
use crate::sse::Sse;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Extension;
use paycheckzero_core::*;
use paycheckzero_storage::{Owner, UserRecord};
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct SheetQuery {
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    pid: Option<String>,
    #[serde(default)]
    cat: Option<String>,
}

fn fallback(view: &Option<String>, f: View) -> View {
    view.as_deref().and_then(View::decode).unwrap_or(f)
}

fn error_sheet(e: &AppError, currency: &str) -> Sse {
    Sse::new().patch(sheet(&t("error.load_title"), None, html! {
        p { (e.human(currency, None)) }
        details class="tech" { summary { (t("common.technical_details")) } code { (e.technical()) } }
    }))
}

async fn month_of(st: &Shared, user: &UserRecord, kind: Owner, id: &Id) -> Result<(Month, bool), AppError> {
    let mid = st.resolve(user, kind, id).await?;
    let l = st.load(user, &mid).await?;
    Ok((l.month, l.archived))
}

fn stat(label: &str, value: String, class: &str, col: &str) -> Markup {
    html! { div class="sheet-stat" { span class="stat-label" { (label) } span class=(format!("stat-value {class}")) data-col=(col) { (value) } } }
}

// ----------------------------------------------------------------------
// Line
// ----------------------------------------------------------------------

pub async fn line(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(lid): Path<Id>, Query(q): Query<SheetQuery>) -> Sse {
    let user = user.0;
    let (m, archived) = match month_of(&st, &user, Owner::ExpenseLine, &lid).await {
        Ok(v) => v,
        Err(e) => return error_sheet(&e, &user.currency),
    };
    let c = ctx(&st, &user, &headers);
    let view = fallback(&q.view, View::Overview { month: m.id.clone() });
    let Some(l) = m.category_views().into_iter().flat_map(|c| c.lines).find(|l| l.id == lid) else {
        return error_sheet(&AppError::NotFound, &user.currency);
    };
    let focus_pc = q.pid.as_deref().map(Id::new);
    let structure = !m.is_locked() && !archived;
    let alloc = m.allocations_editable() && !archived;
    let cat_name = m.category(&l.category_id).map(|x| x.name.clone()).unwrap_or_default();
    let txs: Vec<&Transaction> = {
        let mut v: Vec<&Transaction> = m.transactions.iter().filter(|t| t.expense_line_id.as_ref() == Some(&lid)).collect();
        v.sort_by_key(|t| std::cmp::Reverse(t.date));
        v
    };
    let over = l.spent > l.planned;
    let wallet = st.wallet(&user).await.unwrap_or_default();
    let all = if wallet.goals_for_line(&l.name).is_empty() { Vec::new() } else { st.all_months(&user).await.unwrap_or_default() };
    Sse::new().patch(sheet(&l.name, Some(&cat_name), html! {
        div class="sheet-stats" {
            (stat(&t("col.planned"), c.money(l.planned), "", "planned"))
            (stat(&t("col.spent"), c.money(l.spent), "", "spent"))
            (stat(&t("col.remaining"), c.money(l.remaining), if l.remaining.is_negative() { "neg" } else { "" }, "remaining"))
        }
        @if over { p class="alert danger" { (icon("alert")) " " (tf("over.line", &[("amount", &c.money(l.spent - l.planned))])) } }
        @if !all.is_empty() { (super::accounts::line_goals(&c, &m, &wallet, &all, &l, &view, alloc)) }

        section class="sheet-section" aria-labelledby="ls-funding" {
            h3 id="ls-funding" { (t("line.funding_title")) }
            ul class="funding-list" {
                @for p in m.paychecks_by_date() {
                    @let this = m.allocation_for(&p.id, &lid).map_or(Cents::ZERO, |a| a.amount);
                    @let free = m.paycheck_unallocated(&p.id);
                    @let pname = m.income_line(&p.income_line_id).map(|x| x.name.clone()).unwrap_or_default();
                    li class=(if focus_pc.as_ref() == Some(&p.id) { "current" } else { "" }) {
                        span class="fund-pc" { strong { (short_date(p.date)) } " · " (pname)
                            @if p.status == PaycheckStatus::Skipped { " · " (t("paycheck.skipped")) }
                            br; span class="muted small" { (tf("line.paycheck_left", &[("amount", &c.money(free))])) } }
                        @if alloc && p.status != PaycheckStatus::Skipped {
                            form data-on:submit__prevent=(post_form_guarded(&format!("/ui/paychecks/{}/lines/{}", p.id, lid))) {
                                (view_input(&view))
                                (money_input(&c, "amount", Some(this), &tf("line.from_paycheck_label", &[("name", &l.name), ("date", &short_date(p.date))]), Some((this + free).get())))
                                span class="field-error" aria-live="polite" {}
                            }
                        } @else {
                            span class="num" { (c.money(this)) }
                        }
                    }
                }
                @if m.paychecks.is_empty() { li class="muted" { (t("overview.no_income_title")) } }
            }
        }

        @if l.is_debt {
            section class="sheet-section" aria-labelledby="ls-debt" {
                h3 id="ls-debt" { (t("line.debt_title")) }
                @if structure {
                    form class="stack" data-on:submit__prevent=(post_form(&format!("/ui/lines/{lid}/debt"))) {
                        (view_input(&view))
                        div class="two-col" {
                            div class="field" {
                                label for="ls-bal" { (t("debt.balance")) }
                                input id="ls-bal" type="text" inputmode="decimal" class="money" name="current_balance" value=[l.current_balance.map(|v| c.money(v))];
                            }
                            div class="field" {
                                label for="ls-min" { (t("debt.minimum")) }
                                input id="ls-min" type="text" inputmode="decimal" class="money" name="minimum_payment" value=[l.minimum_payment.map(|v| c.money(v))];
                            }
                        }
                        button type="submit" class="btn" { (t("common.save")) }
                    }
                } @else {
                    p { (t("debt.balance")) " " (l.current_balance.map(|b| c.money(b)).unwrap_or_else(|| "—".into())) " · "
                        (t("debt.minimum")) " " (l.minimum_payment.map(|b| c.money(b)).unwrap_or_else(|| "—".into())) }
                }
                p { (super::plan::debt_outlook(&c, &l, true)) }
                @if let Some(min) = l.minimum_payment {
                    @if l.planned < min { p class="warn-text" { (tf("debt.below_minimum", &[("amount", &c.money(min - l.planned))])) } }
                }
            }
        }

        section class="sheet-section" aria-labelledby="ls-tx" {
            h3 id="ls-tx" { (t("line.recent_tx")) }
            @if txs.is_empty() { p class="muted small" { (t("line.no_tx")) } }
            ul class="mini-list" {
                @for x in txs.iter().take(8) {
                    li { span { (short_date(x.date)) " · " (x.payee.clone().unwrap_or_else(|| t("tx.no_payee"))) } span class="num" { (c.money(x.amount)) } }
                }
            }
        }

        @if structure {
            section class="sheet-section" aria-labelledby="ls-edit" {
                h3 id="ls-edit" { (t("line.edit_title")) }
                form class="stack" data-signals=(format!("{{_lscat: '{}'}}", l.category_id)) data-on:submit__prevent=(post_form(&format!("/ui/lines/{lid}/edit"))) {
                    (view_input(&view))
                    div class="field" {
                        label for="ls-name" { (t("line.name_field")) }
                        input id="ls-name" type="text" name="name" value=(l.name) required maxlength="100";
                    }
                    div class="field" {
                        label for="ls-cat" { (t("fund.new_category")) }
                        select id="ls-cat" name="category_id" data-bind:_lscat {
                            @for cat in m.categories_sorted() { option value=(cat.id) selected[cat.id == l.category_id] { (cat.name) } }
                        }
                    }
                    // One position list per category; the chosen category's shows.
                    @for cat in m.categories_sorted() {
                        @let others: Vec<(&str, usize)> = m.lines_of(&cat.id).into_iter().filter(|x| x.id != lid).enumerate().map(|(i, x)| (x.name.as_str(), i + 1)).collect();
                        @let current = (cat.id == l.category_id).then(|| m.lines_of(&cat.id).iter().position(|x| x.id == lid).unwrap_or(0));
                        div class="field" data-show=(format!("$_lscat == '{}'", cat.id)) style=[(cat.id != l.category_id).then_some("display: none")] {
                            label for=(format!("ls-pos-{}", cat.id)) { (t("position.label")) }
                            (position_select(&format!("ls-pos-{}", cat.id), &format!("pos_{}", cat.id), &others, current))
                        }
                    }
                    button type="submit" class="btn primary" { (t("common.save")) }
                }
                div class="sheet-actions" {
                    form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/lines/{lid}/move"))) {
                        (view_input(&view)) input type="hidden" name="direction" value="up";
                        button type="submit" class="btn small" aria-label=(tf("line.move_up", &[("name", &l.name)])) { (icon("up")) " " (t("common.move_up")) }
                    }
                    form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/lines/{lid}/move"))) {
                        (view_input(&view)) input type="hidden" name="direction" value="down";
                        button type="submit" class="btn small" aria-label=(tf("line.move_down", &[("name", &l.name)])) { (icon("down")) " " (t("common.move_down")) }
                    }
                    form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/lines/{lid}/delete"))) {
                        (view_input(&view))
                        button type="submit" class="btn small danger" aria-label=(tf("line.delete", &[("name", &l.name)])) data-confirm=(tf("line.delete_confirm", &[("name", &l.name)])) {
                            (icon("trash")) " " (t("common.delete"))
                        }
                    }
                }
            }
        }
    }))
}

// ----------------------------------------------------------------------
// Assign money from a paycheck
// ----------------------------------------------------------------------

pub async fn assign(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Query(q): Query<SheetQuery>) -> Sse {
    let user = user.0;
    let (m, archived) = match month_of(&st, &user, Owner::Paycheck, &pid).await {
        Ok(v) => v,
        Err(e) => return error_sheet(&e, &user.currency),
    };
    let c = ctx(&st, &user, &headers);
    let view = fallback(&q.view, View::Paycheck { month: m.id.clone(), paycheck: pid.clone() });
    let Some(p) = m.paycheck(&pid) else { return error_sheet(&AppError::NotFound, &user.currency) };
    let unallocated = m.paycheck_unallocated(&pid);
    let editable = m.allocations_editable() && !archived && p.status != PaycheckStatus::Skipped;
    let structure = !m.is_locked() && !archived;
    let tithe = m.tithe_amount(&pid, 10);
    let current_tithe = m.categories.iter().find(|x| x.name.eq_ignore_ascii_case("Giving"))
        .and_then(|x| m.lines_of(&x.id).into_iter().find(|l| l.name.eq_ignore_ascii_case("Tithe")))
        .and_then(|l| m.allocation_for(&pid, &l.id)).map_or(Cents::ZERO, |a| a.amount);
    let targets: Vec<&ExpenseLine> = m.expense_lines.iter().filter(|l| m.line_unfunded_target(&l.id).is_positive()).collect();
    let fund_url = format!("/ui/months/{}/fund", m.id);
    Sse::new().patch(sheet(
        &tf("assign.title", &[("date", &short_date(p.date))]),
        Some(&tf("fund.available", &[("amount", &c.money(unallocated))])),
        html! {
            @if !editable || !unallocated.is_positive() {
                p class="muted" { (t("paycheck.all_assigned")) " " (t("paycheck.all_assigned_hint")) }
            } @else {
                div class="quick-actions" {
                    @if current_tithe == tithe {
                        span class="pill ok" { (icon("check")) " " (tf("give.done", &[("amount", &c.money(tithe))])) }
                    } @else if tithe.is_positive() {
                        form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/give"))) {
                            (view_input(&view))
                            button type="submit" class="btn" { (tf("give.button", &[("amount", &c.money(tithe))])) }
                        }
                    }
                }
                @if !targets.is_empty() {
                    section class="targets sheet-section" aria-labelledby="targets-h" {
                        h3 id="targets-h" { (t("targets.title")) }
                        ul {
                            @for l in &targets {
                                @let unfunded = m.line_unfunded_target(&l.id);
                                @let amount = unfunded.min(unallocated);
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
                (fund_form(&c, &m, &pid, &view, unallocated, structure))
            }
        },
    ))
}

// ----------------------------------------------------------------------
// Paycheck details
// ----------------------------------------------------------------------

pub async fn paycheck(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(pid): Path<Id>, Query(q): Query<SheetQuery>) -> Sse {
    let user = user.0;
    let (m, archived) = match month_of(&st, &user, Owner::Paycheck, &pid).await {
        Ok(v) => v,
        Err(e) => return error_sheet(&e, &user.currency),
    };
    let c = ctx(&st, &user, &headers);
    let view = fallback(&q.view, View::Paycheck { month: m.id.clone(), paycheck: pid.clone() });
    let Some(p) = m.paycheck(&pid) else { return error_sheet(&AppError::NotFound, &user.currency) };
    let structure = !m.is_locked() && !archived;
    let name = m.income_line(&p.income_line_id).map(|l| l.name.clone()).unwrap_or_default();
    Sse::new().patch(sheet(&tf("paycheck.sheet_title", &[("date", &short_date(p.date))]), Some(&name), html! {
        div class="sheet-stats" {
            (stat(&t("paycheck.planned"), c.money(p.planned_amount), "", "planned"))
            (stat(&t("col.actual"), p.actual_amount.map_or_else(|| "—".into(), |a| c.money(a)), "", "actual"))
            (stat(&t("col.status"), t(&format!("status.{}", p.status.as_str())), "", "status"))
        }
        @if !archived && p.status != PaycheckStatus::Skipped {
            (actual_form(&c, &m, p, &view))
        }
        @if structure {
            form class="stack" data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/planned"))) {
                (view_input(&view))
                label for="pc-planned" { (t("paycheck.planned_amount")) }
                div class="inline-field" {
                    input id="pc-planned" type="text" inputmode="decimal" class="money" name="amount" value=(c.money(p.planned_amount)) required;
                    button type="submit" class="btn" { (t("common.save")) }
                }
            }
            div class="sheet-actions" {
                form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/status"))) {
                    (view_input(&view))
                    @if p.status == PaycheckStatus::Skipped {
                        input type="hidden" name="status" value="planned";
                        button type="submit" class="btn small" { (t("paycheck.unskip")) }
                    } @else {
                        input type="hidden" name="status" value="skipped";
                        button type="submit" class="btn small" data-confirm=(t("paycheck.skip_confirm")) { (t("paycheck.skip")) }
                    }
                }
                form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/delete"))) {
                    (view_input(&view))
                    button type="submit" class="btn small danger" data-confirm=(t("paycheck.delete_confirm")) { (icon("trash")) " " (t("paycheck.delete")) }
                }
            }
        }
        p class="page-links" { a href=(format!("/months/{}/income", m.id)) { (t("plan.manage_income")) } }
    }))
}

// ----------------------------------------------------------------------
// Transactions
// ----------------------------------------------------------------------

pub async fn tx_new(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(mid): Path<Id>, Query(q): Query<SheetQuery>) -> Sse {
    let user = user.0;
    let loaded = match st.load(&user, &mid).await {
        Ok(l) => l,
        Err(e) => return error_sheet(&e, &user.currency),
    };
    let m = loaded.month;
    let c = ctx(&st, &user, &headers);
    let view = fallback(&q.view, View::Transactions { month: mid.clone(), filter: TxFilter::All });
    let wallet = st.wallet(&user).await.unwrap_or_default();
    Sse::new().patch(sheet(&t("tx.add"), None, tx_new_form(&c, &m, &view, "new-tx", &wallet)))
}

fn tx_new_form(c: &Ctx, m: &Month, view: &View, prefix: &str, wallet: &Wallet) -> Markup {
    // Default the paycheck to the one being viewed.
    let default_pc = match view { View::Paycheck { paycheck, .. } => Some(paycheck.clone()), _ => None };
    let date = default_tx_date(c, m);
    let transfers = wallet.accounts_sorted().len() >= 2;
    html! {
        @if transfers {
            p class="sheet-switch" {
                button type="button" class="link" data-online-only-link
                    data-on:click=(super::open_sheet(&format!("/ui/sheet/transfer/new/{}?view={}", m.id, view.encode()))) {
                    (icon("transfer")) " " (t("transfer.instead"))
                }
            }
        }
        form id="add-tx" class="tx-form" data-clear data-offline="create_transaction" data-month=(m.id)
            data-on:submit__prevent=(format!("pz.checkSplit(el) && @post('/ui/months/{}/transactions', {{contentType: 'form'}})", m.id)) {
            (view_input(view))
            (tx_fields(c, m, prefix, &[], date, default_pc.as_ref(), wallet))
            div class="form-actions wide" {
                button type="submit" class="btn primary block" { (t("tx.save")) }
            }
        }
    }
}

/// Offline copies of the sheets that work without a connection (adding or
/// editing a transaction, recording what a paycheck actually paid). They
/// ride along with each month page, so the service worker's cached page
/// can still open them; `app.js` uses them only while offline.
pub(super) fn offline_templates(c: &Ctx, m: &Month, view: &View, wallet: &Wallet) -> Markup {
    html! {
        template id="offline-tx" {
            (sheet(&t("tx.add"), None, tx_new_form(c, m, view, "off-tx", wallet)))
        }
        @if let View::Paycheck { paycheck, .. } = view {
            @if let Some(p) = m.paycheck(paycheck).filter(|p| p.status != PaycheckStatus::Skipped) {
                @let name = m.income_line(&p.income_line_id).map(|l| l.name.clone()).unwrap_or_default();
                template id="offline-paycheck" data-paycheck=(p.id) {
                    (sheet(&tf("paycheck.sheet_title", &[("date", &short_date(p.date))]), Some(&name), actual_form(c, m, p, view)))
                }
            }
        }
    }
}

fn actual_form(c: &Ctx, m: &Month, p: &Paycheck, view: &View) -> Markup {
    let pid = &p.id;
    html! {
        form class="stack" data-offline="actual" data-month=(m.id) data-paycheck=(pid)
            data-base-actual=[p.actual_amount.map(Cents::get)]
            data-on:submit__prevent=(post_form(&format!("/ui/paychecks/{pid}/actual"))) {
            (view_input(view))
            label for="pc-actual" { (t("paycheck.actual_amount")) }
            div class="inline-field" {
                input id="pc-actual" type="text" inputmode="decimal" class="money" name="amount" value=[p.actual_amount.map(|v| c.money(v))] placeholder=(t("paycheck.actual_placeholder"));
                button type="submit" class="btn primary" { (t("paycheck.record_actual")) }
            }
            p class="hint" { (t("paycheck.actual_hint")) }
        }
    }
}

pub async fn tx_edit(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(tid): Path<Id>, Query(q): Query<SheetQuery>) -> Sse {
    let user = user.0;
    let (m, _) = match month_of(&st, &user, Owner::Transaction, &tid).await {
        Ok(v) => v,
        Err(e) => return error_sheet(&e, &user.currency),
    };
    let c = ctx(&st, &user, &headers);
    let view = fallback(&q.view, View::Transactions { month: m.id.clone(), filter: TxFilter::All });
    let Some(x) = m.transaction(&tid) else { return error_sheet(&AppError::NotFound, &user.currency) };
    let wallet = st.wallet(&user).await.unwrap_or_default();
    if x.is_transfer() {
        return Sse::new().patch(sheet(&t("transfer.edit_title"), None, super::accounts::transfer_form(&c, &m, &view, &wallet, Some(x), &super::accounts::TransferPrefill::default())));
    }
    let parts: Vec<&Transaction> = match &x.split_group { Some(g) => m.split_parts(g), None => vec![x] };
    let split = parts.len() > 1;
    let base = serde_json::json!({
        "date": x.date, "amount": x.amount.get(), "payee": x.payee, "notes": x.notes,
        "expense_line_id": x.expense_line_id, "paycheck_id": x.paycheck_id, "account_id": x.account_id,
    }).to_string();
    let first_id = parts[0].id.clone();
    let date = default_tx_date(&c, &m);
    let months = st.store.list_months(&user.id, false).await.unwrap_or_default();
    let more = tx_more(&m, x, split, &view, &months, &wallet);
    Sse::new().patch(sheet(&t(if split { "split.edit_title" } else { "tx.edit_title" }), None, html! {
        @if split {
            form class="tx-form" data-online-only
                data-on:submit__prevent=(format!("pz.checkSplit(el) && @post('/ui/transactions/{first_id}', {{contentType: 'form'}})")) {
                (view_input(&view))
                (tx_fields(&c, &m, &format!("split-{first_id}"), &parts, date, None, &wallet))
                div class="form-actions wide" { button type="submit" class="btn primary block" { (t("common.save")) } }
            }
            form class="dialog-danger" data-on:submit__prevent=(post_form(&format!("/ui/splits/{}/delete", x.split_group.clone().unwrap_or_default()))) {
                (view_input(&view))
                input type="hidden" name="part_of" value=(first_id);
                button type="submit" class="btn small danger" data-confirm=(t("split.delete_confirm")) { (icon("trash")) " " (t("split.delete")) }
            }
            (more)
        } @else {
            form class="tx-form" data-offline="update_transaction" data-month=(m.id) data-tx=(x.id) data-base=(base.clone())
                data-on:submit__prevent=(format!("pz.checkSplit(el) && @post('/ui/transactions/{}', {{contentType: 'form'}})", x.id)) {
                (view_input(&view))
                (tx_fields(&c, &m, &format!("tx-{}", x.id), &[x], date, None, &wallet))
                div class="form-actions wide" { button type="submit" class="btn primary block" { (t("common.save")) } }
            }
            form class="dialog-danger" data-offline="delete_transaction" data-month=(m.id) data-tx=(x.id) data-base=(base)
                data-on:submit__prevent=(post_form(&format!("/ui/transactions/{}/delete", x.id))) {
                (view_input(&view))
                button type="submit" class="btn small danger" data-confirm=(t("tx.delete_confirm")) { (icon("trash")) " " (t("tx.delete")) }
            }
            (more)
        }
    }))
}

/// "More" in the edit sheet: count the transaction in another month, or
/// mark a payment as money moved to another account (paying a card).
fn tx_more(m: &Month, x: &Transaction, split: bool, view: &View, months: &[paycheckzero_storage::MonthMeta], wallet: &Wallet) -> Markup {
    let can_transfer = !split && x.amount.is_negative() && wallet.accounts_sorted().len() >= 2;
    html! {
        details class="tx-more" {
            summary { (t("tx.more")) }
            form class="stack" data-online-only data-on:submit__prevent=(post_form(&format!("/ui/transactions/{}/move", x.id))) {
                (view_input(view))
                label for="tx-move-month" { (t("tx.count_in")) }
                div class="inline-field" {
                    select id="tx-move-month" name="month_id" {
                        @for mm in months.iter().rev() {
                            option value=(mm.id) selected[mm.id == m.id] { (month_label(mm.year_month)) }
                        }
                    }
                    button type="submit" class="btn" { (t("tx.move")) }
                }
                p class="hint" { (t("tx.count_in_hint")) }
            }
            @if can_transfer {
                form class="stack" data-online-only data-on:submit__prevent=(post_form(&format!("/ui/transactions/{}/transfer", x.id))) {
                    (view_input(view))
                    @if x.account_id.is_none() {
                        label for="tx-xfer-from" { (t("transfer.from")) }
                        select id="tx-xfer-from" name="from" required { (super::pages::account_options(wallet, None, false)) }
                    }
                    label for="tx-xfer-to" { (t("tx.payment_to")) }
                    div class="inline-field" {
                        select id="tx-xfer-to" name="to" required {
                            option value="" { (t("tx.payment_to_pick")) }
                            (super::pages::account_options(wallet, None, false))
                        }
                        button type="submit" class="btn" { (t("tx.mark_transfer")) }
                    }
                    p class="hint" { (t("tx.mark_transfer_hint")) }
                }
            }
        }
    }
}

// ----------------------------------------------------------------------
// Category and new line
// ----------------------------------------------------------------------

/// A Datastar expression: is the category id in `signal` a Debt category?
pub(super) fn debt_category_check(m: &Month, signal: &str) -> String {
    let ids: Vec<String> = m.categories.iter().filter(|x| x.kind == CategoryKind::Debt).map(|x| format!("'{}'", x.id)).collect();
    format!("[{}].includes({signal})", ids.join(","))
}

/// Balance owed and minimum payment for a new debt line, shown while `show`
/// holds. They're kept apart from the paycheck amount so the total debt is
/// never mistaken for what a paycheck puts toward it.
pub(super) fn debt_fields(m: &Month, prefix: &str, show: &str) -> Markup {
    html! {
        @if m.categories.iter().any(|x| x.kind == CategoryKind::Debt) {
            div class="two-col" data-show=(show) style="display: none" {
                div class="field" {
                    label for=(format!("{prefix}-bal")) { (t("newline.balance")) }
                    input id=(format!("{prefix}-bal")) type="text" inputmode="decimal" class="money" name="current_balance" placeholder="0.00" autocomplete="off";
                }
                div class="field" {
                    label for=(format!("{prefix}-min")) { (t("debt.minimum")) }
                    input id=(format!("{prefix}-min")) type="text" inputmode="decimal" class="money" name="minimum_payment" placeholder="0.00" autocomplete="off";
                }
            }
            p class="hint" data-show=(show) style="display: none" { (t("newline.balance_hint")) }
        }
    }
}

/// Where to put an item: "At the top" or "After X", for each `(X, index)`
/// in `after`, where index is where the item lands among the rest. The
/// choice nearest at or before `current` (its index now) is selected, or the
/// last when it isn't in this list yet.
fn position_select(id: &str, name: &str, after: &[(&str, usize)], current: Option<usize>) -> Markup {
    let chosen = match current {
        Some(cur) => after.iter().map(|(_, i)| *i).filter(|i| *i <= cur).max().unwrap_or(0),
        None => after.last().map_or(0, |(_, i)| *i),
    };
    html! {
        select id=(id) name=(name) {
            option value="0" selected[chosen == 0] { (t("position.top")) }
            @for (o, i) in after {
                option value=(i) selected[chosen == *i] { (tf("position.after", &[("name", o)])) }
            }
        }
    }
}

pub async fn category(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(cid): Path<Id>, Query(q): Query<SheetQuery>) -> Sse {
    let user = user.0;
    let (m, _) = match month_of(&st, &user, Owner::Category, &cid).await {
        Ok(v) => v,
        Err(e) => return error_sheet(&e, &user.currency),
    };
    let _ = ctx(&st, &user, &headers);
    let view = fallback(&q.view, View::Overview { month: m.id.clone() });
    let Some(cat) = m.category(&cid).cloned() else { return error_sheet(&AppError::NotFound, &user.currency) };
    Sse::new().patch(sheet(&cat.name, Some(&t("category.sheet_sub")), html! {
        form class="stack" data-on:submit__prevent=(post_form(&format!("/ui/categories/{cid}/rename"))) {
            (view_input(&view))
            label for="cs-name" { (t("category.name_field")) }
            div class="inline-field" {
                input id="cs-name" type="text" name="name" value=(cat.name) required maxlength="100";
                button type="submit" class="btn primary" { (t("common.save")) }
            }
        }
        @let cats = m.categories_sorted();
        // Only categories with lines are in the main list, so only they are
        // offered; the index still counts every category.
        @let others: Vec<(&str, usize)> = cats.iter().filter(|x| x.id != cid).enumerate()
            .filter(|(_, x)| !m.lines_of(&x.id).is_empty()).map(|(i, x)| (x.name.as_str(), i + 1)).collect();
        form class="stack" data-on:submit__prevent=(post_form(&format!("/ui/months/{}/place", m.id))) {
            (view_input(&view))
            input type="hidden" name="kind" value="category";
            input type="hidden" name="id" value=(cid);
            input type="hidden" name="category_id" value=(cid);
            label for="cs-pos" { (t("position.label")) }
            div class="inline-field" {
                (position_select("cs-pos", "index", &others, cats.iter().position(|x| x.id == cid)))
                button type="submit" class="btn" { (t("position.move")) }
            }
        }
        div class="sheet-actions" {
            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/categories/{cid}/move"))) {
                (view_input(&view)) input type="hidden" name="direction" value="up";
                button type="submit" class="btn small" aria-label=(tf("category.move_up", &[("name", &cat.name)])) { (icon("up")) " " (t("common.move_up")) }
            }
            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/categories/{cid}/move"))) {
                (view_input(&view)) input type="hidden" name="direction" value="down";
                button type="submit" class="btn small" aria-label=(tf("category.move_down", &[("name", &cat.name)])) { (icon("down")) " " (t("common.move_down")) }
            }
            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/categories/{cid}/delete"))) {
                (view_input(&view))
                button type="submit" class="btn small danger" aria-label=(tf("category.delete", &[("name", &cat.name)])) data-confirm=(tf("category.delete_confirm", &[("name", &cat.name)])) {
                    (icon("trash")) " " (t("common.delete"))
                }
            }
        }
    }))
}

pub async fn new_line(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(mid): Path<Id>, Query(q): Query<SheetQuery>) -> Sse {
    let user = user.0;
    let loaded = match st.load(&user, &mid).await {
        Ok(l) => l,
        Err(e) => return error_sheet(&e, &user.currency),
    };
    let m = loaded.month;
    let c = ctx(&st, &user, &headers);
    let view = fallback(&q.view, View::Overview { month: mid.clone() });
    let pid = q.pid.as_deref().map(Id::new).filter(|p| m.paycheck(p).is_some());
    let free = pid.as_ref().map(|p| m.paycheck_unallocated(p));
    let chosen = q.cat.as_deref().map(Id::new).filter(|x| m.category(x).is_some());
    let cats = m.categories_sorted();
    let initial_cat = chosen.clone().or_else(|| cats.first().map(|x| x.id.clone())).map(|x| x.to_string()).unwrap_or_else(|| "__new".into());
    // Debt lines carry a balance owed, kept apart from what a paycheck pays
    // toward it, so the total debt is never mistaken for this month's plan.
    let is_debt = debt_category_check(&m, "$_nlcat");
    Sse::new().patch(sheet(&t("plan.new_line"), pid.as_ref().and_then(|p| m.paycheck(p)).map(|p| tf("newline.sub", &[("date", &short_date(p.date))])).as_deref(), html! {
        form class="stack" id="new-line-form" data-signals=(format!("{{_nlcat: '{}'}}", initial_cat))
            data-on:submit__prevent=(post_form_guarded(&format!("/ui/months/{mid}/lines"))) {
            (view_input(&view))
            @if let Some(p) = &pid { input type="hidden" name="paycheck_id" value=(p); }
            div class="field" {
                label for="nl-name" { (t("line.name_field")) }
                input id="nl-name" type="text" name="name" required maxlength="100" autocomplete="off";
            }
            div class="field" {
                label for="nl-cat" { (t("fund.new_category")) }
                select id="nl-cat" name="category_id" data-bind:_nlcat {
                    @for cat in &cats { option value=(cat.id) selected[initial_cat == cat.id.to_string()] { (cat.name) } }
                    option value="__new" selected[initial_cat == "__new"] { (t("newline.new_category")) }
                }
            }
            div class="field" data-show="$_nlcat == '__new'" {
                label for="nl-newcat" { (t("category.name_field")) }
                input id="nl-newcat" type="text" name="new_category" maxlength="100";
            }
            (debt_fields(&m, "nl", &is_debt))
            @if let Some(f) = free {
                div class="field" {
                    label for="nl-amount" {
                        span data-show=(format!("!{is_debt}")) { (t("newline.amount")) }
                        span data-show=(is_debt) style="display: none" { (t("newline.payment")) }
                    }
                    input id="nl-amount" type="text" inputmode="decimal" class="money" name="amount" placeholder="0.00" autocomplete="off" data-max-cents=(f.get());
                    p class="hint" { (tf("fund.available", &[("amount", &c.money(f))])) }
                }
            }
            @if pid.is_none() && !m.paychecks.is_empty() && m.allocations_editable() {
                @let first_free = m.paychecks_by_date().into_iter().find(|p| m.paycheck_unallocated(&p.id).is_positive()).map(|p| p.id.clone());
                div class="two-col" {
                    div class="field" {
                        label for="nl-amount" { (t("newline.amount_any")) }
                        input id="nl-amount" type="text" inputmode="decimal" class="money" name="amount" placeholder="0.00" autocomplete="off";
                    }
                    div class="field" {
                        label for="nl-pc" { (t("newline.from")) }
                        select id="nl-pc" name="paycheck_id" {
                            @for p in m.paychecks_by_date() {
                                option value=(p.id) selected[first_free.as_ref() == Some(&p.id)] {
                                    (tf("newline.from_option", &[("date", &short_date(p.date)), ("amount", &c.money(m.paycheck_unallocated(&p.id)))]))
                                }
                            }
                        }
                    }
                }
                p class="hint" { (t("newline.optional")) }
            }
            span class="field-error" aria-live="polite" {}
            button type="submit" class="btn primary block" { (t("line.add")) }
        }
    }))
}
