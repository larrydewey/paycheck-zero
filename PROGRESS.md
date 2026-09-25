# PaycheckZero — Implementation Progress

Read this first every session. Spec: `PaycheckZero_Formal_Specification_v0.4.4.md` (frozen).

## Status (2026-09-25, branch `v1-build`)

Everything in the spec is implemented and tested. **Not yet "v1 complete"** under §13.10: all Playwright tests pass, but core flows have not been checked on real devices (emulation only, which §13.11 allows for now).

| Area | State |
|------|-------|
| Domain (`core`) | Done — 46 tests |
| Storage (`storage`) | Done — SQLite, PostgreSQL 17, MariaDB 11 verified |
| REST API §9, auth §13.5, sync §13.7 | Done — 5 in-process HTTP suites |
| Datastar UI §3/§14, reports §15, exports §13.6, PWA | Done |
| Playwright §13.1 | 350 runs across 4 browsers passing, stable over repeated runs (earlier suite also passed on PostgreSQL and MariaDB) |
| Accessibility | axe WCAG 2.0/2.1 A+AA: zero violations on every screen |

## Decisions (agreed with the user before building)

1. Rebuilt storage and web from scratch; the earlier core was rewritten too (it had an allocation-update bug and lacked recurrence, variance, copy and currency). The pre-rebuild snapshot is commit `86aa219`.
2. §2.11 "copy structure + planned": planned amounts are copied as a non-binding **target** per line (`expense_lines.target_amount`). Planned stays derived; the paycheck view offers one-click "Fund $X".
3. Overview Planned edit (§14.6): increases come from the earliest paycheck with unassigned money, then later ones. If there isn't enough, it's refused (409 / "you're $X short"). Decreases come off the latest-dated allocations first.
4. Variance (§2.9): "Use actual as planned" sets planned = actual, with the invariant 6 cascade. On a locked month, "Re-assign variance" opens allocation-only editing. "Done — re-lock" is allowed only once every variance is applied and the month is at exactly zero.
5. Skipped paychecks count as no income and have no Safe-to-Spend.
6. §2.10 Spent is literal: the sum of absolute values of all linked transactions, so refunds add to Spent.
7. Offline: the service worker caches pages and their SSE content (network first). Transaction create/edit/delete and paycheck actuals are queued in IndexedDB and replayed through `POST /sync`. Each op carries a base copy; if the server changed meanwhile, the user sees a conflict and picks "Keep mine" (force) or "Keep the other version". Allocation edits are online-only.
8. Engines: sqlx `Any` driver with one portable schema (VARCHAR ids/dates, BIGINT cents, table-level FKs, `"year_month"` quoted; MariaDB sessions use `ANSI_QUOTES`).
9. Auth: HttpOnly cookies for the web UI (access 15 min, refresh 30 days, rotating, stored hashed) with silent refresh in middleware. Bearer tokens for the API. "Log out of all devices" deletes refresh tokens and bumps a token version.
10. Single user: registration closes once an account exists.
11. Recurrence is our own JSON: weekly/biweekly by anchor date, semi-monthly (2 days), monthly (days; past month end clamps). Editing a schedule keeps paychecks whose dates still match, adds new ones, and deletes the rest with their allocations (with a notice). A new line amount flows only to paychecks still at the old default.
12. Currencies: 20 two-decimal ISO currencies, en-US formatting. A currency change converts every month with a user rate, rounding half-even. Each paycheck is converted with largest-remainder distribution, so fully allocated paychecks stay exactly allocated.
13–15. Browsers installed. Real devices not tested. One commit per area.

## Other interpretations / deviations

- §9 says "FastAPI"; §7.2 mandates Rust, so the REST contract is served by Axum.
- The schema follows §8 with additive columns: users `timezone`, `currency`, `token_version`, `last_month_id`; months `reassigning`, `archived_at`, `version`; category `kind`; line `sort_order`, `target_amount`; `position` on ordered children. There are also `refresh_tokens`, `sync_ops` and `schema_migrations` tables. The obsolete `db/*.sql` (v0.4.3) scripts were removed; the source of truth is `storage/src/schema.rs`.
- Report definitions (§15): actual income = paycheck actuals + unlinked positive transactions. Actual spending = line Spent + unlinked expenses (shown as "Uncategorized"). Categories are matched across months by name.
- "Today" is the user's timezone date. In test mode it is pinned by `/__test/reset`.
- Month-level `total_spent` / CSV month row counts line Spent only (§2.10), not unlinked expenses.
- UI mutation endpoints require the `Datastar-Request` header (CSRF defence for cookie auth).

## UX polish pass (2026-09-25)

A screenshot tour of every screen on desktop and phone (`npx playwright test -c tour.config.ts` in `e2e/`, output in `e2e/tour-shots/`) drove these changes:

- Phones get a bottom tab bar (top tabs were cut off) and one sticky column header instead of labels on every row.
- Transactions are a compact list; editing opens a dialog; Expense/Income is a toggle.
- The variance flow is one panel with numbered steps. The month banner no longer says "Every dollar has a job" while a variance is pending.
- Summary cards: income vs plan counts only paychecks received so far (no more "−$4,000" before payday); spending shows a progress meter.
- Reports have a planned-vs-spent bar chart, and all-zero rows are hidden.
- Assign form: tidy grid and a "Use all $X" shortcut; it's hidden once the paycheck is fully assigned.
- Styled confirmation dialogs, a saving indicator and progress bar, SVG icons, drag-and-drop reordering on the overview (mouse; move buttons remain for keyboard/phones), grouped timezone picker, dark mode (axe-checked), and "no income yet" status for empty months.

Bugs this caught:

- An edit dialog stayed modal after saving, leaving the page inert. The dialog is now closed before the server re-renders.
- A late income-suggestion response re-opened the box after "No thanks". Dismissal is now sticky until the user types again.
- Error toasts covered the last controls on phones; they now sit at the top.

## Round 2: owner feedback (2026-09-25)

| # | Request | Done |
|---|---------|------|
| 1 | Add-line (and other add forms) should clear after submit | Add forms empty after success; a rejected add keeps what you typed |
| 2 | Automatic 10% tithing | Owner chose one-click: "Give 10% ($X)" per paycheck funds Giving → Tithe (created if missing), rounded half-even |
| 3 | Save on mouse leave | Changed inline fields save when the pointer leaves the row (blur/Enter still work) |
| 4 | Field math (600+80) | `+ - * / ( )` in every amount field, exact fractions, same evaluator on server and in browser; field shows the result |
| 5–7 | Edit per-paycheck funding on the month page, show zeros, choose paycheck when adding | Owner chose expandable: each line has "By paycheck", with one editable amount per paycheck ($0.00 included); "Add a line" takes an amount and a paycheck |
| 8 | Income tagged to a paycheck should reconcile "Actually received" | Deposits (income tagged to a paycheck) set its actual and mark it Received; reports don't double-count |
| 9 | Split transactions | Parts across lines *and* paychecks, total must match, edit/delete as one; schema v2 adds `transactions.split_group` |
| 10 | Overspending warnings | Month banner, row highlight + "Over by $X", category flag, toast when a transaction overspends, negative Safe-to-Spend warning |
| 11 | Granular reports, exporting | Report tabs: Summary (category → line drill-down), Trends (3/6/12 months, categories or lines, actual or planned), Payees (any date range), Export (month CSV/JSON, transactions for any date range, every report as CSV) |
| 12 | Android doesn't log in | Not reproducible (login works in Android emulation, including over a LAN IP). Hardened: sign-in works without JavaScript, explains dropped cookies and script failures, and the server logs each attempt with the user agent. Needs a retry on the device |

## Open question for the product owner

- **Safe-to-Spend (§2.7):** the formula is `planned − allocations − tagged spending`. A properly fully-assigned paycheck therefore always shows `$0 − spending`, i.e. a negative number as soon as anything is tagged. Implemented literally (the spec is frozen). The likely intent is `planned − tagged spending` (or `allocations − tagged spending`); it needs a decision.

## Known limitations

- Playwright's WebKit build needs Ubuntu 24.04 libraries. On this Omarchy host it runs through Playwright's Docker image automatically (`e2e/global-setup.ts`).
- Playwright's WebKit offline emulation also blocks service-worker cache hits. On mobile Safari the test therefore asserts offline readiness (worker in control, page and content cached). Actual offline serving is asserted on Chromium, Firefox and mobile Chrome.
- JWT secret defaults to random per start; set `PZ_JWT_SECRET` in production.

## Commands

```bash
make test                  # clippy + cargo tests + Playwright (all browsers)
cargo run -p paycheckzero-web
cd e2e && npm test         # single command E2E
cd e2e && npm run test:update   # refresh visual baselines
```

## Next steps

- Real-device pass on iOS Safari and Android Chrome (release gate §13.10).
- Optional: CI workflow running `make test` (Postgres/MariaDB services).
