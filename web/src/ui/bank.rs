//! Bank connections (Teller): connect, choose accounts, sync, reconnect,
//! disconnect. The sync itself lives in `crate::bank`.

use super::actions::{done, failed, field, view_of, F};
use super::pages::ctx;
use super::*;
use crate::auth::AuthUser;
use crate::bank::{remote_accounts, seal, sync_link, RemoteAccount, SyncReport};
use crate::error::{AppError, AppResult};
use crate::sse::Sse;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::{Extension, Form};
use chrono::Datelike;
use paycheckzero_core::*;
use paycheckzero_storage::UserRecord;
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct BankQuery {
    #[serde(default)]
    view: Option<String>,
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

/// "Sep 10, 3:04 PM" in the user's timezone.
pub fn when(c: &Ctx, rfc3339: &str) -> String {
    let tz: chrono_tz::Tz = c.user.timezone.parse().unwrap_or(chrono_tz::UTC);
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|d| d.with_timezone(&tz).format("%b %-d, %-I:%M %p").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}

/// The button that opens Teller Connect (new connection, or reconnecting
/// `enrollment` when given). `app.js` loads the script and posts the result
/// through the `#teller-enroll` form.
fn connect_button(cfg: &crate::config::TellerConfig, label: &str, enrollment: Option<&str>, class: &str) -> Markup {
    html! {
        button type="button" class=(class) data-teller-connect
            data-app-id=(cfg.app_id) data-environment=(cfg.environment) data-src=(cfg.connect_js)
            data-enrollment=[enrollment] { (icon("bank")) " " (label) }
    }
}

/// Hidden form Teller Connect's result is posted through.
fn enroll_form(view: &View) -> Markup {
    html! {
        form id="teller-enroll" hidden data-on:submit__prevent=(post_form("/ui/bank/enroll")) {
            (view_input(view))
            input type="hidden" name="access_token";
            input type="hidden" name="enrollment_id";
            input type="hidden" name="institution";
            input type="hidden" name="import_from" data-from;
        }
    }
}

/// Hidden form Plaid Link's result is posted through.
fn plaid_form(view: &View, reconnect: Option<&Id>) -> Markup {
    html! {
        form id="plaid-enroll" hidden data-on:submit__prevent=(post_form("/ui/bank/plaid/enroll")) {
            (view_input(view))
            input type="hidden" name="public_token";
            input type="hidden" name="institution";
            input type="hidden" name="import_from" data-from;
            @if let Some(id) = reconnect { input type="hidden" name="link_id" value=(id); }
        }
    }
}

fn plaid_button(cfg: &crate::config::PlaidConfig, token: &str, label: &str, class: &str) -> Markup {
    html! {
        button type="button" class=(class) data-plaid-connect data-token=(token) data-src=(cfg.link_js) { (icon("bank")) " " (label) }
    }
}

/// Paste-a-token form for SimpleFIN (new, or reconnecting `link`).
fn simplefin_form(view: &View, reconnect: Option<&Id>) -> Markup {
    html! {
        form class="stack" id="simplefin-form" data-on:submit__prevent=(post_form("/ui/bank/simplefin")) {
            (view_input(view))
            input type="hidden" name="import_from" data-from;
            @if let Some(id) = reconnect { input type="hidden" name="link_id" value=(id); }
            label for="sf-token" { (t("bank.sf_token")) }
            textarea id="sf-token" name="setup_token" rows="3" required autocomplete="off" spellcheck="false" placeholder=(t("bank.sf_token_placeholder")) {}
            p class="hint" { (t("bank.sf_hint")) " " a href="https://bridge.simplefin.org" target="_blank" rel="noopener" { "bridge.simplefin.org" } }
            button type="submit" class="btn primary" { (t("bank.sf_connect")) }
        }
    }
}

pub async fn connect_sheet(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Query(q): Query<BankQuery>) -> Sse {
    let user = user.0;
    let c = ctx(&st, &user, &headers);
    let view = view_or(&q.view, &user);
    let first = c.today.with_day(1).unwrap_or(c.today);
    // A Plaid Link token is made per connection attempt.
    let plaid_token = match st.plaid() {
        Ok(p) => p.link_token(user.id.as_str(), None).await.ok(),
        Err(_) => None,
    };
    Sse::new().patch(sheet(&t("bank.connect_title"), Some(&t("bank.connect_sub")), html! {
        ul class="bank-points" {
            li { (icon("lock")) " " (t("bank.point_login")) }
            li { (icon("check")) " " (t("bank.point_read_only")) }
            li { (icon("transfer")) " " (t("bank.point_sorting")) }
        }
        div class="field" {
            label for="bank-from" { (t("bank.import_from")) }
            input id="bank-from" type="date" value=(first.to_string()) required;
            p class="hint" { (t("bank.import_from_hint")) }
        }
        h3 class="provider-h" { (t("bank.choose_provider")) }
        div class="providers" {
            @if let Some(cfg) = st.cfg.teller.as_ref() {
                section class="provider" aria-labelledby="pv-teller" {
                    h4 id="pv-teller" { "Teller" }
                    p class="small muted" { (t("bank.pv_teller")) }
                    (enroll_form(&view))
                    (connect_button(cfg, &t("bank.continue_teller"), None, "btn primary block"))
                }
            }
            @if let (Some(cfg), Some(token)) = (st.cfg.plaid.as_ref(), plaid_token.as_deref()) {
                section class="provider" aria-labelledby="pv-plaid" {
                    h4 id="pv-plaid" { "Plaid" }
                    p class="small muted" { (t("bank.pv_plaid")) }
                    (plaid_form(&view, None))
                    (plaid_button(cfg, token, &t("bank.continue_plaid"), "btn primary block"))
                }
            }
            section class="provider" aria-labelledby="pv-sf" {
                h4 id="pv-sf" { "SimpleFIN Bridge" }
                p class="small muted" { (t("bank.pv_simplefin")) }
                details class="more-options" open[st.cfg.teller.is_none() && st.cfg.plaid.is_none()] {
                    summary { (t("bank.sf_open")) }
                    (simplefin_form(&view, None))
                }
            }
        }
    }))
}

fn compatible(local: &Account, remote: &RemoteAccount) -> bool {
    local.kind.group() == remote.kind.group()
}

fn map_body(link: &BankLink, remote: &[RemoteAccount], w: &Wallet, view: &View) -> Markup {
    html! {
        form class="stack" id="bank-map" data-on:submit__prevent=(post_form(&format!("/ui/bank/{}/map", link.id))) {
            (view_input(view))
            @for ra in remote {
                @let current = w.accounts.iter().find(|a| a.external_id.as_deref() == Some(ra.id.as_str()) && a.link_id.as_ref() == Some(&link.id));
                div class="field map-row" {
                    label for=(format!("map-{}", ra.id)) {
                        strong { (ra.label()) } " " span class="muted small" { (t(&format!("accounts.kind_{}", ra.kind.as_str()))) }
                    }
                    select id=(format!("map-{}", ra.id)) name=(format!("map_{}", ra.id)) {
                        option value="new" selected[current.is_none()] { (tf("bank.map_new_as", &[("kind", &t(&format!("accounts.kind_{}", ra.kind.as_str())))])) }
                        @for a in w.accounts_sorted().into_iter().filter(|a| compatible(a, ra) && (a.link_id.is_none() || current.is_some_and(|c| c.id == a.id))) {
                            option value=(a.id) selected[current.is_some_and(|c| c.id == a.id)] { (tf("bank.map_use", &[("name", &a.name)])) }
                        }
                        option value="skip" { (t("bank.map_skip")) }
                    }
                    @if current.is_none() {
                        details class="kind-override" {
                            summary class="small" { (t("bank.map_change_kind")) }
                            label class="visually-hidden" for=(format!("kind-{}", ra.id)) { (tf("bank.map_kind_label", &[("name", &ra.label())])) }
                            select id=(format!("kind-{}", ra.id)) name=(format!("kind_{}", ra.id)) {
                                @for k in AccountKind::ALL {
                                    option value=(k.as_str()) selected[k == ra.kind] { (t(&format!("accounts.kind_{}", k.as_str()))) }
                                }
                            }
                        }
                    }
                }
            }
            p class="hint" { (t("bank.map_hint")) }
            button type="submit" class="btn primary block" { (t("bank.map_submit")) }
        }
    }
}

fn map_sheet(link: &BankLink, remote: &[RemoteAccount], w: &Wallet, view: &View) -> Markup {
    sheet(&tf("bank.map_title", &[("bank", &link.institution)]), Some(&t("bank.map_sub")), map_body(link, remote, w, view))
}

pub async fn map_sheet_handler(State(st): State<Shared>, Extension(user): Extension<AuthUser>, Path(id): Path<Id>, Query(q): Query<BankQuery>) -> Sse {
    let user = user.0;
    let view = view_or(&q.view, &user);
    let w = match st.wallet(&user).await {
        Ok(w) => w,
        Err(e) => return sheet_error(&e, &user.currency),
    };
    let Some(link) = w.link(&id).cloned() else { return sheet_error(&AppError::NotFound, &user.currency) };
    match remote_accounts(&st, &user, &link).await {
        Ok(remote) => Sse::new().patch(map_sheet(&link, &remote, &w, &view)),
        Err(e) => sheet_error(&e, &user.currency),
    }
}

pub async fn link_sheet(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Query(q): Query<BankQuery>) -> Sse {
    let user = user.0;
    let c = ctx(&st, &user, &headers);
    let view = view_or(&q.view, &user);
    let (w, all) = match (st.wallet(&user).await, st.all_months(&user).await) {
        (Ok(w), Ok(a)) => (w, a),
        (Err(e), _) | (_, Err(e)) => return sheet_error(&e, &user.currency),
    };
    let Some(link) = w.link(&id).cloned() else { return sheet_error(&AppError::NotFound, &user.currency) };
    let accts = w.linked_accounts(&id);
    let enc = view.encode();
    // Plaid reconnects through Link "update mode", which needs its own token.
    let plaid_update = if link.provider == crate::bank::PLAID && link.status != LinkStatus::Active {
        match (st.plaid(), crate::bank::open(&st.cfg.data_key, &link.access_token)) {
            (Ok(p), Some(access)) => p.link_token(user.id.as_str(), Some(&access)).await.ok(),
            _ => None,
        }
    } else {
        None
    };
    Sse::new().patch(sheet(&link.institution, Some(&tf("bank.via", &[("provider", crate::bank::provider_name(&link.provider))])), html! {
        (status_line(&c, &link))
        @if link.status != LinkStatus::Active {
            @match link.provider.as_str() {
                crate::bank::SIMPLEFIN => {
                    p class="small" { (t("bank.sf_reconnect")) }
                    (simplefin_form(&view, Some(&link.id)))
                },
                crate::bank::PLAID => {
                    @if let (Some(cfg), Some(token)) = (st.cfg.plaid.as_ref(), plaid_update.as_deref()) {
                        (plaid_form(&view, Some(&link.id)))
                        (plaid_button(cfg, token, &t("bank.reconnect"), "btn primary block"))
                    }
                },
                _ => {
                    @if let Some(cfg) = st.cfg.teller.as_ref() {
                        (enroll_form(&view))
                        (connect_button(cfg, &t("bank.reconnect"), Some(&link.enrollment_id), "btn primary block"))
                    }
                },
            }
        } @else {
            form data-on:submit__prevent=(post_form(&format!("/ui/bank/{id}/sync"))) {
                (view_input(&view))
                button type="submit" class="btn primary block" { (t("bank.sync_now")) }
            }
        }
        section class="sheet-section" aria-labelledby="bl-acc" {
            h3 id="bl-acc" { (t("bank.accounts")) }
            @if accts.is_empty() { p class="muted small" { (t("bank.no_accounts")) } }
            ul class="mini-list" {
                @for a in &accts {
                    li { span { (a.name) }
                        span class="num" { @if a.kind.is_card() { "−" (c.money(w.owed(&a.id, &all))) } @else { (c.money(w.balance(&a.id, &all))) } } }
                }
            }
            button type="button" class="btn small" data-on:click=(open_sheet(&format!("/ui/sheet/bank/{id}/map?view={enc}"))) { (t("bank.choose_accounts")) }
        }
        p class="small muted" { (tf("bank.import_since", &[("date", &link.import_from.format("%b %-d, %Y").to_string())])) }
        div class="sheet-actions" {
            form class="inline" data-on:submit__prevent=(post_form(&format!("/ui/bank/{id}/delete"))) {
                (view_input(&view))
                button type="submit" class="btn small danger" data-confirm=(tf("bank.disconnect_confirm", &[("bank", &link.institution)])) { (t("bank.disconnect")) }
            }
        }
    }))
}

pub fn status_line(c: &Ctx, link: &BankLink) -> Markup {
    html! {
        @match link.status {
            LinkStatus::Active => {
                p class="small muted" data-link-status="active" {
                    @match &link.last_sync {
                        Some(s) => { (tf("bank.synced_at", &[("when", &when(c, s))])) },
                        None => { (t("bank.never_synced")) },
                    }
                }
            },
            LinkStatus::NeedsReconnect => { p class="warn-text" data-link-status="reconnect" { (icon("alert")) " " (t("bank.needs_reconnect")) } },
            LinkStatus::Error => {
                p class="warn-text" data-link-status="error" { (icon("alert")) " " (t("bank.sync_failed")) " " (link.last_error.clone().unwrap_or_default()) }
            },
        }
    }
}

/// The "Connected banks" list on the Accounts screen.
pub fn links_section(c: &Ctx, w: &Wallet, enc: &str) -> Markup {
    html! {
        @if !w.links.is_empty() {
            section aria-labelledby="links-h" id="bank-links" {
                div class="section-head" { h2 id="links-h" { (t("bank.connected")) } }
                ul class="rows card" {
                    @for l in &w.links {
                        li class="row bank-link" data-link=(l.institution) {
                            button type="button" class="row-main" data-on:click=(open_sheet(&format!("/ui/sheet/bank/{}?view={enc}", l.id)))
                                aria-label=(tf("bank.link_details", &[("bank", &l.institution)])) {
                                span class="row-name" { span class="acct-icon" { (icon("bank")) } (l.institution) }
                                span class="row-meta" { (status_line(c, l)) }
                            }
                            div class="row-amount" { span class="num muted small" { (tf("bank.n_accounts", &[("n", &w.linked_accounts(&l.id).len().to_string())])) } }
                        }
                    }
                }
            }
        }
    }
}

fn report_toasts(user: &UserRecord, bank: &str, r: &SyncReport) -> Vec<Markup> {
    let s = &r.stats;
    let mut parts = vec![tf("bank.r_added", &[("n", &s.added.to_string())])];
    if s.categorized > 0 {
        parts.push(tf("bank.r_categorized", &[("n", &s.categorized.to_string())]));
    }
    if s.matched > 0 {
        parts.push(tf("bank.r_matched", &[("n", &s.matched.to_string())]));
    }
    if s.transfers > 0 {
        parts.push(tf("bank.r_transfers", &[("n", &s.transfers.to_string())]));
    }
    if s.paychecks > 0 {
        parts.push(tf("bank.r_paychecks", &[("n", &s.paychecks.to_string())]));
    }
    let mut v = vec![toast(ToastKind::Success, &format!("{} {}.", tf("bank.r_synced", &[("bank", bank)]), parts.join(", ")), None)];
    if r.needs_line > 0 {
        v.push(toast(ToastKind::Warning, &tf("bank.r_needs_line", &[("n", &r.needs_line.to_string())]), None));
    }
    if !r.missing_months.is_empty() {
        let months: Vec<String> = r.missing_months.iter().map(|m| month_label(*m)).collect();
        v.push(toast(ToastKind::Warning, &tf("bank.r_missing", &[("months", &months.join(", "))]), None));
    }
    let _ = user;
    v
}

async fn sync_and_finish(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View, link: &Id) -> Sse {
    let bank = st.wallet(user).await.ok().and_then(|w| w.link(link).map(|l| l.institution.clone())).unwrap_or_default();
    match sync_link(st, user, link).await {
        Ok(r) => done(st, user, headers, view, report_toasts(user, &bank, &r)).await,
        // Close the sheet so the connection's new status shows in the list.
        Err(e) => failed(st, user, headers, view, &e).await.script("pz.closeSheet()"),
    }
}

/// A new or refreshed connection. `reconnect` (or a known enrollment)
/// refreshes the token and syncs; otherwise the "choose accounts" sheet
/// opens.
struct Enrollment {
    provider: &'static str,
    enrollment_id: String,
    institution: String,
    token: String,
    reconnect: Option<Id>,
}

fn import_from(st: &Shared, user: &UserRecord, f: &std::collections::HashMap<String, String>) -> chrono::NaiveDate {
    let today = st.today_for(user);
    chrono::NaiveDate::parse_from_str(field(f, "import_from"), "%Y-%m-%d").unwrap_or_else(|_| today.with_day(1).unwrap_or(today))
}

async fn finish_enrollment(st: &Shared, user: &UserRecord, headers: &HeaderMap, view: &View, e: Enrollment, from: chrono::NaiveDate) -> Sse {
    let sealed = seal(&st.cfg.data_key, &e.token);
    let existing = match e.reconnect {
        Some(id) => Some(id),
        None => st.wallet(user).await.ok().and_then(|w| w.links.iter().find(|l| l.provider == e.provider && l.enrollment_id == e.enrollment_id).map(|l| l.id.clone())),
    };
    if let Some(id) = existing {
        let lid = id.clone();
        let r = st.mutate_wallet(user, move |w, _| {
            let l = w.link_mut(&lid)?;
            l.access_token = sealed;
            l.status = LinkStatus::Active;
            l.last_error = None;
            Ok(())
        }).await;
        if let Err(err) = r {
            return failed(st, user, headers, view, &err).await;
        }
        return sync_and_finish(st, user, headers, view, &id).await;
    }
    let mut link = BankLink {
        id: Id::generate(),
        provider: e.provider.into(),
        enrollment_id: e.enrollment_id,
        institution: e.institution,
        access_token: sealed,
        status: LinkStatus::Active,
        last_error: None,
        import_from: from,
        last_sync: None,
        cursor: None,
    };
    let remote = match remote_accounts(st, user, &link).await {
        Ok(r) => r,
        Err(err) => return failed(st, user, headers, view, &err).await,
    };
    // SimpleFIN and Plaid name the bank through its accounts.
    if let Some(name) = remote.iter().find_map(|r| r.institution.clone()).filter(|_| link.institution == t("bank.default_name")) {
        link.institution = name;
    }
    let l2 = link.clone();
    match st.mutate_wallet(user, move |w, _| {
        w.links.push(l2);
        Ok(())
    }).await {
        Ok(((), w)) => Sse::new().patch(map_sheet(&link, &remote, &w, view)),
        Err(err) => failed(st, user, headers, view, &err).await,
    }
}

pub async fn enroll(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, view_or(&None, &user));
    let token = field(&f, "access_token").to_string();
    let enrollment = field(&f, "enrollment_id").to_string();
    if token.is_empty() || enrollment.is_empty() {
        return failed(&st, &user, &headers, &view, &AppError::bad(t("bank.connect_failed"))).await;
    }
    if st.cfg.teller.is_none() {
        return failed(&st, &user, &headers, &view, &AppError::bad(t("bank.not_configured"))).await;
    }
    let institution = match field(&f, "institution") {
        "" => t("bank.default_name"),
        s => s.to_string(),
    };
    let from = import_from(&st, &user, &f);
    let e = Enrollment { provider: crate::bank::TELLER, enrollment_id: enrollment, institution, token, reconnect: None };
    finish_enrollment(&st, &user, &headers, &view, e, from).await
}

pub async fn simplefin_enroll(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, view_or(&None, &user));
    let setup = field(&f, "setup_token").to_string();
    let access = match crate::bank::SimpleFin::new().claim(&setup, st.cfg.simplefin_allow_http).await {
        Ok(a) => a,
        Err(crate::bank::ProviderError::Other(m) | crate::bank::ProviderError::Reconnect(m)) => return failed(&st, &user, &headers, &view, &AppError::bad(m)).await,
    };
    // One access URL covers every bank linked at SimpleFIN Bridge.
    let enrollment = {
        use sha2::Digest;
        hex::encode(&sha2::Sha256::digest(access.as_bytes())[..8])
    };
    let from = import_from(&st, &user, &f);
    let reconnect = field(&f, "link_id");
    let e = Enrollment {
        provider: crate::bank::SIMPLEFIN,
        enrollment_id: enrollment,
        institution: t("bank.default_name"),
        token: access,
        reconnect: (!reconnect.is_empty()).then(|| Id::new(reconnect)),
    };
    finish_enrollment(&st, &user, &headers, &view, e, from).await
}

pub async fn plaid_enroll(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, view_or(&None, &user));
    let plaid = match st.plaid() {
        Ok(p) => p,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let reconnect = field(&f, "link_id").to_string();
    if !reconnect.is_empty() {
        // Update mode keeps the same access token; the connection works again.
        let id = Id::new(&reconnect);
        let lid = id.clone();
        if let Err(e) = st.mutate_wallet(&user, move |w, _| {
            let l = w.link_mut(&lid)?;
            l.status = LinkStatus::Active;
            l.last_error = None;
            Ok(())
        }).await {
            return failed(&st, &user, &headers, &view, &e).await;
        }
        return sync_and_finish(&st, &user, &headers, &view, &id).await;
    }
    let (access, item) = match plaid.exchange(field(&f, "public_token")).await {
        Ok(v) => v,
        Err(crate::bank::ProviderError::Other(m) | crate::bank::ProviderError::Reconnect(m)) => return failed(&st, &user, &headers, &view, &AppError::bad(m)).await,
    };
    let institution = match field(&f, "institution") {
        "" => t("bank.default_name"),
        s => s.to_string(),
    };
    let from = import_from(&st, &user, &f);
    let e = Enrollment { provider: crate::bank::PLAID, enrollment_id: item, institution, token: access, reconnect: None };
    finish_enrollment(&st, &user, &headers, &view, e, from).await
}

pub async fn map_accounts(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, view_or(&None, &user));
    let link = match st.wallet(&user).await.map(|w| w.link(&id).cloned()) {
        Ok(Some(l)) => l,
        Ok(None) => return failed(&st, &user, &headers, &view, &AppError::NotFound).await,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let remote = match remote_accounts(&st, &user, &link).await {
        Ok(r) => r,
        Err(e) => return failed(&st, &user, &headers, &view, &e).await,
    };
    let today = st.today_for(&user);
    let choices: Vec<(RemoteAccount, String)> = remote.into_iter().map(|ra| {
        let choice = field(&f, &format!("map_{}", ra.id)).to_string();
        (ra, choice)
    }).collect();
    let kinds: Vec<(String, AccountKind)> = choices.iter().filter_map(|(ra, _)| AccountKind::parse(field(&f, &format!("kind_{}", ra.id))).map(|k| (ra.id.clone(), k))).collect();
    let lid = id.clone();
    let r: AppResult<((), Wallet)> = st.mutate_wallet(&user, move |w, _| {
        for (ra, choice) in &choices {
            // Unlink whatever this bank account was mapped to before.
            for a in w.accounts.iter_mut().filter(|a| a.external_id.as_deref() == Some(ra.id.as_str()) && a.link_id.as_ref() == Some(&lid)) {
                a.link_id = None;
            }
            let target = match choice.as_str() {
                "skip" => continue,
                "" | "new" => {
                    let kind = kinds.iter().find(|(id, _)| id == &ra.id).map_or(ra.kind, |(_, k)| *k);
                    w.add_account(&ra.label(), kind, Cents::ZERO, today, None)?
                }
                other => {
                    let id = Id::new(other);
                    if w.account(&id).is_none() {
                        return Err(AppError::NotFound);
                    }
                    id
                }
            };
            if let Some(a) = w.accounts.iter_mut().find(|a| a.id == target) {
                a.external_id = Some(ra.id.clone());
                a.link_id = Some(lid.clone());
            }
        }
        Ok(())
    }).await;
    if let Err(e) = r {
        return failed(&st, &user, &headers, &view, &e).await;
    }
    sync_and_finish(&st, &user, &headers, &view, &id).await
}

pub async fn sync_now(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, view_or(&None, &user));
    sync_and_finish(&st, &user, &headers, &view, &id).await
}

pub async fn disconnect(State(st): State<Shared>, Extension(user): Extension<AuthUser>, headers: HeaderMap, Path(id): Path<Id>, Form(f): F) -> Sse {
    let user = user.0;
    let view = view_of(&f, view_or(&None, &user));
    match st.mutate_wallet(&user, move |w, _| Ok(w.remove_link(&id)?)).await {
        Ok(_) => done(&st, &user, &headers, &view, vec![toast(ToastKind::Success, &t("bank.disconnected"), None)]).await,
        Err(e) => failed(&st, &user, &headers, &view, &e).await,
    }
}

/// Settings card: which bank providers are available, and how to add them.
pub fn settings_card(st: &Shared) -> Markup {
    html! {
        section class="card" aria-labelledby="bank-h" {
            h2 id="bank-h" class="h3" { (t("bank.settings_title")) }
            p class="muted small" { (t("bank.settings_intro")) }
            ul class="provider-status" {
                li data-provider="simplefin" {
                    strong { "SimpleFIN Bridge" } " — " (icon("check")) " " (t("bank.settings_sf"))
                }
                li data-provider="teller" {
                    strong { "Teller" } " — "
                    @match st.cfg.teller.as_ref() {
                        Some(cfg) => {
                            (icon("check")) " " (tf("bank.settings_on", &[("env", &cfg.environment)]))
                            @if cfg.cert.is_none() && cfg.environment != "sandbox" { " " span class="warn-text" { (t("bank.settings_no_cert")) } }
                        },
                        None => { span class="muted" { (t("bank.settings_off_teller")) } " " code { "PZ_TELLER_APP_ID" } ", " code { "PZ_TELLER_ENV" } ", " code { "PZ_TELLER_CERT" } ", " code { "PZ_TELLER_KEY" } },
                    }
                }
                li data-provider="plaid" {
                    strong { "Plaid" } " — "
                    @match st.cfg.plaid.as_ref() {
                        Some(cfg) => { (icon("check")) " " (tf("bank.settings_on", &[("env", &cfg.environment)])) },
                        None => { span class="muted" { (t("bank.settings_off_plaid")) } " " code { "PZ_PLAID_CLIENT_ID" } ", " code { "PZ_PLAID_SECRET" } ", " code { "PZ_PLAID_ENV" } },
                    }
                }
            }
        }
    }
}
