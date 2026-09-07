# PaycheckZero – Formal Product & Technical Specification
**Version 0.4.4 (Frozen)**  
**Status**: Implementation-ready. Any behavior not defined in this document is forbidden.

This document is the single source of truth. An implementing LLM or engineer must follow it exactly. All behavior must be fully tested via Test-Driven Development (TDD) and Playwright end-to-end tests as defined in Section 13.

---

## 1. Product Identity

**Internal name**: PaycheckZero  

**One-sentence definition**:  
A strict zero-based budgeting application in which the user constructs the entire monthly budget exclusively by deciding, paycheck by paycheck, what each paycheck will fund. The monthly totals are always derived from those paycheck-level decisions and must sum to exact zero.

**Licensing**: Open-source.

**Platforms (v1)**:
- Full web application (desktop + responsive mobile web) powered by Datastar
- Progressive Web App (PWA) support for mobile browsers
- Native iOS / Android clients are out of scope for v1

**Naming / Branding Direction**: The final product name and marketing language must clearly signal “paycheck-first zero-based budgeting.”

---

## 2. Core Domain Model (Authoritative)

All monetary values are stored and calculated as **integer cents** (or equivalent smallest currency unit). Floating-point arithmetic for money is forbidden.

**Presentation boundary (mandatory):**
- Backend, domain crate, database, and any JSON/API payloads use **integer cents only**.
- The frontend (Datastar UI) **must** present and accept amounts in **dollars and cents** (e.g. `12.50`, `$1,234.56`) using proper locale formatting.
- Conversion between display dollars and integer cents happens only at the UI boundary. The domain never sees floating-point money.

### 2.1 Month
- Identified by `year-month` (e.g., 2026-09).
- Contains zero or more `IncomeLine`s, which generate `Paycheck`s.
- Contains `ExpenseLine`s whose planned amounts are **derived**.
- State: `Draft` | `Locked`.
- A Month may transition to `Locked` **only** when:  
  `sum(all Paycheck.planned_amount) − sum(all derived ExpenseLine.planned_amount) == 0`.
- Once `Locked`, planned amounts and allocations cannot be changed except through the variance / re-assignment flow.
- **Users must be able to create, select, edit (while Draft), and lock Months for any year-month — past, current, or future.** The concept of “today” is used only for derived views (e.g. rolling Safe-to-Spend) and must never prevent working in a different Month.

### 2.2 IncomeLine
- Unlimited per Month.
- Fields:
  - `id` (UUID)
  - `name` (string, 1–100 characters, required)
  - `planned_amount` (integer cents ≥ 0)
  - `schedule_type`: `one_off` | `recurring`
  - `recurrence_rule` (required if recurring; must support common patterns: weekly, bi-weekly, semi-monthly, monthly on specific days)
  - `expected_dates[]` (concrete dates inside the Month)
- Smart suggestion (v1 required): When creating an IncomeLine, the system must propose name, amount, and recurrence based on historical IncomeLines with the same or similar name from prior months. User may accept, edit, or discard.

### 2.3 Paycheck
- Concrete instance of an IncomeLine on a specific date.
- Generated automatically from `IncomeLine.expected_dates`.
- Fields:
  - `id`
  - `income_line_id`
  - `date`
  - `planned_amount` (integer cents ≥ 0; defaults to parent but can be overridden)
  - `actual_amount` (nullable integer cents)
  - `status`: `planned` | `received` | `skipped`

### 2.4 ExpenseCategory
- Hierarchical container.
- Seeded on first use of any Month with: Giving, Saving, Housing, Transportation, Food, Personal, Lifestyle, Health, Insurance, Debt, Other.
- User may rename, reorder, add, or delete categories and lines freely while Month is `Draft`.

### 2.5 ExpenseLine
- Belongs to one ExpenseCategory.
- Fields:
  - `id`
  - `category_id`
  - `name`
  - `planned_amount` → **always derived** as `sum(Allocation.amount for this line)`. Never stored independently.
  - For Debt lines only (v1): additional fields `current_balance` (cents) and `minimum_payment` (cents).

### 2.6 Allocation (Source of Truth)
```
Allocation {
  id
  expense_line_id
  paycheck_id
  amount          // integer cents > 0
}
```

**Invariants (must be enforced client-side and server-side at all times):**

1. `ExpenseLine.planned_amount == sum of its Allocations` (derived, never independent).
2. For every Paycheck P: `sum(Allocations of P) ≤ P.planned_amount`.
3. **A Paycheck is only considered fully valid when it is completely allocated** (`sum(Allocations of P) == P.planned_amount`). Leaving money unallocated is not a normal end state; the UI must actively push the user to allocate everything.
4. For the Month: `sum(all Paycheck.planned_amount) − sum(all derived ExpenseLine.planned_amount) == 0` before locking is allowed.
5. An Allocation amount may never be zero or negative. To remove funding, the Allocation is deleted.
6. When a Paycheck’s `planned_amount` is reduced below the sum of its current Allocations, the system must immediately reduce or delete excess Allocations (user is shown the affected lines and must resolve any resulting under-funding).
7. When a Paycheck is deleted or marked `skipped`, all its Allocations are deleted and the affected ExpenseLines’ derived planned amounts decrease accordingly. The user is notified and must re-balance to zero if necessary.

### 2.7 Safe-to-Spend (Authoritative Formulas)

**Per-paycheck (primary daily number):**
```
safe_to_spend(P) = P.planned_amount 
                 − sum(Allocations assigned to P) 
                 − sum(absolute value of expense transactions **explicitly tagged** to P)

Only transactions that have `paycheck_id = P` reduce that paycheck’s Safe-to-Spend. Transactions linked only to an ExpenseLine do not affect Safe-to-Spend unless they are also explicitly tagged to the paycheck.
```

**Rolling / look-ahead (secondary):**
```
rolling_available = sum( safe_to_spend(P) for every Paycheck P in the Month where P.date ≥ today )
```

Both numbers must update in real time as Allocations or tagged transactions change.

### 2.8 Transaction (v1 – Manual Only)
- Fields: `id`, `date`, `amount` (signed integer cents: positive = income, negative = expense), `payee`, `notes`, `expense_line_id` (nullable), `paycheck_id` (nullable – optional tagging).
- Linking a transaction to an ExpenseLine reduces the remaining available amount for that line.
- Optionally tagging it to a Paycheck also reduces that paycheck’s Safe-to-Spend.
- Bank linking / automatic import is explicitly out of scope for v1.

### 2.9 Variance Handling & Locked Month Behavior
When a Paycheck’s `actual_amount` ≠ `planned_amount`:
- System surfaces a clear variance indicator.
- Offers guided suggestions of under-funded or over-funded lines.
- User must eventually adjust Allocations so the Month returns to exact zero.
- System never auto-forces the adjustment.

**After a Month is Locked:**
- Planned amounts and Allocations are frozen and cannot be edited directly.
- Users may still record Transactions and update `actual_amount` on Paychecks.
- Variance re-assignment is allowed: it temporarily re-opens limited editing of Allocations so the user can bring the Month back to zero. Once the variance is resolved, the Month returns to the locked state.

---


### 2.10 Spent (Authoritative Definition)
- For any ExpenseLine, **Spent** = the sum of the absolute values of all Transactions linked to that ExpenseLine (`expense_line_id`).
- Spent is fully derived. It is never stored independently and is never manually editable.
- Transactions not linked to an ExpenseLine do not contribute to any line’s Spent.


### 2.11 Creating a New Month
When a user creates a new Month they must be presented with a clear choice (modal or equivalent):
1. **Start blank** – only starter categories are seeded.
2. **Copy structure only** – categories and expense lines are copied from a chosen existing Month; all Planned amounts start at 0; allocations are empty.
3. **Copy structure + Planned amounts** – categories, expense lines, and Planned amounts are copied; allocations start empty (user re-allocates across the new Month’s paychecks).

The source Month for copy operations is chosen by the user (commonly the previous month).


### 2.12 Deleting / Archiving a Month
- Soft-delete (archive) is the primary mechanism. Archived months are hidden from default lists but can be restored.
- Permanent delete is also permitted and requires a strong confirmation (e.g. typing the month name or a clear “I understand this is irreversible” step).
- Both Draft and Locked months may be archived or permanently deleted.

## 3. Primary (and Only) User Flow

1. User creates or selects a Month → system seeds starter categories.
2. User adds one or more IncomeLines (with smart suggestions) → system generates Paychecks.
3. User works **exclusively** from Paycheck views:
   - Selects a Paycheck.
   - Creates new ExpenseLines on the fly or selects existing ones.
   - Assigns positive amounts from the current Paycheck to those lines (creating or updating Allocations).
   - Can visit other Paychecks and assign additional portions of the same ExpenseLine (true splitting).
4. System continuously maintains the invariants and shows live Safe-to-Spend + live monthly aggregate zero status.
5. When the aggregate reaches exact zero, user may lock the Month.
6. After locking, only actuals, transactions, and variance re-assignments are permitted.

**Default screen after login**: The last Month the user was working in (remembered per user). Within that Month, show the current or next upcoming Paycheck (or the first Paycheck if the Month is entirely in the past/future). Display its Safe-to-Spend prominently and the list of what it currently funds.

**Monthly overview**: Secondary view that displays the derived totals, category roll-ups, and zero status. Any edit performed here is immediately translated into Allocation create/update/delete operations; it never creates an independent monthly planned amount.

---

## 4. v1 Scope (Absolute)

**Must be implemented in v1**
- Entire domain model and invariants above
- Paycheck-first planning as the sole construction method
- Full splitting of any ExpenseLine across any number of Paychecks
- Per-paycheck Safe-to-Spend (primary) + rolling look-ahead (secondary)
- Basic Debt category with `current_balance` and `minimum_payment`
- Reports (see Section 15):
  - Month-over-month comparison (planned vs actual income, planned vs actual spending per category)
  - Year-to-date (YTD) totals and category breakdown
  - Year-over-year (YoY) comparison for the selected month
  - Quick-win summary cards (selected month totals, variance, simple trends)
- Manual transactions with optional paycheck tagging
- Recurring + one-off IncomeLines with smart suggestions
- Strict zero-based enforcement on the aggregate
- Single-user only
- Lightweight onboarding: account creation → add first paycheck(s) → begin funding from that paycheck

**Explicitly forbidden in v1**
- Any monthly-first planning path
- Bank linking or automatic transaction import
- First-class Goals / sinking funds
- Debt snowball or avalanche tools
- Shared / multi-user budgets
- Advanced analytics, net-worth, forecasts, custom report builders
- Guided multi-step onboarding wizard

---

## 5. Edge Cases (Must Be Handled Explicitly)

- Reducing a Paycheck’s planned amount below current allocations → automatic reduction/deletion of excess Allocations + user notification of under-funded lines.
- Deleting or skipping a Paycheck → all its Allocations deleted; derived planned amounts decrease; user must re-balance if the Month is no longer at zero.
- Creating an ExpenseLine with no Allocations → its derived planned_amount is 0; it does not affect the zero check until funded.
- Mid-month addition of a new Paycheck or IncomeLine → allowed; user must allocate the new money or re-balance.
- Changing an Allocation amount → both the source Paycheck’s Safe-to-Spend and the ExpenseLine’s derived planned_amount update instantly.
- Attempt to lock a non-zero Month → blocked with clear message showing the exact remaining difference.

---

## 6. Non-Functional Requirements (v1)

- All money in integer cents.
- Invariants enforced on both client and server.
- Mobile apps must support offline viewing and manual transaction entry; sync when connectivity returns.
- Accessibility: WCAG 2.1 AA (web) and equivalent platform standards (iOS/Android).
- User can export any Month as CSV at any time.
- Data model must support future addition of goals, bank linking, and multi-user without breaking existing data.

---

## 7. Technical Architecture

### 7.1 Repository Structure
- Single monorepo.
- Shared core: pure Rust domain crate containing models, business rules, invariant enforcement, Safe-to-Spend calculations, allocation logic, and zero-based validation.
- `apps/web` (or equivalent): Rust web server that serves Datastar-powered HTML and SSE.
- Future mobile wrappers (if any) consume the same domain crate.

### 7.2 Backend
- Language / framework: **Rust** (Axum, Actix-web, or equivalent modern async framework).
- The domain crate is pure Rust and has no web or database dependencies where possible; adapters live at the edges.
- Invariants from this specification are enforced in the domain/service layer (never only in the client).
- Authentication: Custom JWT (short-lived access + refresh tokens).
- Sync: Custom offline-capable layer with optimistic UI, queue, and user-visible resolvable conflicts (web PWA + future clients).

### 7.3 Database
- **Primary for v1**: SQLite (single-file, simplest local / no-Docker deployment).
- **Supported via configuration**: PostgreSQL and MariaDB.
- All monetary values stored as integer (cents / smallest currency unit). Floating-point money is forbidden.
- Schema must remain portable across the supported engines (avoid engine-specific features in core migrations where practical).

### 7.4 Web Frontend
- **Datastar** (hypermedia framework).
- Backend drives the UI via HTML + SSE morph/patch events.
- Frontend reactivity via Datastar `data-*` attributes.
- Responsive design required (desktop + mobile web).
- Progressive Web App (PWA) capabilities for installability and basic offline behavior on mobile browsers.
- Feature parity for every v1 requirement is delivered through the web application.

### 7.5 Mobile Strategy (v1)
- Web-first.
- Mobile experience = responsive Datastar web app + PWA.
- Native iOS / Android clients are explicitly out of scope for v1 (may be added later via wrappers that reuse the Rust domain crate).

### 7.6 Deployment Preference
- Preferred: single static binary + SQLite file (no Docker required).
- Docker is optional but supported for those who want it.
- Configuration (database URL, secrets, etc.) via environment variables or a simple config file.

### 7.7 Cross-cutting Rules
- All money handling in backend/domain/database uses integer cents exclusively. Frontend displays and accepts dollars-and-cents; conversion occurs only at the UI boundary.
- Domain invariants are enforced server-side on every mutation.
- Playwright end-to-end tests (Section 13) are mandatory against the Datastar UI.
- Data export (CSV + full-month snapshot) must be available from the web app.
- Accessibility: WCAG 2.1 AA target, enforced via Playwright + axe.

### 7.8 Priority Order (for future technical decisions)
1. Correctness of domain invariants and zero-based rules
2. Reliability and testability (Playwright + TDD)
3. Simple deployment (binary + SQLite preferred)
4. Long-term maintainability of the pure Rust domain crate

---

## 8. Database Schema (SQLite primary; Postgres / MariaDB supported)

```sql
-- Users (single-user for v1, but structured for future multi-user)
CREATE TABLE users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email           TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Months
CREATE TABLE months (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    year_month      DATE NOT NULL,          -- first day of the month, e.g. 2026-09-01
    status          TEXT NOT NULL CHECK (status IN ('draft', 'locked')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, year_month)
);

-- Income Lines
CREATE TABLE income_lines (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    month_id        UUID NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    planned_amount  BIGINT NOT NULL CHECK (planned_amount >= 0),
    schedule_type   TEXT NOT NULL CHECK (schedule_type IN ('one_off', 'recurring')),
    recurrence_rule TEXT,                  -- iCal RRULE or equivalent JSON
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Paychecks (concrete instances)
CREATE TABLE paychecks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    income_line_id  UUID NOT NULL REFERENCES income_lines(id) ON DELETE CASCADE,
    date            DATE NOT NULL,
    planned_amount  BIGINT NOT NULL CHECK (planned_amount >= 0),
    actual_amount   BIGINT CHECK (actual_amount IS NULL OR actual_amount >= 0),
    status          TEXT NOT NULL CHECK (status IN ('planned', 'received', 'skipped')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (income_line_id, date)
);

-- Expense Categories
CREATE TABLE expense_categories (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    month_id        UUID NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Expense Lines
-- planned_amount is DERIVED – never stored
CREATE TABLE expense_lines (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    category_id     UUID NOT NULL REFERENCES expense_categories(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    -- Debt-specific (nullable for non-debt lines)
    current_balance BIGINT CHECK (current_balance IS NULL OR current_balance >= 0),
    minimum_payment BIGINT CHECK (minimum_payment IS NULL OR minimum_payment >= 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Allocations (source of truth for all planned spending)
CREATE TABLE allocations (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    expense_line_id UUID NOT NULL REFERENCES expense_lines(id) ON DELETE CASCADE,
    paycheck_id     UUID NOT NULL REFERENCES paychecks(id) ON DELETE CASCADE,
    amount          BIGINT NOT NULL CHECK (amount > 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (expense_line_id, paycheck_id)   -- one allocation per expense+paycheck pair
);

-- Transactions (manual only in v1)
CREATE TABLE transactions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    month_id        UUID NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    date            DATE NOT NULL,
    amount          BIGINT NOT NULL,        -- signed: positive = income, negative = expense
    payee           TEXT,
    notes           TEXT,
    expense_line_id UUID REFERENCES expense_lines(id) ON DELETE SET NULL,
    paycheck_id     UUID REFERENCES paychecks(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Helpful indexes
CREATE INDEX idx_months_user_year ON months(user_id, year_month);
CREATE INDEX idx_paychecks_date ON paychecks(date);
CREATE INDEX idx_allocations_paycheck ON allocations(paycheck_id);
CREATE INDEX idx_allocations_expense ON allocations(expense_line_id);
CREATE INDEX idx_transactions_month ON transactions(month_id);
```

**Derived values (never stored):**
- `ExpenseLine.planned_amount` = `SUM(allocations.amount)` for that line
- Month zero-based status = `SUM(paychecks.planned_amount) - SUM(derived expense planned amounts)`

---

## 9. Core API Contract (FastAPI / REST)

Base URL: `/api/v1`  
Authentication: `Authorization: Bearer <access_token>` on all endpoints except auth.

All request/response money fields are integers (cents).  
All endpoints that mutate allocations or planned amounts must re-validate the invariants and return `409 Conflict` with a clear error if any invariant would be violated.

### Auth
```
POST /auth/register
POST /auth/login          → { access_token, refresh_token }
POST /auth/refresh
POST /auth/logout
```

### Months
```
GET    /months
POST   /months                  { year_month: "2026-09-01" }
GET    /months/{month_id}
POST   /months/{month_id}/lock  (only succeeds if currently at exact zero)
```

### Income Lines & Paychecks
```
GET    /months/{month_id}/income-lines
POST   /months/{month_id}/income-lines
PATCH  /income-lines/{id}
DELETE /income-lines/{id}

GET    /months/{month_id}/paychecks
PATCH  /paychecks/{id}          (planned_amount, actual_amount, status)
```

### Categories & Expense Lines
```
GET    /months/{month_id}/categories
POST   /months/{month_id}/categories
PATCH  /categories/{id}
DELETE /categories/{id}

GET    /months/{month_id}/expense-lines
POST   /months/{month_id}/expense-lines
PATCH  /expense-lines/{id}
DELETE /expense-lines/{id}
```

### Allocations (the critical surface)
```
GET    /paychecks/{paycheck_id}/allocations
POST   /paychecks/{paycheck_id}/allocations
       body: { expense_line_id, amount }
       – creates or updates the unique (expense_line, paycheck) pair

PATCH  /allocations/{id}        { amount }
DELETE /allocations/{id}

# Convenience: move money between paychecks
POST   /allocations/transfer
       body: { from_paycheck_id, to_paycheck_id, expense_line_id, amount }
```

### Transactions
```
GET    /months/{month_id}/transactions
POST   /months/{month_id}/transactions
PATCH  /transactions/{id}
DELETE /transactions/{id}
```

### Derived / Read-only views
```
GET    /paychecks/{id}/safe-to-spend          → { safe_to_spend: number, rolling_available: number }
GET    /months/{id}/summary                   → zero status, totals, category roll-ups
GET    /months/{id}/export                    → CSV download
```

### Error Format (all 4xx/5xx)
```json
{
  "error": {
    "code": "INVARIANT_VIOLATION",
    "message": "Allocating this amount would overdraw paycheck X by 1500 cents",
    "details": { ... }
  }
}
```

---

## 10. Invariant Enforcement Points (mandatory)

The service layer **must** re-check after every mutating request:

1. No Paycheck is over-allocated.
2. Every ExpenseLine’s derived planned amount equals the sum of its allocations.
3. Month can only be locked when aggregate planned income − aggregate derived planned expenses == 0.
4. Reducing a paycheck’s planned_amount that would break allocations is either rejected or automatically cascades (with explicit user confirmation if the API supports it).

---

## 11. Testing Requirement (Mandatory)

**All behavior defined in this specification must be fully tested using Test-Driven Development (TDD) and Playwright end-to-end tests.**

- Write the failing test first.
- Implement the minimum code to make the test pass.
- Refactor while keeping tests green.
- Domain invariants, Safe-to-Spend calculations, allocation rules, locking rules, edge cases, and API contract behavior are all required to have comprehensive automated tests.
- Both unit tests (shared core + service layer) and integration / API tests are required.
- Mobile and web clients must have tests covering the critical user flows and invariant enforcement on the client side.
- Playwright requirements are defined in full in Section 13.

No feature is considered complete until its tests exist and pass.

---

## 12. Sample Acceptance Criteria

- User can fund Rent as $500 from Paycheck 1 and $500 from Paycheck 2 in Month A, then change to $0 + $1000 in Month B with no restrictions.
- Creating an Allocation that would make a Paycheck over-allocated is rejected immediately.
- Derived ExpenseLine.planned_amount always equals the sum of its Allocations; there is never a divergent stored value.
- Safe-to-Spend for a Paycheck updates in real time when any of its Allocations change.
- Month cannot be locked unless aggregate planned income − aggregate derived planned expenses == 0.
- Deleting a Paycheck that has Allocations correctly removes those Allocations and updates all derived amounts.

---

## 13. Production Quality, Testing & Release Requirements

This section is normative. Any implementation that does not satisfy these rules is considered incomplete and must not be called production-ready or v1 complete.

### 13.1 Playwright End-to-End Testing (Mandatory)

- **Coverage**: Every user-visible feature and flow must have at least one Playwright test that asserts the UI actually changes correctly.
- **Depth**: Tests must cover happy path + error states + edge cases (over-allocation, lock when not zero, deleting funded paychecks, mid-month changes, split across many paychecks, variance handling, etc.).
- **Extras**: Visual regression (screenshots) + accessibility checks (axe or equivalent) must run inside the Playwright suite.
- **Isolation**: Every test suite starts from a full database reset + deterministic seed. No shared mutable state between tests.
- **Organization**: Tests grouped by feature/area (auth, months, paychecks, allocations, transactions, export, offline/sync, accessibility) using shared fixtures and page objects.
- **Browser/device matrix**: Desktop Chrome + Firefox + mobile Safari/Chrome emulation.
- **Execution**: Tests must be runnable with a single command. Currently local-only; they must pass before any claim of “done.”

### 13.2 UI State Completeness

Every major screen must implement:
- Skeleton loaders while data is fetching
- Dedicated empty states with clear next-action guidance
- Recoverable error states
- Offline banners / indicators

### 13.3 Error Messaging

- Primary message: clear, human, and actionable (e.g. “This paycheck is over-allocated by $12.50”).
- Secondary: collapsible technical details for power users / debugging.
- Never generic “Something went wrong” as the only message for invariant violations.

### 13.4 Performance Targets

- Initial load < 2 seconds on a mid-range phone and average laptop.
- Interactions must feel instant (perceived < 100 ms).

### 13.5 Authentication & Session Security

- Short-lived access tokens + refresh tokens.
- Secure storage on each platform.
- Automatic silent refresh.
- “Log out of all devices” support.

### 13.6 Data Export

- Deterministic, complete CSV export of a month — itself covered by Playwright tests.
- Full-month snapshot export (JSON or ZIP) suitable for backup/restore.

### 13.7 Offline & Sync

- Optimistic UI.
- Robust offline queue for creates/edits.
- User-visible, resolvable sync conflicts.

### 13.8 Accessibility

- WCAG 2.1 AA on web.
- Equivalent platform accessibility guidelines on iOS and Android.
- Enforced via Playwright + axe (or equivalent).

### 13.9 Internationalization, Currency & Timezone

- All user-facing strings externalized (i18n-ready).
- English only for v1 content.
- User-selectable currency with proper locale formatting.
- All money remains integer cents (or equivalent smallest unit) internally.
- **Changing currency** converts all existing amounts using a rate supplied by the user at the time of the change.
- **Timezone** is stored on the user profile. It defaults to the browser timezone at registration and is used for all “today”-relative calculations (rolling Safe-to-Spend, default paycheck selection, etc.).

### 13.10 Hard Release Gate for v1

A build may be called “v1 complete” only when:
- All Playwright tests pass, and
- Core flows work correctly on real devices.

### 13.11 Explicitly Not Required for v1

- Monitoring / alerting / Sentry
- Hard Lighthouse performance budgets enforced in CI
- Multiple languages
- Real-device cloud testing (emulation is sufficient for now)

---


## 14. UI / UX Requirements (EveryDollar-closeness)

This section is normative. Previous implementations that omitted these behaviors are considered non-compliant.

### 14.1 Overall Interaction Goal
A user who is familiar with EveryDollar must feel immediately at home, with one primary difference: planning is paycheck-first rather than monthly-first. Visual hierarchy, inline editing, and column concepts should be recognizably similar.

### 14.2 Inline Editing
- Expense line **names** and **Planned amounts** are editable inline on both the Paycheck view and the Monthly overview.
- Click-to-edit or always-visible editable fields are both acceptable; a separate modal must not be required for the common case.
- **Remaining** is strictly read-only (derived). Users never type into the Remaining column.
- Spent is primarily driven by transactions. Direct inline edit of Spent is not required for v1.

### 14.3 Category Grouping & Summaries
- One level of hierarchy: Category → Expense lines (EveryDollar style).
- Categories are collapsible / expandable.
- Each **category header** must show summary data:
  - Total Planned for the category
  - Total Spent for the category
  - Remaining for the category (read-only, derived)
- When expanded, the individual expense lines appear underneath the category header, each showing their own Planned / Spent / Remaining values.
- Category summaries must update live when any child line’s Planned amount or related transactions change.

### 14.4 Column Model (Paycheck & Monthly views)
Base columns (required):
- Name (inline editable)
- Planned (inline editable)
- Spent
- Remaining (read-only)

Slightly denser layouts are permitted if they remain scannable (e.g. additional context such as which paycheck funded a line, or a compact progress indicator). The core Planned / Spent / Remaining triad must always be present and obvious.

### 14.5 Paycheck View (Primary Daily Experience)
- Default landing screen after login = current or next Paycheck.
- Prominently displays Safe-to-Spend for that paycheck.
- Shows the list of expense lines (and their categories) that this paycheck is currently funding, using the grouping and column rules above.
- User can inline-edit Planned amounts and names directly in this view.
- User can create new expense lines and allocate them to the current paycheck without leaving the view.

### 14.6 Monthly Overview (Secondary)
- Shows the derived aggregate of all paycheck decisions.
- Uses the same category grouping, summary headers, and Planned / Spent / Remaining columns.
- Inline editing here immediately updates the underlying Allocations (there is never an independent monthly planned amount).

### 14.7 Visual & Interaction Polish
- Skeleton loaders, empty states with clear next actions, recoverable errors, and offline indicators remain mandatory (Section 13).
- Expand/collapse state of categories should be remembered per user/session where practical.
- All of the above behaviors must be covered by Playwright tests (including inline edit, expand/collapse, and live update of category summaries).

---


## 15. Reports & Metrics (v1)

Reports are first-class but kept deliberately lightweight. Advanced analytics, net-worth, forecasts, and custom report builders remain out of scope.

### 15.1 Required Reports

1. **Month-over-month (MoM)**  
   For any selected Month, compare against the immediately previous Month:
   - Total planned income vs actual income
   - Total planned expenses vs actual spending
   - Per-category planned vs actual

2. **Year-to-date (YTD)**  
   From January 1 of the selected Month’s year through the end of the selected Month:
   - Total income (planned and actual)
   - Total expenses (planned and actual)
   - Category breakdown

3. **Year-over-year (YoY)**  
   Selected Month compared with the same month in the previous year (when data exists):
   - Income and expense totals
   - Per-category comparison

4. **Quick-win summary cards** (shown on Monthly overview or a simple Reports screen)
   - Selected month: total income, total expenses, remaining-to-zero status
   - Variance (actual − planned) for income and for expenses
   - Simple indicator of whether the user is over- or under-spending relative to plan

### 15.2 Rules

- All report values are derived from existing Planned amounts, Allocations, and Transactions. No separate reporting data store is required for v1.
- Reports must work for past, current, and future Months (future Months will simply show planned figures and zero actuals).
- Reports are covered by Playwright tests that assert correct numbers for seeded data.
- Export (CSV of a single Month + full-month snapshot) remains mandatory and is separate from these interactive reports.

---

**End of Specification v0.4.4**  
This document is frozen. Implementation must conform to it exactly. All domain, UI/UX, reporting, production-quality, Playwright, and money-presentation requirements are mandatory.
This document is frozen. Implementation must conform to it exactly. All domain, UI/UX, reporting, production-quality, and Playwright requirements are mandatory.
This document is frozen. Implementation must conform to it exactly. All production-quality, Playwright, UI/UX, Month-range, and reporting requirements are mandatory.
