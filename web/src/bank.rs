//! Bank sync through a data provider. All are optional; use any mix:
//!
//! - **SimpleFIN Bridge** (simplefin.org): the person pastes a setup token,
//!   which is exchanged once for an access URL. No server settings needed.
//! - **Plaid** (plaid.com): the browser runs Plaid Link with a link token
//!   from the server; the public token is exchanged for an access token.
//!
//! Every provider is reduced to the same accounts / transactions /
//! balances, so importing follows one set of rules
//! (`paycheckzero_core::bank`), and afterwards each synced account is
//! reconciled to the bank's balance. Secrets are kept encrypted.

use crate::error::{AppError, AppResult};
use crate::i18n::t;
use crate::Shared;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use chrono::{Datelike, NaiveDate, SecondsFormat, Utc};
use paycheckzero_core::bank::{import_into_month, BankTx, ImportStats};
use paycheckzero_core::*;
use paycheckzero_storage::UserRecord;
use serde::Deserialize;
use std::collections::BTreeMap;

// ----------------------------------------------------------------------
// Token sealing
// ----------------------------------------------------------------------

/// Encrypts a token for storage: base64(nonce ‖ ciphertext).
#[must_use]
pub fn seal(key: &[u8; 32], plain: &str) -> String {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), plain.as_bytes()).unwrap_or_default();
    let mut out = nonce.to_vec();
    out.extend(ct);
    base64::engine::general_purpose::STANDARD.encode(out)
}

/// Decrypts a sealed token; `None` if the key changed or it was tampered with.
#[must_use]
pub fn open(key: &[u8; 32], sealed: &str) -> Option<String> {
    let raw = base64::engine::general_purpose::STANDARD.decode(sealed).ok()?;
    if raw.len() < 13 {
        return None;
    }
    let (nonce, ct) = raw.split_at(12);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    String::from_utf8(cipher.decrypt(Nonce::from_slice(nonce), ct).ok()?).ok()
}

// ----------------------------------------------------------------------
// Common shapes
// ----------------------------------------------------------------------

/// A bank account as a provider reports it.
#[derive(Debug, Clone)]
pub struct RemoteAccount {
    pub id: String,
    pub name: String,
    pub mask: Option<String>,
    /// Our best guess; the person can change it when adding the account.
    pub kind: AccountKind,
    pub institution: Option<String>,
}

impl RemoteAccount {
    /// "Checking ••1234".
    #[must_use]
    pub fn label(&self) -> String {
        match &self.mask {
            Some(l) if !l.is_empty() => format!("{} ••{l}", self.name),
            _ => self.name.clone(),
        }
    }
}

/// A posted transaction as a provider reports it (negative = money out).
#[derive(Debug, Clone)]
struct RemoteTx {
    id: String,
    account: String,
    date: NaiveDate,
    amount: Cents,
    payee: String,
}

/// What one pull brings back.
#[derive(Debug, Default)]
struct Pulled {
    txs: Vec<RemoteTx>,
    /// (account id, balance; for cards and loans what's owed, positive).
    balances: Vec<(String, Cents)>,
    /// Transactions the bank withdrew (Plaid).
    removed: Vec<String>,
    cursor: Option<String>,
}

/// Guesses an account's kind from its name (SimpleFIN has no types).
fn kind_from_name(name: &str) -> AccountKind {
    let n = name.to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| n.contains(x));
    if has(&["401", "403b", "ira", "roth", "retire", "pension", "tsp"]) {
        AccountKind::Retirement
    } else if has(&["brokerage", "invest", "stock", "individual"]) {
        AccountKind::Investment
    } else if has(&["card", "credit", "visa", "mastercard", "amex", "discover", "sapphire", "freedom"]) {
        AccountKind::CreditCard
    } else if has(&["saving", "money market", "mmsa", "cd "]) {
        AccountKind::Savings
    } else {
        AccountKind::Checking
    }
}

#[derive(Debug)]
pub enum ProviderError {
    /// The bank needs the user to sign in again.
    Reconnect(String),
    Other(String),
}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        ProviderError::Other(e.to_string())
    }
}

/// "-12.34" → -1234 cents.
fn parse_amount(s: &str) -> Option<Cents> {
    let s = s.trim();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (whole, frac) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let frac = format!("{frac:0<2}");
    let cents: i64 = frac.get(..2)?.parse().ok()?;
    let v = whole.checked_mul(100)?.checked_add(cents)?;
    Some(Cents::new(if neg { -v } else { v }))
}

// ----------------------------------------------------------------------
// SimpleFIN
// ----------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SfOrg {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SfTx {
    id: String,
    posted: i64,
    amount: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    payee: Option<String>,
    #[serde(default)]
    pending: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SfAccount {
    id: String,
    name: String,
    #[serde(default)]
    org: Option<SfOrg>,
    balance: String,
    #[serde(default)]
    transactions: Vec<SfTx>,
}

#[derive(Debug, Deserialize)]
struct SfResponse {
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default)]
    accounts: Vec<SfAccount>,
}

pub struct SimpleFin {
    http: reqwest::Client,
}

impl SimpleFin {
    #[must_use]
    pub fn new() -> SimpleFin {
        SimpleFin { http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().unwrap_or_default() }
    }

    /// Exchanges a setup token (base64 of a claim URL) for an access URL.
    pub async fn claim(&self, setup_token: &str, allow_http: bool) -> Result<String, ProviderError> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(setup_token.trim())
            .map_err(|_| ProviderError::Other(t("bank.sf_bad_token")))?;
        let url = String::from_utf8(raw).map_err(|_| ProviderError::Other(t("bank.sf_bad_token")))?;
        if !(url.starts_with("https://") || (allow_http && url.starts_with("http://"))) {
            return Err(ProviderError::Other(t("bank.sf_bad_token")));
        }
        let resp = self.http.post(url.trim()).header("Content-Length", "0").send().await?;
        if !resp.status().is_success() {
            return Err(ProviderError::Other(t("bank.sf_claim_failed")));
        }
        let access = resp.text().await?.trim().to_string();
        if !access.starts_with("http") {
            return Err(ProviderError::Other(t("bank.sf_claim_failed")));
        }
        Ok(access)
    }

    async fn fetch(&self, access_url: &str, start: Option<NaiveDate>, balances_only: bool) -> Result<SfResponse, ProviderError> {
        let mut url = reqwest::Url::parse(access_url).map_err(|_| ProviderError::Reconnect("bad access url".into()))?;
        let user = url.username().to_string();
        let pass = url.password().map(str::to_string);
        let _ = url.set_username("");
        let _ = url.set_password(None);
        let mut req = self.http.get(format!("{}/accounts", url.as_str().trim_end_matches('/'))).basic_auth(user, pass);
        if balances_only {
            req = req.query(&[("balances-only", "1")]);
        } else if let Some(d) = start {
            let ts = d.and_hms_opt(0, 0, 0).map_or(0, |dt| dt.and_utc().timestamp());
            req = req.query(&[("start-date", ts.to_string())]);
        }
        let resp = req.send().await?;
        if resp.status().as_u16() == 403 || resp.status().as_u16() == 401 {
            return Err(ProviderError::Reconnect("access revoked".into()));
        }
        if !resp.status().is_success() {
            return Err(ProviderError::Other(format!("SimpleFIN answered {}", resp.status())));
        }
        resp.json::<SfResponse>().await.map_err(|e| ProviderError::Other(format!("unexpected response from SimpleFIN: {e}")))
    }

    async fn accounts(&self, access_url: &str) -> Result<Vec<RemoteAccount>, ProviderError> {
        let r = self.fetch(access_url, None, true).await?;
        Ok(r.accounts
            .into_iter()
            .map(|a| {
                let owed = parse_amount(&a.balance).is_some_and(Cents::is_negative);
                let mut kind = kind_from_name(&a.name);
                if owed && kind == AccountKind::Checking && !a.name.to_lowercase().contains("check") {
                    kind = AccountKind::CreditCard;
                }
                RemoteAccount { id: a.id, name: a.name, mask: None, kind, institution: a.org.and_then(|o| o.name) }
            })
            .collect())
    }

    async fn pull(&self, access_url: &str, accounts: &[String], from: NaiveDate, cards: &[String]) -> Result<Pulled, ProviderError> {
        let r = self.fetch(access_url, Some(from), false).await?;
        if !r.errors.is_empty() && r.accounts.is_empty() {
            return Err(ProviderError::Other(r.errors.join("; ")));
        }
        let mut p = Pulled::default();
        for a in r.accounts.into_iter().filter(|a| accounts.contains(&a.id)) {
            if let Some(bal) = parse_amount(&a.balance) {
                // SimpleFIN shows card debt as a negative balance.
                p.balances.push((a.id.clone(), if cards.contains(&a.id) { -bal } else { bal }));
            }
            for t in a.transactions.into_iter().filter(|t| t.pending != Some(true)) {
                let Some(amount) = parse_amount(&t.amount) else { continue };
                let Some(date) = chrono::DateTime::from_timestamp(t.posted, 0).map(|d| d.date_naive()) else { continue };
                let payee = t.payee.filter(|p| !p.trim().is_empty()).unwrap_or(t.description);
                p.txs.push(RemoteTx { id: format!("sf:{}", t.id), account: a.id.clone(), date, amount, payee });
            }
        }
        Ok(p)
    }
}

impl Default for SimpleFin {
    fn default() -> Self {
        Self::new()
    }
}

// ----------------------------------------------------------------------
// Plaid
// ----------------------------------------------------------------------

pub struct Plaid {
    http: reqwest::Client,
    cfg: crate::config::PlaidConfig,
}

#[derive(Debug, Deserialize)]
struct PBalances {
    #[serde(default)]
    current: Option<f64>,
    #[serde(default)]
    available: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PAccount {
    account_id: String,
    name: String,
    #[serde(default)]
    mask: Option<String>,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    subtype: Option<String>,
    balances: PBalances,
}

#[derive(Debug, Deserialize)]
struct PTx {
    transaction_id: String,
    account_id: String,
    amount: f64,
    date: NaiveDate,
    #[serde(default)]
    name: String,
    #[serde(default)]
    merchant_name: Option<String>,
    #[serde(default)]
    pending: bool,
}

#[derive(Debug, Deserialize)]
struct PRemoved {
    transaction_id: String,
}

#[derive(Debug, Deserialize)]
struct PSync {
    #[serde(default)]
    added: Vec<PTx>,
    #[serde(default)]
    removed: Vec<PRemoved>,
    next_cursor: String,
    #[serde(default)]
    has_more: bool,
}

fn cents_f(v: f64) -> Cents {
    // Plaid sends money as JSON numbers; round to the nearest cent.
    #[allow(clippy::cast_possible_truncation)]
    Cents::new((v * 100.0).round() as i64)
}

impl PAccount {
    fn local_kind(&self) -> AccountKind {
        let sub = self.subtype.clone().unwrap_or_default();
        match self.kind.as_str() {
            "credit" | "loan" => AccountKind::CreditCard,
            "investment" => {
                if ["401k", "403b", "457b", "ira", "roth", "roth 401k", "pension", "retirement", "sep ira", "simple ira", "tsp"].contains(&sub.as_str()) {
                    AccountKind::Retirement
                } else {
                    AccountKind::Investment
                }
            }
            _ if sub == "savings" || sub == "money market" || sub == "cd" => AccountKind::Savings,
            _ => AccountKind::Checking,
        }
    }
}

impl Plaid {
    pub fn new(cfg: &crate::config::PlaidConfig) -> Plaid {
        Plaid { http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().unwrap_or_default(), cfg: cfg.clone() }
    }

    async fn post<T: serde::de::DeserializeOwned>(&self, path: &str, mut body: serde_json::Value) -> Result<T, ProviderError> {
        body["client_id"] = self.cfg.client_id.clone().into();
        body["secret"] = self.cfg.secret.clone().into();
        let resp = self.http.post(format!("{}{path}", self.cfg.api.trim_end_matches('/'))).json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if status.is_success() {
            return serde_json::from_str(&text).map_err(|e| ProviderError::Other(format!("unexpected response from Plaid: {e}")));
        }
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let code = v["error_code"].as_str().unwrap_or_default().to_string();
        if matches!(code.as_str(), "ITEM_LOGIN_REQUIRED" | "ITEM_NOT_FOUND" | "ACCESS_NOT_GRANTED" | "INVALID_ACCESS_TOKEN") {
            Err(ProviderError::Reconnect(code))
        } else {
            Err(ProviderError::Other(format!("Plaid: {} {}", code, v["error_message"].as_str().unwrap_or_default())))
        }
    }

    /// A Link token (new connection, or "update mode" for `access_token`).
    pub async fn link_token(&self, user_id: &str, access_token: Option<&str>) -> Result<String, ProviderError> {
        let mut body = serde_json::json!({
            "client_name": "PaycheckZero",
            "user": { "client_user_id": user_id },
            "country_codes": self.cfg.countries,
            "language": "en",
        });
        match access_token {
            Some(a) => body["access_token"] = a.into(),
            None => body["products"] = serde_json::json!(["transactions"]),
        }
        let v: serde_json::Value = self.post("/link/token/create", body).await?;
        v["link_token"].as_str().map(str::to_string).ok_or_else(|| ProviderError::Other("no link token".into()))
    }

    /// Public token → (access token, item id).
    pub async fn exchange(&self, public_token: &str) -> Result<(String, String), ProviderError> {
        let v: serde_json::Value = self.post("/item/public_token/exchange", serde_json::json!({ "public_token": public_token })).await?;
        match (v["access_token"].as_str(), v["item_id"].as_str()) {
            (Some(a), Some(i)) => Ok((a.to_string(), i.to_string())),
            _ => Err(ProviderError::Other("Plaid exchange failed".into())),
        }
    }

    async fn raw_accounts(&self, access: &str) -> Result<Vec<PAccount>, ProviderError> {
        #[derive(Deserialize)]
        struct R {
            accounts: Vec<PAccount>,
        }
        let r: R = self.post("/accounts/get", serde_json::json!({ "access_token": access })).await?;
        Ok(r.accounts)
    }

    async fn accounts(&self, access: &str) -> Result<Vec<RemoteAccount>, ProviderError> {
        Ok(self
            .raw_accounts(access)
            .await?
            .into_iter()
            .map(|a| RemoteAccount { kind: a.local_kind(), id: a.account_id, name: a.name, mask: a.mask, institution: None })
            .collect())
    }

    async fn pull(&self, access: &str, accounts: &[String], from: NaiveDate, cursor: Option<String>) -> Result<Pulled, ProviderError> {
        let mut p = Pulled::default();
        let mut cursor = cursor;
        for _ in 0..40 {
            let mut body = serde_json::json!({ "access_token": access, "count": 500 });
            if let Some(c) = &cursor {
                body["cursor"] = c.clone().into();
            }
            let page: PSync = self.post("/transactions/sync", body).await?;
            for t in page.added.into_iter().filter(|t| !t.pending && t.date >= from && accounts.contains(&t.account_id)) {
                // Plaid's amounts are positive for money leaving the account.
                let payee = t.merchant_name.filter(|m| !m.trim().is_empty()).unwrap_or(t.name);
                p.txs.push(RemoteTx { id: format!("plaid:{}", t.transaction_id), account: t.account_id, date: t.date, amount: -cents_f(t.amount), payee });
            }
            p.removed.extend(page.removed.into_iter().map(|r| format!("plaid:{}", r.transaction_id)));
            cursor = Some(page.next_cursor);
            if !page.has_more {
                break;
            }
        }
        p.cursor = cursor;
        for a in self.raw_accounts(access).await?.into_iter().filter(|a| accounts.contains(&a.account_id)) {
            if let Some(v) = a.balances.current.or(a.balances.available) {
                p.balances.push((a.account_id, cents_f(v)));
            }
        }
        Ok(p)
    }
}

// ----------------------------------------------------------------------
// Sync
// ----------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SyncReport {
    pub stats: ImportStats,
    /// Months that don't exist yet, so their transactions wait.
    pub missing_months: Vec<NaiveDate>,
    /// Transactions now waiting for a line.
    pub needs_line: usize,
}

fn first_of(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

pub const SIMPLEFIN: &str = "simplefin";
pub const PLAID: &str = "plaid";

/// "SimpleFIN", "Plaid" (or the stored name of a provider that's gone).
#[must_use]
pub fn provider_name(p: &str) -> &str {
    match p {
        SIMPLEFIN => "SimpleFIN",
        PLAID => "Plaid",
        "teller" => "Teller",
        other => other,
    }
}

/// Provider credentials saved in the app (Settings → Bank providers).
/// They take precedence over environment variables and are stored
/// encrypted as one server-wide setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct SavedProviders {
    #[serde(default)]
    pub plaid: Option<SavedPlaid>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct SavedPlaid {
    pub client_id: String,
    pub secret: String,
    pub environment: String,
    pub countries: Vec<String>,
}

const PROVIDERS_SETTING: &str = "bank_providers";

impl crate::AppState {
    #[must_use]
    pub fn plaid_cfg(&self) -> Option<crate::config::PlaidConfig> {
        if let Some(s) = self.providers.read().ok().and_then(|p| p.plaid.clone()) {
            return Some(crate::config::PlaidConfig {
                api: crate::config::plaid_api(&s.environment),
                link_js: crate::config::plaid_link_js(),
                client_id: s.client_id,
                secret: s.secret,
                environment: s.environment,
                countries: s.countries,
                from_env: false,
            });
        }
        if self.ignore_env_providers() { None } else { self.cfg.plaid.clone() }
    }

    #[must_use]
    pub fn saved_providers(&self) -> SavedProviders {
        self.providers.read().map(|p| p.clone()).unwrap_or_default()
    }

    /// Loads saved provider settings (at start, and after a test reset).
    pub async fn load_providers(&self) {
        let saved = match self.store.setting(PROVIDERS_SETTING).await {
            Ok(Some(sealed)) => open(&self.cfg.data_key, &sealed).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_else(|| {
                tracing::warn!("saved bank provider settings can't be read (data key changed?); set them again in Settings");
                SavedProviders::default()
            }),
            _ => SavedProviders::default(),
        };
        if let Ok(mut p) = self.providers.write() {
            *p = saved;
        }
    }

    pub async fn save_providers(&self, p: SavedProviders) -> AppResult<()> {
        let value = if p == SavedProviders::default() {
            None
        } else {
            Some(seal(&self.cfg.data_key, &serde_json::to_string(&p).map_err(|e| AppError::Internal(e.to_string()))?))
        };
        self.store.set_setting(PROVIDERS_SETTING, value.as_deref()).await?;
        if let Ok(mut g) = self.providers.write() {
            *g = p;
        }
        Ok(())
    }

    pub fn plaid(&self) -> AppResult<Plaid> {
        self.plaid_cfg().as_ref().map(Plaid::new).ok_or_else(|| AppError::bad(t("bank.not_configured")))
    }
}

fn to_app(e: ProviderError) -> AppError {
    match e {
        ProviderError::Reconnect(_) => AppError::bad(t("bank.needs_reconnect")),
        ProviderError::Other(msg) => AppError::bad(format!("{} {msg}", t("bank.sync_failed"))),
    }
}

async fn mark_link(st: &Shared, user: &UserRecord, link: &Id, status: LinkStatus, error: Option<String>, synced: bool, cursor: Option<String>) -> AppResult<()> {
    let link = link.clone();
    st.mutate_wallet(user, move |w, _| {
        let l = w.link_mut(&link)?;
        l.status = status;
        l.last_error = error;
        if synced {
            l.last_sync = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
        }
        if cursor.is_some() {
            l.cursor = cursor;
        }
        Ok(())
    })
    .await?;
    Ok(())
}

async fn provider_accounts(st: &Shared, link: &BankLink, token: &str) -> Result<Vec<RemoteAccount>, ProviderError> {
    let app = |e: AppError| ProviderError::Other(e.to_string());
    match link.provider.as_str() {
        SIMPLEFIN => SimpleFin::new().accounts(token).await,
        PLAID => st.plaid().map_err(app)?.accounts(token).await,
        _ => Err(ProviderError::Other(t("bank.provider_gone"))),
    }
}

/// The remote accounts of a connection (for choosing what to import).
pub async fn remote_accounts(st: &Shared, user: &UserRecord, link: &BankLink) -> AppResult<Vec<RemoteAccount>> {
    let token = open(&st.cfg.data_key, &link.access_token).ok_or_else(|| AppError::bad(t("bank.needs_reconnect")))?;
    match provider_accounts(st, link, &token).await {
        Ok(a) => Ok(a),
        Err(e) => {
            if let ProviderError::Reconnect(code) = &e {
                mark_link(st, user, &link.id, LinkStatus::NeedsReconnect, Some(code.clone()), false, None).await?;
            }
            Err(to_app(e))
        }
    }
}

/// Pulls a connection's new transactions and balances.
pub async fn sync_link(st: &Shared, user: &UserRecord, link_id: &Id) -> AppResult<SyncReport> {
    let wallet = st.wallet(user).await?;
    let link = wallet.link(link_id).cloned().ok_or(AppError::NotFound)?;
    let Some(token) = open(&st.cfg.data_key, &link.access_token) else {
        mark_link(st, user, link_id, LinkStatus::NeedsReconnect, Some("stored token unreadable".into()), false, None).await?;
        return Err(AppError::bad(t("bank.needs_reconnect")));
    };
    match pull(st, user, &wallet, &link, &token).await {
        Ok((report, cursor)) => {
            mark_link(st, user, link_id, LinkStatus::Active, None, true, cursor).await?;
            Ok(report)
        }
        Err(e) => {
            let (status, msg) = match &e {
                ProviderError::Reconnect(c) => (LinkStatus::NeedsReconnect, c.clone()),
                ProviderError::Other(m) => (LinkStatus::Error, m.clone()),
            };
            mark_link(st, user, link_id, status, Some(msg), false, None).await?;
            Err(to_app(e))
        }
    }
}

async fn pull(st: &Shared, user: &UserRecord, wallet: &Wallet, link: &BankLink, token: &str) -> Result<(SyncReport, Option<String>), ProviderError> {
    let internal = |e: AppError| ProviderError::Other(e.to_string());
    let locals: Vec<Account> = wallet.linked_accounts(&link.id).into_iter().filter(|a| a.external_id.is_some()).cloned().collect();
    let by_ext = |ext: &str| locals.iter().find(|a| a.external_id.as_deref() == Some(ext));
    let exts: Vec<String> = locals.iter().filter_map(|a| a.external_id.clone()).collect();
    let linked: Vec<Id> = wallet.accounts.iter().filter(|a| a.link_id.is_some()).map(|a| a.id.clone()).collect();
    // After the first sync, look back a month (banks post late), never
    // before the connection's start date.
    let from = if link.last_sync.is_some() { (st.today_for(user) - chrono::Duration::days(30)).max(link.import_from) } else { link.import_from };
    let pulled = match link.provider.as_str() {
        SIMPLEFIN => {
            let cards: Vec<String> = locals.iter().filter(|a| a.kind.is_card()).filter_map(|a| a.external_id.clone()).collect();
            SimpleFin::new().pull(token, &exts, from, &cards).await?
        }
        PLAID => st.plaid().map_err(internal)?.pull(token, &exts, link.import_from, link.cursor.clone()).await?,
        _ => return Err(ProviderError::Other(t("bank.provider_gone"))),
    };
    let seen = st.store.bank_seen(&user.id).await.map_err(|e| internal(e.into()))?;
    let mut per_month: BTreeMap<NaiveDate, Vec<BankTx>> = BTreeMap::new();
    for tx in pulled.txs {
        if seen.contains(&tx.id) || tx.date < link.import_from {
            continue;
        }
        let Some(a) = by_ext(&tx.account) else { continue };
        per_month.entry(first_of(tx.date)).or_default().push(BankTx {
            external_id: tx.id,
            account: a.id.clone(),
            date: tx.date,
            amount: tx.amount,
            payee: Some(tx.payee),
            depository: a.kind.group() == AccountGroup::Cash,
        });
    }
    let history = st.all_months(user).await.map_err(internal)?;
    let mut report = SyncReport::default();
    for (ym, mut batch) in per_month {
        batch.sort_by_key(|b| b.date);
        let ids: Vec<String> = batch.iter().map(|b| b.external_id.clone()).collect();
        let mid = st.store.month_id_by_year_month(&user.id, ym).await.map_err(|e| internal(e.into()))?;
        let Some(mid) = mid else {
            report.missing_months.push(ym);
            continue;
        };
        let linked = linked.clone();
        let hist = history.clone();
        match st.mutate(user, &mid, move |m| Ok(import_into_month(m, batch, &hist, &linked))).await {
            Ok((stats, m)) => {
                report.stats.added += stats.added;
                report.stats.matched += stats.matched;
                report.stats.categorized += stats.categorized;
                report.stats.transfers += stats.transfers;
                report.stats.paychecks += stats.paychecks;
                report.needs_line += m.transactions.iter().filter(|t| t.needs_line() && t.external_id.is_some()).count();
                st.store.mark_bank_seen(&user.id, &ids).await.map_err(|e| internal(e.into()))?;
            }
            Err(AppError::Archived) => report.missing_months.push(ym),
            Err(e) => return Err(internal(e)),
        }
    }
    // Transactions the bank withdrew (e.g. a reversed charge).
    if !pulled.removed.is_empty() {
        for m in &history {
            let gone: Vec<Id> = m.transactions.iter().filter(|t| t.external_id.as_ref().is_some_and(|e| pulled.removed.contains(e))).map(|t| t.id.clone()).collect();
            if !gone.is_empty() {
                let _ = st.mutate(user, &m.id, move |mm| {
                    for id in &gone {
                        mm.delete_transaction(id)?;
                    }
                    Ok(())
                }).await;
            }
        }
    }
    let today = st.today_for(user);
    let balances: Vec<(Id, Cents)> = pulled.balances.into_iter().filter_map(|(ext, bal)| by_ext(&ext).map(|a| (a.id.clone(), bal))).collect();
    st.mutate_wallet(user, move |w, all| {
        for (id, bal) in &balances {
            w.reconcile(id, *bal, all, today)?;
        }
        Ok(())
    })
    .await
    .map_err(internal)?;
    Ok((report, pulled.cursor))
}

/// Syncs every active connection every `hours` (0 turns it off).
pub fn spawn_background_sync(st: Shared) {
    let hours = st.cfg.bank_sync_hours;
    if hours == 0 {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(hours * 3600));
        tick.tick().await; // the first tick fires immediately; wait one period first
        loop {
            tick.tick().await;
            let Ok(users) = st.store.user_ids().await else { continue };
            for uid in users {
                let Ok(Some(user)) = st.store.user_by_id(&uid).await else { continue };
                let Ok(w) = st.wallet(&user).await else { continue };
                for l in w.links.iter().filter(|l| l.status == LinkStatus::Active) {
                    match sync_link(&st, &user, &l.id).await {
                        Ok(r) => tracing::info!(institution = %l.institution, added = r.stats.added, "bank sync"),
                        Err(e) => tracing::warn!(institution = %l.institution, "bank sync failed: {e}"),
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip_and_resist_tampering() {
        let k = [3u8; 32];
        let s = seal(&k, "token_abc");
        assert_ne!(s, seal(&k, "token_abc"), "fresh nonce each time");
        assert_eq!(open(&k, &s).as_deref(), Some("token_abc"));
        assert_eq!(open(&[4u8; 32], &s), None);
        assert_eq!(open(&k, "bm9wZQ=="), None);
    }

    #[test]
    fn amounts_parse_exactly() {
        assert_eq!(parse_amount("-12.34"), Some(Cents::new(-1234)));
        assert_eq!(parse_amount("1950"), Some(Cents::new(195_000)));
        assert_eq!(parse_amount(" 0.5"), Some(Cents::new(50)));
        assert_eq!(parse_amount("abc"), None);
    }
}
