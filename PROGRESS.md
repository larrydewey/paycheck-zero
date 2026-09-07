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

## Known TODO / next session
- [x] Run `cargo test` for core; ensure green. (23/23 passed)
- [x] `storage/`: SQLite adapter (rusqlite), load/save `Month` aggregate.
- [x] `storage/`: Round-trip tests (5/5 passed).
- [ ] `web/`: Axum routes per §9, JWT (access+refresh), invariant re-validation → 409, Datastar HTML+SSE, CSV export.
- [ ] Reports MoM/YTD/YoY/summary (§15) + CSV/snapshot export (§13.6).
- [ ] Reports MoM/YTD/YoY/summary (§15) + CSV/snapshot export (§13.6).
- [ ] Datastar frontend: paycheck view (default landing, safe-to-spend prominent, inline edit), monthly overview, collapsible categories w/ live summaries, empty/skeleton/error/offline states.
- [ ] Playwright E2E (§13.1): grouped by feature, reset+seed per run, a11y (axe) + visual.
- [ ] PWA manifest + service worker for offline.

## Commands
- `cargo test` (workspace tests)
- `cargo run -p paycheckzero-web` (start server, once web crate exists)

## Conventions
- TDD: write failing test → minimal impl → refactor (spec §11).
- Rust style per rust-skills (no unwrap in prod, thiserror for core, newtype ids, etc.).
