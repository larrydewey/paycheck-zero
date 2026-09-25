//! HTTP-level tests of the REST API (spec §9), auth (§13.5), sync (§13.7)
//! and the cookie/CSRF rules of the web UI, run in-process against
//! in-memory SQLite.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use paycheckzero_web::config::Config;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn app() -> Router {
    let state = paycheckzero_web::build_state(Config::for_tests()).await.unwrap();
    state.clock.set_fixed(chrono::NaiveDate::from_ymd_opt(2026, 9, 10));
    paycheckzero_web::app(state)
}

struct Resp {
    status: StatusCode,
    json: Value,
    text: String,
    cookies: Vec<String>,
}

async fn call(app: &Router, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Resp {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    send(app, req).await
}

async fn send(app: &Router, req: Request<Body>) -> Resp {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookies = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    Resp { status, json, text, cookies }
}

async fn register(app: &Router) -> String {
    let r = call(app, "POST", "/api/v1/auth/register", None, Some(json!({"email": "me@example.com", "password": "secret-pass", "timezone": "America/Chicago"}))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
    r.json["access_token"].as_str().unwrap().to_string()
}

/// Creates Sept 2026 with a biweekly $1,000 income and a Rent line.
async fn setup(app: &Router, tok: &str) -> (String, String, String, String) {
    let m = call(app, "POST", "/api/v1/months", Some(tok), Some(json!({"year_month": "2026-09"}))).await;
    assert_eq!(m.status, StatusCode::CREATED, "{}", m.text);
    let mid = m.json["id"].as_str().unwrap().to_string();
    let il = call(app, "POST", &format!("/api/v1/months/{mid}/income-lines"), Some(tok), Some(json!({
        "name": "Acme", "planned_amount": 100_000, "schedule_type": "recurring",
        "recurrence_rule": {"kind": "biweekly", "anchor": "2026-09-04"}
    }))).await;
    assert_eq!(il.status, StatusCode::CREATED, "{}", il.text);
    let p1 = il.json["paychecks"][0]["id"].as_str().unwrap().to_string();
    let p2 = il.json["paychecks"][1]["id"].as_str().unwrap().to_string();
    let cats = call(app, "GET", &format!("/api/v1/months/{mid}/categories"), Some(tok), None).await;
    let housing = cats.json.as_array().unwrap().iter().find(|c| c["name"] == "Housing").unwrap()["id"].as_str().unwrap().to_string();
    let line = call(app, "POST", &format!("/api/v1/months/{mid}/expense-lines"), Some(tok), Some(json!({"category_id": housing, "name": "Rent"}))).await;
    assert_eq!(line.status, StatusCode::CREATED);
    (mid, p1, p2, line.json["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn auth_lifecycle() {
    let app = app().await;
    assert_eq!(call(&app, "GET", "/api/v1/months", None, None).await.status, StatusCode::UNAUTHORIZED);
    let reg = call(&app, "POST", "/api/v1/auth/register", None, Some(json!({"email": "Me@Example.com", "password": "secret-pass"}))).await;
    assert_eq!(reg.status, StatusCode::CREATED);
    // Single user: registration closes.
    let again = call(&app, "POST", "/api/v1/auth/register", None, Some(json!({"email": "x@example.com", "password": "secret-pass"}))).await;
    assert_eq!(again.status, StatusCode::FORBIDDEN);
    assert_eq!(again.json["error"]["code"], "REGISTRATION_CLOSED");
    let bad = call(&app, "POST", "/api/v1/auth/login", None, Some(json!({"email": "me@example.com", "password": "nope"}))).await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    let login = call(&app, "POST", "/api/v1/auth/login", None, Some(json!({"email": "me@example.com", "password": "secret-pass"}))).await;
    assert_eq!(login.status, StatusCode::OK);
    let access = login.json["access_token"].as_str().unwrap().to_string();
    let refresh = login.json["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(call(&app, "GET", "/api/v1/months", Some(&access), None).await.status, StatusCode::OK);
    // Refresh rotates.
    let r1 = call(&app, "POST", "/api/v1/auth/refresh", None, Some(json!({"refresh_token": refresh}))).await;
    assert_eq!(r1.status, StatusCode::OK);
    let new_refresh = r1.json["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(new_refresh, refresh);
    // Logout revokes the refresh token.
    call(&app, "POST", "/api/v1/auth/logout", None, Some(json!({"refresh_token": new_refresh}))).await;
    let dead = call(&app, "POST", "/api/v1/auth/refresh", None, Some(json!({"refresh_token": new_refresh}))).await;
    assert_eq!(dead.status, StatusCode::UNAUTHORIZED);
    // Logout of all devices invalidates access tokens too.
    assert_eq!(call(&app, "POST", "/api/v1/auth/logout-all", Some(&access), None).await.status, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "GET", "/api/v1/months", Some(&access), None).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn allocations_and_invariants() {
    let app = app().await;
    let tok = register(&app).await;
    let (mid, p1, p2, rent) = setup(&app, &tok).await;

    // Split Rent across both paychecks.
    let a1 = call(&app, "POST", &format!("/api/v1/paychecks/{p1}/allocations"), Some(&tok), Some(json!({"expense_line_id": rent, "amount": 50_000}))).await;
    assert_eq!(a1.status, StatusCode::CREATED, "{}", a1.text);
    assert_eq!(a1.json["safe_to_spend"], 50_000);
    let a2 = call(&app, "POST", &format!("/api/v1/paychecks/{p2}/allocations"), Some(&tok), Some(json!({"expense_line_id": rent, "amount": 50_000}))).await;
    assert_eq!(a2.json["line_planned"], 100_000);

    // Over-allocation: 409 with the spec error format.
    let over = call(&app, "POST", &format!("/api/v1/paychecks/{p1}/allocations"), Some(&tok), Some(json!({"expense_line_id": rent, "amount": 101_500}))).await;
    assert_eq!(over.status, StatusCode::CONFLICT);
    assert_eq!(over.json["error"]["code"], "INVARIANT_VIOLATION");
    assert_eq!(over.json["error"]["details"]["over_cents"], 1_500);
    assert!(over.json["error"]["message"].as_str().unwrap().contains("1500 cents"));

    // Zero / negative amounts are rejected (invariant 5).
    let zero = call(&app, "POST", &format!("/api/v1/paychecks/{p1}/allocations"), Some(&tok), Some(json!({"expense_line_id": rent, "amount": 0}))).await;
    assert_eq!(zero.status, StatusCode::CONFLICT);
    let aid = a1.json["allocation"]["id"].as_str().unwrap().to_string();
    let neg = call(&app, "PATCH", &format!("/api/v1/allocations/{aid}"), Some(&tok), Some(json!({"amount": -1}))).await;
    assert_eq!(neg.status, StatusCode::CONFLICT);

    // Lock blocked with exact remaining difference.
    let lock = call(&app, "POST", &format!("/api/v1/months/{mid}/lock"), Some(&tok), None).await;
    assert_eq!(lock.status, StatusCode::CONFLICT);
    assert_eq!(lock.json["error"]["details"]["difference_cents"], 100_000);

    // Safe-to-spend and summary.
    let sts = call(&app, "GET", &format!("/api/v1/paychecks/{p1}/safe-to-spend"), Some(&tok), None).await;
    assert_eq!(sts.json["safe_to_spend"], 50_000);
    assert_eq!(sts.json["rolling_available"], 50_000, "only the 18th is on or after Sep 10");

    // Transfer moves money between paychecks.
    let tr = call(&app, "POST", "/api/v1/allocations/transfer", Some(&tok), Some(json!({"from_paycheck_id": p1, "to_paycheck_id": p2, "expense_line_id": rent, "amount": 20_000}))).await;
    assert_eq!(tr.status, StatusCode::OK, "{}", tr.text);
    assert_eq!(tr.json["from"]["amount"], 30_000);
    assert_eq!(tr.json["to"]["amount"], 70_000);

    // Monthly-overview edit translates into allocations.
    let pl = call(&app, "PATCH", &format!("/api/v1/expense-lines/{rent}"), Some(&tok), Some(json!({"planned_amount": 200_000}))).await;
    assert_eq!(pl.status, StatusCode::OK, "{}", pl.text);
    assert_eq!(pl.json["planned"], 200_000);
    let summary = call(&app, "GET", &format!("/api/v1/months/{mid}/summary"), Some(&tok), None).await;
    assert_eq!(summary.json["is_zero"], true);

    // Reducing a paycheck cascades (invariant 6) and reports the impact.
    let red = call(&app, "PATCH", &format!("/api/v1/paychecks/{p1}"), Some(&tok), Some(json!({"planned_amount": 90_000}))).await;
    assert_eq!(red.status, StatusCode::OK);
    assert_eq!(red.json["impact"]["reduced_lines"][0], rent);
    assert_eq!(red.json["paycheck"]["allocated"], 90_000);

    // Deleting a funded paycheck removes its allocations (invariant 7).
    let del = call(&app, "DELETE", &format!("/api/v1/paychecks/{p2}"), Some(&tok), None).await;
    assert_eq!(del.status, StatusCode::OK);
    let m = call(&app, "GET", &format!("/api/v1/months/{mid}"), Some(&tok), None).await;
    let line = m.json["categories"].as_array().unwrap().iter().flat_map(|c| c["lines"].as_array().unwrap().clone()).find(|l| l["id"] == rent.as_str()).unwrap();
    assert_eq!(line["planned"], 90_000);
    assert_eq!(m.json["zero_difference"], 0);

    // Lock, then planning is frozen but transactions work.
    assert_eq!(call(&app, "POST", &format!("/api/v1/months/{mid}/lock"), Some(&tok), None).await.status, StatusCode::OK);
    let frozen = call(&app, "PATCH", &format!("/api/v1/allocations/{aid}"), Some(&tok), Some(json!({"amount": 1}))).await;
    assert_eq!(frozen.status, StatusCode::CONFLICT);
    assert_eq!(frozen.json["error"]["code"], "MONTH_LOCKED");
    let tx = call(&app, "POST", &format!("/api/v1/months/{mid}/transactions"), Some(&tok), Some(json!({
        "date": "2026-09-05", "amount": -2_500, "payee": "Landlord", "expense_line_id": rent, "paycheck_id": p1
    }))).await;
    assert_eq!(tx.status, StatusCode::CREATED, "{}", tx.text);
    let sts = call(&app, "GET", &format!("/api/v1/paychecks/{p1}/safe-to-spend"), Some(&tok), None).await;
    assert_eq!(sts.json["safe_to_spend"], -2_500);

    // Variance flow.
    call(&app, "PATCH", &format!("/api/v1/paychecks/{p1}"), Some(&tok), Some(json!({"actual_amount": 95_000}))).await;
    let v = call(&app, "GET", &format!("/api/v1/months/{mid}/variance"), Some(&tok), None).await;
    assert_eq!(v.json["net_variance"], 5_000);
    assert_eq!(call(&app, "POST", &format!("/api/v1/months/{mid}/reassignment/begin"), Some(&tok), None).await.status, StatusCode::OK);
    let fin = call(&app, "POST", &format!("/api/v1/months/{mid}/reassignment/finish"), Some(&tok), None).await;
    assert_eq!(fin.json["error"]["code"], "UNRESOLVED_VARIANCE");
    assert_eq!(call(&app, "POST", &format!("/api/v1/paychecks/{p1}/apply-actual"), Some(&tok), None).await.status, StatusCode::OK);
    let not_zero = call(&app, "POST", &format!("/api/v1/months/{mid}/reassignment/finish"), Some(&tok), None).await;
    assert_eq!(not_zero.json["error"]["details"]["difference_cents"], 5_000);
    // Re-assignment re-opens allocation editing.
    let extra = call(&app, "POST", &format!("/api/v1/paychecks/{p1}/allocations"), Some(&tok), Some(json!({"expense_line_id": rent, "amount": 95_000}))).await;
    assert_eq!(extra.status, StatusCode::CREATED, "{}", extra.text);
    let fin = call(&app, "POST", &format!("/api/v1/months/{mid}/reassignment/finish"), Some(&tok), None).await;
    assert_eq!(fin.status, StatusCode::OK, "{}", fin.text);
    assert_eq!(fin.json["status"], "locked");
    assert_eq!(fin.json["reassigning"], false);
}

#[tokio::test]
async fn months_copy_archive_delete_export_restore() {
    let app = app().await;
    let tok = register(&app).await;
    let (mid, p1, _, rent) = setup(&app, &tok).await;
    call(&app, "POST", &format!("/api/v1/paychecks/{p1}/allocations"), Some(&tok), Some(json!({"expense_line_id": rent, "amount": 40_000}))).await;

    // Duplicate year-month refused.
    let dup = call(&app, "POST", "/api/v1/months", Some(&tok), Some(json!({"year_month": "2026-09-01"}))).await;
    assert_eq!(dup.json["error"]["code"], "MONTH_EXISTS");

    // Copy structure + planned: targets, no allocations.
    let oct = call(&app, "POST", "/api/v1/months", Some(&tok), Some(json!({"year_month": "2026-10", "copy_mode": "structure_and_planned", "source_month_id": mid}))).await;
    assert_eq!(oct.status, StatusCode::CREATED, "{}", oct.text);
    let rent_oct = oct.json["categories"].as_array().unwrap().iter().flat_map(|c| c["lines"].as_array().unwrap().clone()).find(|l| l["name"] == "Rent").unwrap();
    assert_eq!(rent_oct["target"], 40_000);
    assert_eq!(rent_oct["planned"], 0);
    let oct_id = oct.json["id"].as_str().unwrap().to_string();

    // Income suggestions come from history.
    let sug = call(&app, "GET", &format!("/api/v1/months/{oct_id}/income-suggestions?q=acm"), Some(&tok), None).await;
    assert_eq!(sug.json[0]["name"], "Acme");
    assert_eq!(sug.json[0]["planned_amount"], 100_000);

    // Past and future months are allowed.
    assert_eq!(call(&app, "POST", "/api/v1/months", Some(&tok), Some(json!({"year_month": "1999-01"}))).await.status, StatusCode::CREATED);

    // Archive hides, restore shows.
    call(&app, "POST", &format!("/api/v1/months/{oct_id}/archive"), Some(&tok), None).await;
    let list = call(&app, "GET", "/api/v1/months", Some(&tok), None).await;
    assert_eq!(list.json.as_array().unwrap().len(), 2);
    let arch_edit = call(&app, "POST", &format!("/api/v1/months/{oct_id}/categories"), Some(&tok), Some(json!({"name": "X"}))).await;
    assert_eq!(arch_edit.json["error"]["code"], "MONTH_ARCHIVED");
    let list = call(&app, "GET", "/api/v1/months?include_archived=true", Some(&tok), None).await;
    assert_eq!(list.json.as_array().unwrap().len(), 3);
    call(&app, "POST", &format!("/api/v1/months/{oct_id}/restore"), Some(&tok), None).await;

    // Permanent delete requires typing the month.
    let no = call(&app, "DELETE", &format!("/api/v1/months/{oct_id}?confirm=nope"), Some(&tok), None).await;
    assert_eq!(no.status, StatusCode::BAD_REQUEST);
    assert_eq!(call(&app, "DELETE", &format!("/api/v1/months/{oct_id}?confirm=2026-10"), Some(&tok), None).await.status, StatusCode::NO_CONTENT);

    // CSV export is deterministic.
    let csv1 = call(&app, "GET", &format!("/api/v1/months/{mid}/export"), Some(&tok), None).await;
    let csv2 = call(&app, "GET", &format!("/api/v1/months/{mid}/export"), Some(&tok), None).await;
    assert_eq!(csv1.status, StatusCode::OK);
    assert_eq!(csv1.text, csv2.text);
    assert!(csv1.text.contains("allocation,2026-09,2026-09-04,Acme,Housing,Rent"));

    // Snapshot -> restore with replace.
    let snap = call(&app, "GET", &format!("/api/v1/months/{mid}/snapshot"), Some(&tok), None).await;
    let conflict = call(&app, "POST", "/api/v1/months/import", Some(&tok), Some(json!({"snapshot": snap.json}))).await;
    assert_eq!(conflict.json["error"]["code"], "MONTH_EXISTS");
    let restored = call(&app, "POST", "/api/v1/months/import", Some(&tok), Some(json!({"snapshot": snap.json, "replace": true}))).await;
    assert_eq!(restored.status, StatusCode::CREATED, "{}", restored.text);
    assert_ne!(restored.json["id"], mid);
    assert_eq!(restored.json["total_planned_expense"], 40_000);

    // Reports.
    let new_id = restored.json["id"].as_str().unwrap();
    let ytd = call(&app, "GET", &format!("/api/v1/months/{new_id}/reports/ytd"), Some(&tok), None).await;
    assert_eq!(ytd.json["figures"]["planned_income"], 200_000);
    let mom = call(&app, "GET", &format!("/api/v1/months/{new_id}/reports/mom"), Some(&tok), None).await;
    assert_eq!(mom.json["other"], Value::Null);
    let cards = call(&app, "GET", &format!("/api/v1/months/{new_id}/reports/cards"), Some(&tok), None).await;
    assert_eq!(cards.json["remaining_to_zero"], 160_000);
}

fn cookie_header(cookies: &[String]) -> String {
    cookies.iter().filter(|c| !c.ends_with('=')).cloned().collect::<Vec<_>>().join("; ")
}

async fn ui_login(app: &Router) -> Vec<String> {
    let req = Request::builder()
        .method("POST")
        .uri("/ui/register")
        .header("datastar-request", "true")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("email=me%40example.com&password=secret-pass&timezone=America%2FChicago"))
        .unwrap();
    let r = send(app, req).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text.contains("window.location.assign"), "{}", r.text);
    r.cookies
}

#[tokio::test]
async fn ui_session_csrf_and_silent_refresh() {
    let app = app().await;
    // Unauthenticated page load redirects to login.
    let r = send(&app, Request::builder().uri("/months").body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let cookies = ui_login(&app).await;
    let jar = cookie_header(&cookies);
    let r = send(&app, Request::builder().uri("/months/content").header(header::COOKIE, &jar).body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.text.starts_with("event: datastar-patch-elements"));
    assert!(r.text.contains("September 2026"), "onboarding created the current month");

    // Mutations without the Datastar header are refused (CSRF).
    let r = send(&app, Request::builder().method("POST").uri("/ui/months").header(header::COOKIE, &jar)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded").body(Body::from("year_month=2026-10")).unwrap()).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Only the refresh cookie: the session is silently refreshed.
    let refresh_only = cookies.iter().find(|c| c.starts_with("pz_rt=")).unwrap().clone();
    let r = send(&app, Request::builder().uri("/months/content").header(header::COOKIE, &refresh_only).body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.cookies.iter().any(|c| c.starts_with("pz_at=") && c.len() > 6), "new access cookie issued");
}

#[tokio::test]
async fn offline_sync_applies_and_detects_conflicts() {
    let app = app().await;
    let cookies = ui_login(&app).await;
    let jar = cookie_header(&cookies);
    // Find the onboarding month.
    let state_months = send(&app, Request::builder().uri("/months/content").header(header::COOKIE, &jar).body(Body::empty()).unwrap()).await;
    let mid = state_months.text.split("href=\"/months/").nth(1).unwrap().split('"').next().unwrap().to_string();

    let sync = |body: Value| {
        Request::builder()
            .method("POST")
            .uri("/sync")
            .header(header::COOKIE, &jar)
            .header("x-pz-sync", "1")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let tx_id = "11111111-1111-4111-8111-111111111111";
    let tx = json!({"date": "2026-09-12", "amount": -1_000, "payee": "Cafe"});
    let r = send(&app, sync(json!({"ops": [{"op_id": "a", "kind": "create_transaction", "month_id": mid, "id": tx_id, "tx": tx}]}))).await;
    assert_eq!(r.json["results"][0]["status"], "applied", "{}", r.text);
    // Replaying the same op is idempotent.
    let r = send(&app, sync(json!({"ops": [{"op_id": "a", "kind": "create_transaction", "month_id": mid, "id": tx_id, "tx": tx}]}))).await;
    assert_eq!(r.json["results"][0]["status"], "applied");
    // Edit based on a stale copy conflicts...
    let stale = json!({"date": "2026-09-12", "amount": -9_999, "payee": "Cafe"});
    let edit = json!({"date": "2026-09-12", "amount": -2_000, "payee": "Cafe"});
    let r = send(&app, sync(json!({"ops": [{"op_id": "b", "kind": "update_transaction", "month_id": mid, "id": tx_id, "base": stale, "tx": edit}]}))).await;
    assert_eq!(r.json["results"][0]["status"], "conflict");
    assert_eq!(r.json["results"][0]["server"]["amount"], -1_000);
    // ...and "keep mine" forces it.
    let r = send(&app, sync(json!({"ops": [{"op_id": "c", "force": true, "kind": "update_transaction", "month_id": mid, "id": tx_id, "base": stale, "tx": edit}]}))).await;
    assert_eq!(r.json["results"][0]["status"], "applied");
}
