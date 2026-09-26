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

## Safe-to-Spend decision (2026-09-25)

The owner asked for "whatever makes the best user experience". Safe-to-Spend is now `planned − allocations − unplanned spending`. Spending tagged to a paycheck on a line that paycheck funds draws from that allocation first. Only spending beyond it, or on unfunded or unlinked lines, lowers Safe-to-Spend.

This removes the literal §2.7 double count, where every fully assigned paycheck went negative. A new "Left in its lines" stat shows what the paycheck still has budgeted and unspent.

## Mobile-first redesign (2026-09-25)

Feedback: "too involved… feels like the website was shoved into a smaller window." The UI was rebuilt phone-first.

- **Navigation.** Four sections: Plan (one paycheck), Budget (the month), Spending (transactions) and Insights (reports). They appear as a bottom tab bar on phones and top tabs on wide screens. The app bar holds the month switcher and settings. Income is reached from Plan ("Manage income and paychecks").
- **One headline per screen.** Plan leads with Safe to Spend, a progress meter and a single "Assign $X" button. Budget leads with income, planned and spent. Spending has a search box and a list grouped by day.
- **Calm rows.** Each line shows its name, what's left and a progress bar, with the amount edited in place. Everything else is one tap away in a bottom sheet (swipe down or tap outside to close):
  - line details: funding per paycheck, debt fields, recent transactions, rename, move to another category, reorder, delete;
  - assign (including Give 10% and copied targets);
  - paycheck details;
  - add or edit a transaction;
  - category edit;
  - new line.
- **Floating "Add a transaction" button** on every month screen.
- **Empty categories** collapse into "+ Name" chips instead of empty cards.
- **Insights.** Summary shows cards and charts only. The comparison tables moved to a Compare tab.
- **Wide screens.** A two-column layout with the summary pinned on the left.
- **Offline.** Month pages carry offline copies of the add/edit transaction and record-actual sheets. Other sheets say they need a connection. The service worker's caches are versioned by a hash of the assets, so a new release never leaves phones on stale CSS or JS.
- **Deviations from earlier rounds.** Line names are edited in the line sheet, not inline. Drag-and-drop was removed; line sheets use move buttons and a category picker instead. Moving a category steps past empty categories, so the visible order always changes.

## Accounts, cards, transfers, goals (2026-09-25)

The owner chose all four recommended designs, and asked that transfers be first-class and that the "Spending" tab be called "Transactions".

- **Navigation.** Five tabs: Plan, Budget, Transactions, Accounts, Insights. A light/dark toggle in the app bar cycles Match device → Light → Dark, and Settings has the same choice. It's saved per device with no flash on load.
- **Transactions that need a line.** Spending with no line is flagged in several places:
  - a count badge on the Transactions tab;
  - an alert on Plan and Budget ("1 transaction ($12.00) needs a line · Sort them");
  - a "Needs a line" filter chip;
  - an amber marker and pill on each such row.

  Plan also shows "Recent from this paycheck", so the plan and what actually happened sit side by side.
- **Accounts (manual + reconcile).** Checking, savings, cash and credit cards live in a user-level wallet (schema v3: `accounts`, `account_adjustments`, `goals`, plus `transactions.account_id` and `transfer_account_id`).
  - A balance is its adjustments plus every transaction on the account, across all months.
  - Reconciling records the gap to the bank as an adjustment.
  - Accounts with history can only be archived, not deleted.
  - Budget shows the balances and the net.
- **Transfers.** A transfer moves money between two accounts. It is never budget spending unless it is linked to a line (paying down debt that wasn't budgeted). It doesn't touch Safe-to-Spend or payee reports.
- **Credit cards (budget-aware).** Purchases count against their lines.
  - Each card shows what's owed, "ready to pay" (this month's line-budgeted card spending not yet paid) and "carried" debt.
  - It also shows utilization, the minimum payment still due, and spending that needs a line.
  - "Pay $X" opens a prefilled transfer.
- **Goals.** Save-up goals follow a budget line (what's planned each month, plus an optional starting amount) or an account balance. Pay-off goals follow a card or a debt line.
  - Each goal shows its progress, its status (On track / Behind / Done / No deadline), what this month needs, and bars for the last six months.
  - "Plan $X more" tops up the line in one tap. The same appears in the line's sheet.
- **CSV export.** Gains `account` and `transfer_account` columns.
- **Fix.** Quick-add rows no longer shift the layout under the next tap after a submit.

## Retirement and investment accounts (2026-09-25)

- **New kinds: Retirement** (401(k), IRA) **and Investment** (brokerage), shown in their own "Retirement & investments" group.
- **Net worth.** The Accounts screen leads with net worth (cash + invested − cards owed).
- **Updating a balance** from a statement records the difference as market growth, not spending and not a correction.
- **Payroll contributions** (pre-tax, never part of take-home pay) are recorded on the account and don't touch the budget.
- **Contributions from checking** are transfers ("Contribute from checking"). Linking one to a budget line makes it planned saving.
- **Each account** shows what was added and its growth this calendar year, plus an activity list. Contribution and growth entries can be deleted.
- **Transaction picker.** Retirement accounts aren't offered for everyday spending.
- **Schema v4** rebuilds `accounts` / `account_adjustments` to widen their CHECK constraints (create, copy, drop, rename). Tested on a copy of real data.
- **Fix.** Desktop toasts no longer cover the floating Add button.

## Plan is one paycheck on its own (2026-09-25)

Owner feedback: each paycheck should show only what that paycheck is doing, and the monthly view covers all of them.

- Plan now lists only the lines this paycheck funds.
- A line's Planned is this paycheck's share, and Spent is spending tagged to this paycheck.
- Categories without funding from this paycheck, empty-category chips and the "only funded" toggle are gone.
- The month-wide items moved to Budget: the zero status, overspending, needs-a-line alerts and the "available from today" stat. Exceptions:
  - While a variance is open or the month is locked, the zero status and variance panel still show on Plan, because they hold the re-lock flow.
  - A "See the whole month" link leads to Budget.
- Lines this paycheck doesn't fund yet are added through Assign or New line. Quick-add on Plan requires an amount.

## Bank sync: Teller, SimpleFIN, Plaid (2026-09-25)

All providers are optional and share one pipeline:
- **Provider layer.** Each provider (`web/src/bank.rs`) is reduced to the same accounts, transactions and balances.
- **Import rules.** Importing (`core/src/bank.rs`, unit-tested) follows one set of rules:
  - dedupe by bank id, plus a `bank_seen` ledger;
  - link hand-entered transactions (same amount, ±3 days);
  - learn lines from the last transaction with the same payee;
  - tag spending to the current paycheck, and deposits that look like a paycheck (±3 days, ±25%), which records the actual;
  - pair an outflow and an inflow across connected accounts into one transfer.
- **After import:** each account is reconciled to the bank balance.

Providers:
- **Teller:** Connect in the browser, then mTLS API calls.
- **SimpleFIN:** paste a setup token, which is claimed once for an access URL. No server config, so it's always available.
- **Plaid:** a Link token from the server, a public-token exchange, `/transactions/sync` with a stored cursor, and removed transactions deleted.

Behavior:
- Pending transactions are skipped.
- Months that don't exist yet are reported and imported later.
- Expired logins show "Sign in again": Teller re-runs Connect with the enrollment, Plaid uses update mode, and SimpleFIN takes a new token.
- The Connect and Link windows open after the sheet closes (a modal sheet would block them).
- Tokens are sealed with ChaCha20-Poly1305 using `PZ_DATA_KEY` (or an auto-created key file).
- Schema v5 adds `external_id` columns, `bank_links` and `bank_seen`.
- Test mode includes fake Teller, SimpleFIN and Plaid servers, so the E2E suite covers every flow without real banks.
- Fix: incremental sync looks back 30 days from the app's "today" (it had used the wall clock).

## Settings → Bank providers (2026-09-25)

Teller and Plaid can be turned on from the app instead of environment variables.
- **Teller:** application ID, environment, and certificate and key uploads. The files are read in the browser, or can be pasted.
- **Plaid:** client ID, secret, environment and countries.
- **Checks on save:** Plaid keys are verified with a link-token call; the Teller certificate and key are parsed as a client identity; outside sandbox a certificate is required.
- **Storage:** saved settings are encrypted in the new `app_settings` table (schema v6). They take effect immediately and override the environment.
- **Secrets are never rendered back.** A blank secret keeps the saved one, and forms clear after saving.
- **Connect sheet:** when a provider is off, it links to Settings.
- **Background sync** now runs even without Teller or Plaid, because SimpleFIN is always available.
- **Tests:** a test-mode reset flag ignores the environment providers so the UI flow can be tested. The fixtures include a self-signed test certificate.

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
