-- ============================================================================
-- PaycheckZero – PostgreSQL Setup
-- Specification version: 0.4.3
-- Requires: PostgreSQL 15+
--
-- Usage:
--   psql -U postgres -d paycheckzero -f setup.sql
-- ============================================================================

BEGIN;

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ---------------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email           TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    full_name       TEXT,
    timezone        TEXT NOT NULL DEFAULT 'UTC',
    currency        TEXT NOT NULL DEFAULT 'USD',
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT users_email_format CHECK (email ~* '^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$')
);

-- ---------------------------------------------------------------------------
-- Months
-- archived_at NULL = active; non-NULL = soft-deleted/archived
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS months (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    year_month      DATE NOT NULL,
    status          TEXT NOT NULL DEFAULT 'draft'
                        CHECK (status IN ('draft', 'locked')),
    archived_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, year_month),
    CONSTRAINT months_year_month_is_first_of_month CHECK (EXTRACT(DAY FROM year_month) = 1)
);

CREATE INDEX IF NOT EXISTS idx_months_user_year ON months(user_id, year_month);
CREATE INDEX IF NOT EXISTS idx_months_archived ON months(archived_at);

-- ---------------------------------------------------------------------------
-- Income Lines
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS income_lines (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    month_id        UUID NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    planned_amount  BIGINT NOT NULL CHECK (planned_amount >= 0),
    schedule_type   TEXT NOT NULL CHECK (schedule_type IN ('one_off', 'recurring')),
    recurrence_rule TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_income_lines_month ON income_lines(month_id);

-- ---------------------------------------------------------------------------
-- Paychecks
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS paychecks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    income_line_id  UUID NOT NULL REFERENCES income_lines(id) ON DELETE CASCADE,
    date            DATE NOT NULL,
    planned_amount  BIGINT NOT NULL CHECK (planned_amount >= 0),
    actual_amount   BIGINT CHECK (actual_amount IS NULL OR actual_amount >= 0),
    status          TEXT NOT NULL DEFAULT 'planned'
                        CHECK (status IN ('planned', 'received', 'skipped')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (income_line_id, date)
);

CREATE INDEX IF NOT EXISTS idx_paychecks_income_line ON paychecks(income_line_id);
CREATE INDEX IF NOT EXISTS idx_paychecks_date ON paychecks(date);

-- ---------------------------------------------------------------------------
-- Expense Categories
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS expense_categories (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    month_id        UUID NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (char_length(trim(name)) > 0),
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_expense_categories_month ON expense_categories(month_id);

-- ---------------------------------------------------------------------------
-- Expense Lines (planned_amount is derived, never stored)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS expense_lines (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    category_id     UUID NOT NULL REFERENCES expense_categories(id) ON DELETE CASCADE,
    name            TEXT NOT NULL CHECK (char_length(trim(name)) > 0),
    current_balance BIGINT CHECK (current_balance IS NULL OR current_balance >= 0),
    minimum_payment BIGINT CHECK (minimum_payment IS NULL OR minimum_payment >= 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_expense_lines_category ON expense_lines(category_id);

-- ---------------------------------------------------------------------------
-- Allocations
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS allocations (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    expense_line_id UUID NOT NULL REFERENCES expense_lines(id) ON DELETE CASCADE,
    paycheck_id     UUID NOT NULL REFERENCES paychecks(id) ON DELETE CASCADE,
    amount          BIGINT NOT NULL CHECK (amount > 0),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (expense_line_id, paycheck_id)
);

CREATE INDEX IF NOT EXISTS idx_allocations_paycheck ON allocations(paycheck_id);
CREATE INDEX IF NOT EXISTS idx_allocations_expense ON allocations(expense_line_id);

-- ---------------------------------------------------------------------------
-- Transactions
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS transactions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    month_id        UUID NOT NULL REFERENCES months(id) ON DELETE CASCADE,
    date            DATE NOT NULL,
    amount          BIGINT NOT NULL,
    payee           TEXT,
    notes           TEXT,
    expense_line_id UUID REFERENCES expense_lines(id) ON DELETE SET NULL,
    paycheck_id     UUID REFERENCES paychecks(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_transactions_month ON transactions(month_id);
CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(date);
CREATE INDEX IF NOT EXISTS idx_transactions_paycheck ON transactions(paycheck_id);
CREATE INDEX IF NOT EXISTS idx_transactions_expense ON transactions(expense_line_id);

-- ---------------------------------------------------------------------------
-- updated_at trigger
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DO $$
DECLARE t TEXT;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'users', 'months', 'income_lines', 'paychecks',
        'expense_categories', 'expense_lines', 'allocations', 'transactions'
    ]
    LOOP
        EXECUTE format('
            DROP TRIGGER IF EXISTS trg_%I_updated_at ON %I;
            CREATE TRIGGER trg_%I_updated_at
                BEFORE UPDATE ON %I
                FOR EACH ROW EXECUTE FUNCTION set_updated_at();
        ', t, t, t, t);
    END LOOP;
END;
$$;

-- ---------------------------------------------------------------------------
-- Views
-- ---------------------------------------------------------------------------
CREATE OR REPLACE VIEW v_expense_line_planned AS
SELECT
    el.id AS expense_line_id,
    el.category_id,
    el.name,
    COALESCE(SUM(a.amount), 0) AS planned_amount
FROM expense_lines el
LEFT JOIN allocations a ON a.expense_line_id = el.id
GROUP BY el.id, el.category_id, el.name;

CREATE OR REPLACE VIEW v_expense_line_spent AS
SELECT
    el.id AS expense_line_id,
    COALESCE(SUM(CASE WHEN t.amount < 0 THEN -t.amount ELSE 0 END), 0) AS spent_amount
FROM expense_lines el
LEFT JOIN transactions t ON t.expense_line_id = el.id
GROUP BY el.id;

CREATE OR REPLACE VIEW v_paycheck_allocated AS
SELECT
    p.id AS paycheck_id,
    p.planned_amount,
    COALESCE(SUM(a.amount), 0) AS allocated_amount,
    p.planned_amount - COALESCE(SUM(a.amount), 0) AS remaining_to_allocate,
    (p.planned_amount = COALESCE(SUM(a.amount), 0)) AS is_fully_allocated
FROM paychecks p
LEFT JOIN allocations a ON a.paycheck_id = p.id
GROUP BY p.id, p.planned_amount;

-- ---------------------------------------------------------------------------
-- Seed (same deterministic data as SQLite)
-- ---------------------------------------------------------------------------
INSERT INTO users (id, email, password_hash, full_name, timezone, currency)
VALUES (
    '11111111-1111-1111-1111-111111111111',
    'demo@paycheckzero.app',
    '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.G2oQ.Y5zqK8K2i',
    'Demo User',
    'America/Denver',
    'USD'
) ON CONFLICT (email) DO NOTHING;

INSERT INTO months (id, user_id, year_month, status)
VALUES (
    '22222222-2222-2222-2222-222222222222',
    '11111111-1111-1111-1111-111111111111',
    '2026-09-01',
    'draft'
) ON CONFLICT (user_id, year_month) DO NOTHING;

INSERT INTO expense_categories (id, month_id, name, sort_order) VALUES
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1', '22222222-2222-2222-2222-222222222222', 'Giving', 10),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa2', '22222222-2222-2222-2222-222222222222', 'Saving', 20),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa3', '22222222-2222-2222-2222-222222222222', 'Housing', 30),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa4', '22222222-2222-2222-2222-222222222222', 'Transportation', 40),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', '22222222-2222-2222-2222-222222222222', 'Food', 50),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa6', '22222222-2222-2222-2222-222222222222', 'Personal', 60),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa7', '22222222-2222-2222-2222-222222222222', 'Lifestyle', 70),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa8', '22222222-2222-2222-2222-222222222222', 'Health', 80),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa9', '22222222-2222-2222-2222-222222222222', 'Insurance', 90),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa10', '22222222-2222-2222-2222-222222222222', 'Debt', 100),
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa11', '22222222-2222-2222-2222-222222222222', 'Other', 110)
ON CONFLICT DO NOTHING;

INSERT INTO income_lines (id, month_id, name, planned_amount, schedule_type, recurrence_rule) VALUES
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '22222222-2222-2222-2222-222222222222', 'Primary Job', 200000, 'recurring', 'FREQ=WEEKLY;INTERVAL=2;BYDAY=FR'),
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2', '22222222-2222-2222-2222-222222222222', 'Side Hustle', 40000, 'one_off', NULL)
ON CONFLICT DO NOTHING;

INSERT INTO paychecks (id, income_line_id, date, planned_amount, status) VALUES
    ('cccccccc-cccc-cccc-cccc-ccccccccccc1', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '2026-09-04', 200000, 'planned'),
    ('cccccccc-cccc-cccc-cccc-ccccccccccc2', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '2026-09-18', 200000, 'planned'),
    ('cccccccc-cccc-cccc-cccc-ccccccccccc3', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2', '2026-09-10', 40000, 'planned')
ON CONFLICT DO NOTHING;

INSERT INTO expense_lines (id, category_id, name, current_balance, minimum_payment) VALUES
    ('dddddddd-dddd-dddd-dddd-ddddddddddd1', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa3', 'Rent', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd2', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', 'Groceries', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd3', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', 'Restaurants', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd4', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa4', 'Gas', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd5', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa10', 'Student Loan', 1250000, 35000),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd6', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1', 'Church Tithe', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd7', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa2', 'Emergency Fund', NULL, NULL)
ON CONFLICT DO NOTHING;

INSERT INTO allocations (id, expense_line_id, paycheck_id, amount) VALUES
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee01', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 50000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee02', 'dddddddd-dddd-dddd-dddd-ddddddddddd2', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 20000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee03', 'dddddddd-dddd-dddd-dddd-ddddddddddd4', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 7500),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee04', 'dddddddd-dddd-dddd-dddd-ddddddddddd6', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 20000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee05', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc1', 10000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee06', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 50000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee07', 'dddddddd-dddd-dddd-dddd-ddddddddddd2', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 20000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee08', 'dddddddd-dddd-dddd-dddd-ddddddddddd3', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 10000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee09', 'dddddddd-dddd-dddd-dddd-ddddddddddd5', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 35000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee10', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc2', 15000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee11', 'dddddddd-dddd-dddd-dddd-ddddddddddd4', 'cccccccc-cccc-cccc-cccc-ccccccccccc3', 5000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee12', 'dddddddd-dddd-dddd-dddd-ddddddddddd3', 'cccccccc-cccc-cccc-cccc-ccccccccccc3', 5000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee13', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc3', 5000),
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee14', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc3', 25000)
ON CONFLICT DO NOTHING;

INSERT INTO transactions (id, month_id, date, amount, payee, notes, expense_line_id, paycheck_id) VALUES
    ('ffffffff-ffff-ffff-ffff-fffffffffff1', '22222222-2222-2222-2222-222222222222', '2026-09-05', -8750, 'Shell', 'Fill-up', 'dddddddd-dddd-dddd-dddd-ddddddddddd4', 'cccccccc-cccc-cccc-cccc-ccccccccccc1'),
    ('ffffffff-ffff-ffff-ffff-fffffffffff2', '22222222-2222-2222-2222-222222222222', '2026-09-06', -4500, 'Kroger', 'Weekly groceries', 'dddddddd-dddd-dddd-dddd-ddddddddddd2', 'cccccccc-cccc-cccc-cccc-ccccccccccc1'),
    ('ffffffff-ffff-ffff-ffff-fffffffffff3', '22222222-2222-2222-2222-222222222222', '2026-09-07', -1200, 'Coffee Shop', NULL, 'dddddddd-dddd-dddd-dddd-ddddddddddd3', NULL)
ON CONFLICT DO NOTHING;

COMMIT;
