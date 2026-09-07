-- ============================================================================
-- PaycheckZero – SQLite Setup (Primary Engine)
-- Specification version: 0.4.3
-- Requires: SQLite 3.37+ (for STRICT tables)
--
-- Usage:
--   sqlite3 paycheckzero.db < setup.sql
--
-- All money is INTEGER cents. Floating-point money is forbidden.
-- ============================================================================

PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;

BEGIN;

-- ---------------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users (
    id              TEXT PRIMARY KEY,                     -- UUID stored as text
    email           TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    full_name       TEXT,
    timezone        TEXT NOT NULL DEFAULT 'UTC',          -- IANA timezone, e.g. America/Denver
    currency        TEXT NOT NULL DEFAULT 'USD',          -- ISO 4217
    is_active       INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- ---------------------------------------------------------------------------
-- Months
-- year_month is stored as first day of month: '2026-09-01'
-- archived_at NULL = active; non-NULL = soft-deleted/archived
-- Permanent delete = actual DELETE after strong confirmation
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS months (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    year_month      TEXT NOT NULL,                        -- 'YYYY-MM-01'
    status          TEXT NOT NULL DEFAULT 'draft'
                        CHECK (status IN ('draft', 'locked')),
    archived_at     TEXT,                                 -- ISO-8601 or NULL
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, year_month)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_months_user_year ON months(user_id, year_month);
CREATE INDEX IF NOT EXISTS idx_months_archived ON months(archived_at);

-- ---------------------------------------------------------------------------
-- Income Lines
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS income_lines (
    id              TEXT PRIMARY KEY,
    month_id        TEXT NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    planned_amount  INTEGER NOT NULL CHECK (planned_amount >= 0),
    schedule_type   TEXT NOT NULL CHECK (schedule_type IN ('one_off', 'recurring')),
    recurrence_rule TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_income_lines_month ON income_lines(month_id);

-- ---------------------------------------------------------------------------
-- Paychecks
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS paychecks (
    id              TEXT PRIMARY KEY,
    income_line_id  TEXT NOT NULL REFERENCES income_lines(id) ON DELETE CASCADE,
    date            TEXT NOT NULL,                        -- 'YYYY-MM-DD'
    planned_amount  INTEGER NOT NULL CHECK (planned_amount >= 0),
    actual_amount   INTEGER CHECK (actual_amount IS NULL OR actual_amount >= 0),
    status          TEXT NOT NULL DEFAULT 'planned'
                        CHECK (status IN ('planned', 'received', 'skipped')),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (income_line_id, date)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_paychecks_income_line ON paychecks(income_line_id);
CREATE INDEX IF NOT EXISTS idx_paychecks_date ON paychecks(date);

-- ---------------------------------------------------------------------------
-- Expense Categories
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS expense_categories (
    id              TEXT PRIMARY KEY,
    month_id        TEXT NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (length(trim(name)) > 0),
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_expense_categories_month ON expense_categories(month_id);

-- ---------------------------------------------------------------------------
-- Expense Lines
-- planned_amount is NEVER stored; it is always derived from allocations
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS expense_lines (
    id              TEXT PRIMARY KEY,
    category_id     TEXT NOT NULL REFERENCES expense_categories(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (length(trim(name)) > 0),
    current_balance INTEGER CHECK (current_balance IS NULL OR current_balance >= 0),  -- Debt only
    minimum_payment INTEGER CHECK (minimum_payment IS NULL OR minimum_payment >= 0),  -- Debt only
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_expense_lines_category ON expense_lines(category_id);

-- ---------------------------------------------------------------------------
-- Allocations (source of truth for planned spending)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS allocations (
    id              TEXT PRIMARY KEY,
    expense_line_id TEXT NOT NULL REFERENCES expense_lines(id) ON DELETE CASCADE,
    paycheck_id     TEXT NOT NULL REFERENCES paychecks(id) ON DELETE CASCADE,
    amount          INTEGER NOT NULL CHECK (amount > 0),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (expense_line_id, paycheck_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_allocations_paycheck ON allocations(paycheck_id);
CREATE INDEX IF NOT EXISTS idx_allocations_expense ON allocations(expense_line_id);

-- ---------------------------------------------------------------------------
-- Transactions (manual only in v1)
-- amount is signed: positive = income, negative = expense
-- Spent for an ExpenseLine = SUM(ABS(amount)) WHERE expense_line_id = ? AND amount < 0
-- Safe-to-Spend is reduced only by transactions with an explicit paycheck_id
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS transactions (
    id              TEXT PRIMARY KEY,
    month_id        TEXT NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    date            TEXT NOT NULL,
    amount          INTEGER NOT NULL,
    payee           TEXT,
    notes           TEXT,
    expense_line_id TEXT REFERENCES expense_lines(id) ON DELETE SET NULL,
    paycheck_id     TEXT REFERENCES paychecks(id) ON DELETE SET NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_transactions_month ON transactions(month_id);
CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(date);
CREATE INDEX IF NOT EXISTS idx_transactions_paycheck ON transactions(paycheck_id);
CREATE INDEX IF NOT EXISTS idx_transactions_expense ON transactions(expense_line_id);

-- ---------------------------------------------------------------------------
-- Derived views
-- ---------------------------------------------------------------------------
CREATE VIEW IF NOT EXISTS v_expense_line_planned AS
SELECT
    el.id AS expense_line_id,
    el.category_id,
    el.name,
    COALESCE(SUM(a.amount), 0) AS planned_amount
FROM expense_lines el
LEFT JOIN allocations a ON a.expense_line_id = el.id
GROUP BY el.id, el.category_id, el.name;

CREATE VIEW IF NOT EXISTS v_expense_line_spent AS
SELECT
    el.id AS expense_line_id,
    COALESCE(SUM(CASE WHEN t.amount < 0 THEN -t.amount ELSE 0 END), 0) AS spent_amount
FROM expense_lines el
LEFT JOIN transactions t ON t.expense_line_id = el.id
GROUP BY el.id;

CREATE VIEW IF NOT EXISTS v_paycheck_allocated AS
SELECT
    p.id AS paycheck_id,
    p.planned_amount,
    COALESCE(SUM(a.amount), 0) AS allocated_amount,
    p.planned_amount - COALESCE(SUM(a.amount), 0) AS remaining_to_allocate,
    CASE WHEN p.planned_amount = COALESCE(SUM(a.amount), 0) THEN 1 ELSE 0 END AS is_fully_allocated
FROM paychecks p
LEFT JOIN allocations a ON a.paycheck_id = p.id
GROUP BY p.id, p.planned_amount;

-- ---------------------------------------------------------------------------
-- Seed data (deterministic UUIDs for tests)
-- ---------------------------------------------------------------------------
INSERT OR IGNORE INTO users (id, email, password_hash, full_name, timezone, currency)
VALUES (
    '11111111-1111-1111-1111-111111111111',
    'demo@paycheckzero.app',
    '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.G2oQ.Y5zqK8K2i',  -- Password123!
    'Demo User',
    'America/Denver',
    'USD'
);

INSERT OR IGNORE INTO months (id, user_id, year_month, status)
VALUES (
    '22222222-2222-2222-2222-222222222222',
    '11111111-1111-1111-1111-111111111111',
    '2026-09-01',
    'draft'
);

INSERT OR IGNORE INTO expense_categories (id, month_id, name, sort_order) VALUES
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1', '22222222-2222-2222-2222-222222222222', 'Giving',         10),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa2', '22222222-2222-2222-2222-222222222222', 'Saving',         20),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa3', '22222222-2222-2222-2222-222222222222', 'Housing',        30),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa4', '22222222-2222-2222-2222-222222222222', 'Transportation', 40),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', '22222222-2222-2222-2222-222222222222', 'Food',           50),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa6', '22222222-2222-2222-2222-222222222222', 'Personal',       60),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa7', '22222222-2222-2222-2222-222222222222', 'Lifestyle',      70),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa8', '22222222-2222-2222-2222-222222222222', 'Health',         80),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa9', '22222222-2222-2222-2222-222222222222', 'Insurance',      90),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa10', '22222222-2222-2222-2222-222222222222', 'Debt',          100),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa11', '22222222-2222-2222-2222-222222222222', 'Other',         110);

INSERT OR IGNORE INTO income_lines (id, month_id, name, planned_amount, schedule_type, recurrence_rule) VALUES
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '22222222-2222-2222-2222-222222222222',
     'Primary Job', 200000, 'recurring', 'FREQ=WEEKLY;INTERVAL=2;BYDAY=FR'),
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2', '22222222-2222-2222-2222-222222222222',
     'Side Hustle',  40000, 'one_off',   NULL);

INSERT OR IGNORE INTO paychecks (id, income_line_id, date, planned_amount, status) VALUES
    ('cccccccc-cccc-cccc-cccc-ccccccccccc1', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '2026-09-04', 200000, 'planned'),
    ('cccccccc-cccc-cccc-cccc-ccccccccccc2', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '2026-09-18', 200000, 'planned'),
    ('cccccccc-cccc-cccc-cccc-ccccccccccc3', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2', '2026-09-10',  40000, 'planned');

INSERT OR IGNORE INTO expense_lines (id, category_id, name, current_balance, minimum_payment) VALUES
    ('dddddddd-dddd-dddd-dddd-ddddddddddd1', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa3', 'Rent',           NULL,    NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd2', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', 'Groceries',      NULL,    NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd3', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', 'Restaurants',    NULL,    NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd4', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa4', 'Gas',            NULL,    NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd5', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa10', 'Student Loan', 1250000,  35000),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd6', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1', 'Church Tithe',   NULL,    NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd7', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa2', 'Emergency Fund', NULL,    NULL);

-- Fully allocated example (paycheck 1 + 2 + side hustle)
INSERT OR IGNORE INTO allocations (id, expense_line_id, paycheck_id, amount) VALUES
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee01', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 50000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee02', 'dddddddd-dddd-dddd-dddd-ddddddddddd2', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 20000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee03', 'dddddddd-dddd-dddd-dddd-ddddddddddd4', 'cccccccc-cccc-cccc-cccc-ccccccccccc1',  7500),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee04', 'dddddddd-dddd-dddd-dddd-ddddddddddd6', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 20000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee05', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 10000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee06', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 50000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee07', 'dddddddd-dddd-dddd-dddd-ddddddddddd2', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 20000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee08', 'dddddddd-dddd-dddd-dddd-ddddddddddd3', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 10000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee09', 'dddddddd-dddd-dddd-dddd-ddddddddddd5', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 35000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee10', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 15000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee11', 'dddddddd-dddd-dddd-dddd-ddddddddddd4', 'cccccccc-cccc-cccc-cccc-ccccccccccc3',  5000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee12', 'dddddddd-dddd-dddd-dddd-ddddddddddd3', 'cccccccc-cccc-cccc-cccc-ccccccccccc3',  5000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee13', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc3',  5000),
    -- remaining 25_000 on side hustle also allocated so every paycheck is fully allocated
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee14', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc3', 25000);

INSERT OR IGNORE INTO transactions (id, month_id, date, amount, payee, notes, expense_line_id, paycheck_id) VALUES
    ('ffffffff-ffff-ffff-ffff-fffffffffff1', '22222222-2222-2222-2222-222222222222', '2026-09-05', -8750, 'Shell',       'Fill-up',          'dddddddd-dddd-dddd-dddd-ddddddddddd4', 'cccccccc-cccc-cccc-cccc-ccccccccccc1'),
    ('ffffffff-ffff-ffff-ffff-fffffffffff2', '22222222-2222-2222-2222-222222222222', '2026-09-06', -4500, 'Kroger',      'Weekly groceries', 'dddddddd-dddd-dddd-dddd-ddddddddddd2', 'cccccccc-cccc-cccc-cccc-ccccccccccc1'),
    ('ffffffff-ffff-ffff-ffff-fffffffffff3', '22222222-2222-2222-2222-222222222222', '2026-09-07', -1200, 'Coffee Shop', NULL,               'dddddddd-dddd-dddd-dddd-ddddddddddd3', NULL);

COMMIT;

-- Demo credentials: demo@paycheckzero.app / Password123!
