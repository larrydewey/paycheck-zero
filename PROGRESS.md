# PaycheckZero — Implementation Progress

Persistence log for multi-session work. **Read this first every session.**
Spec source of truth: [`PaycheckZero_Formal_Specification_v0.4.4.md`](./PaycheckZero_Formal_Specification_v0.4.4.md) (v0.4.4, frozen).

## Stack (per spec §7)
- Rust workspace: `core/` (pure domain), `storage/` (SQLite adapter), `web/` (Axum + Datastar)
- SQLite primary (schema already in `db/`), integer cents money
- Datastar frontend, JWT auth, Playwright E2E
- Toolchain confirmed: rustc 1.98, node 26.5 (Playwright), sqlite 3.53

## Status Legend
- [ ] not started · [~] in progress · [x] done (tests green)

## Work log (newest at bottom)

### Session 1 — scaffold + pure domain crate
- [x] Explored spec + environment. Confirmed toolchain. Noted prior Python venv is a non-conformant leftover (spec mandates Rust) — left untouched.
- [x] Created `PROGRESS.md`.
- [x] Cargo workspace scaffold (`Cargo.toml` + `core` crate).
- [x] Pure domain crate: `money`, `id`, `models`, `error`, `domain` (invariants + safe-to-spend + zero validation + cascades).
- [x] `cargo test -p paycheckzero-core` — 23/23 passed.

### Session 2 — storage crate (SQLite adapter)
- [x] Scaffolding `storage/` crate with `Repository` trait (`repo.rs`).
- [x] SQLite backend (`sqlite.rs`): in-memory schema, load/save `Month`, `list_months`.
- [x] All compilation errors fixed (type inference, param expansion, mutability).
- [x] `cargo build --workspace` — clean (zero errors, zero warnings).
- [x] Round-trip tests: 5/5 passed (save/load, optional fields, list, overwrite, not-found).

### Session 3 — REST API (§9) + JWT auth (§13.5) + reports (§15)
- [x] §9 REST: full CRUD surface (`web/src/routes/months.rs`) incl. income-lines, paychecks, allocations/transfer, categories, expense-lines, transactions, safe-to-spend, month summary; 6 `find_month_by_*` helpers.
- [x] JWT auth (§13.5): `users` + `refresh_tokens` tables, `AuthRepository`, access (15 min) + refresh (30 d, sha256-hashed, rotated) tokens, argon2, `register/login/refresh/logout`, `require_auth` middleware on `/api/v1`.
- [x] Reports (§15): `web/src/report.rs` pure computations (MoM, YTD, YoY, summary cards) + routes under `/months/{month_id}/reports/…`.
- [x] Fixed deadlock: `find_month_by_*` held the DB Mutex across loop bodies → hoisted `list_months()` before iterating, nested `.lock()` per item.
- [x] Live smoke verified: auth lifecycle, rotations/replay, seeded June/July via API, all reports correct.

### Session 4 — Datastar frontend (§14)
- [x] `web/src/ui.rs`: fragment renderers (login page, dashboard, month view) — summary cards, paycheck-first view with prominent safe-to-spend (§14.5), per-paycheck editable planned + allocations (PATCH/POST + refetch), category overview with live planned/spent/remaining (§14.3) and inline line rename.
- [x] `web/src/routes/ui.rs`: SSE `datastar-merge-fragments` endpoints — public `/ui/session` (login or dashboard by Authorization), guarded `/ui/month/{id}`.
- [x] `web/templates/index.html` Datastar shell: CDN bundle 0.26.2, localStorage tokens, sync refresh-at-boot, `data-headers` Authorization on every fragment action.
- [x] Tests/clippy: workspace 46 tests green, clippy `-D warnings` clean (incl. fixing pre-existing `.into()`/sort lints).
- [x] Live smoke: shell serves; session returns login without token, dashboard + month-view with token; the exact Datastar PATCH/POST payloads round-trip and safe-to-spend recomputes.
- [~] UI polish still open (skeleton phase): zero/empty/error/offline states, register landing, transaction + expense-line creation flows, month locking UX, collapse-toggle persistence.

## Architecture decisions
- **Money**: `Cents(i64)` newtype, integer only, no floats. Display conversion only at UI boundary (web crate).
- **Ids**: `Id` newtype over `String` (UUID text). `Id::generate()` via uuid v4.
- **Domain = in-memory aggregate**: `Month` owns all children. Storage loads a `Month` aggregate, service mutates it with invariant checks, storage persists. Keeps `core` free of DB/web deps (spec §7.1).
- **Derived (never stored)**: line `planned` (sum of allocations), line `spent` (sum abs of negative txns linked to line), paycheck `allocated`, `safe_to_spend`, rolling, zero status.
- **Zero check**: `sum(paycheck.planned) - sum(all allocations)`. Zero ⟺ every paycheck fully allocated (invariant 2 + 3).
- **Spent**: matches provided DB view `v_expense_line_spent` (sum of `-amount` for `amount<0` linked txns).
- **Cents→dollars** handled in web/JSON only.

## Spec interpretation notes (frozen spec ambiguities)
- §2.10 "Spent = sum of absolute values of all Transactions" — implemented as negatives-only (expenses), matching the shipped DB views; in practice only expense txns link to a line.
- §9 header says "FastAPI" but §7.2 + README mandate Rust — using Rust/Axum; "REST" contract preserved.
- Invariant 6 (reduce planned below allocations): domain reduces/deletes allocations in deterministic order to fit new planned, returns affected lines for UI notification.
- Fragments served as hand-rolled `datastar-merge-fragments` SSE events (stable since Datastar 0.15, matches 0.26.2 in shell) instead of the `datastar` crate — no extra dependency.

## Known TODO / next session
- [x] Run `cargo test` for core; ensure green. (23/23 passed)
- [x] `storage/`: SQLite adapter (rusqlite), load/save `Month` aggregate.
- [x] `storage/`: Round-trip tests (5/5 passed).
- [x] `web/`: Axum routes per §9 (full CRUD), JWT (access+refresh) with rotation + logout, invariant re-validation → 409, CSV export, Datastar HTML+SSE fragments.
- [x] Reports MoM/YTD/YoY/summary (§15) + CSV/snapshot export (§13.6).
- [~] Datastar frontend: paycheck view (default landing, safe-to-spend prominent, inline edit) + monthly overview + collapsible categories w/ live summaries DONE at skeleton level; still open: empty/skeleton/error/offline states, registering-from-UI flow, transaction & expense-line creation flows, month locking UX.
- [ ] Playwright E2E (§13.1): grouped by feature, reset+seed per run, a11y (axe) + visual.
- [ ] PWA manifest + service worker for offline.

## Commands
- `cargo test` (workspace tests)
- `cargo run -p paycheckzero-web` (start server, once web crate exists)

## Conventions
- TDD: write failing test → minimal impl → refactor (spec §11).
- Rust style per rust-skills (no unwrap in prod, thiserror for core, newtype ids, etc.).
