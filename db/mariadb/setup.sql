-- ============================================================================
-- PaycheckZero – MariaDB / MySQL Setup
-- Specification version: 0.4.3
-- Requires: MariaDB 10.6+ or MySQL 8.0+
--
-- Usage:
--   mariadb paycheckzero < setup.sql
--   mysql paycheckzero < setup.sql
-- ============================================================================

START TRANSACTION;

-- ---------------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users (
    id              CHAR(36) PRIMARY KEY,
    email           VARCHAR(255) NOT NULL,
    password_hash   VARCHAR(255) NOT NULL,
    full_name       VARCHAR(255),
    timezone        VARCHAR(64) NOT NULL DEFAULT 'UTC',
    currency        CHAR(3) NOT NULL DEFAULT 'USD',
    is_active       TINYINT(1) NOT NULL DEFAULT 1,
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    UNIQUE KEY users_email_unique (email)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- ---------------------------------------------------------------------------
-- Months
-- archived_at NULL = active; non-NULL = soft-deleted/archived
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS months (
    id              CHAR(36) PRIMARY KEY,
    user_id         CHAR(36) NOT NULL,
    year_month      DATE NOT NULL,
    status          ENUM('draft', 'locked') NOT NULL DEFAULT 'draft',
    archived_at     DATETIME(6) NULL,
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    UNIQUE KEY months_user_year_unique (user_id, year_month),
    CONSTRAINT fk_months_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_months_archived ON months(archived_at);

-- ---------------------------------------------------------------------------
-- Income Lines
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS income_lines (
    id              CHAR(36) PRIMARY KEY,
    month_id        CHAR(36) NOT NULL,
    name            VARCHAR(100) NOT NULL,
    planned_amount  BIGINT NOT NULL,
    schedule_type   ENUM('one_off', 'recurring') NOT NULL,
    recurrence_rule TEXT,
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    CONSTRAINT fk_income_lines_month FOREIGN KEY (month_id) REFERENCES months(id) ON DELETE CASCADE,
    CONSTRAINT chk_income_planned CHECK (planned_amount >= 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_income_lines_month ON income_lines(month_id);

-- ---------------------------------------------------------------------------
-- Paychecks
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS paychecks (
    id              CHAR(36) PRIMARY KEY,
    income_line_id  CHAR(36) NOT NULL,
    date            DATE NOT NULL,
    planned_amount  BIGINT NOT NULL,
    actual_amount   BIGINT NULL,
    status          ENUM('planned', 'received', 'skipped') NOT NULL DEFAULT 'planned',
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    UNIQUE KEY paychecks_income_date_unique (income_line_id, date),
    CONSTRAINT fk_paychecks_income FOREIGN KEY (income_line_id) REFERENCES income_lines(id) ON DELETE CASCADE,
    CONSTRAINT chk_paycheck_planned CHECK (planned_amount >= 0),
    CONSTRAINT chk_paycheck_actual CHECK (actual_amount IS NULL OR actual_amount >= 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_paychecks_date ON paychecks(date);

-- ---------------------------------------------------------------------------
-- Expense Categories
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS expense_categories (
    id              CHAR(36) PRIMARY KEY,
    month_id        CHAR(36) NOT NULL,
    name            VARCHAR(255) NOT NULL,
    sort_order      INT NOT NULL DEFAULT 0,
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    CONSTRAINT fk_expense_categories_month FOREIGN KEY (month_id) REFERENCES months(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_expense_categories_month ON expense_categories(month_id);

-- ---------------------------------------------------------------------------
-- Expense Lines (planned_amount is derived)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS expense_lines (
    id              CHAR(36) PRIMARY KEY,
    category_id     CHAR(36) NOT NULL,
    name            VARCHAR(255) NOT NULL,
    current_balance BIGINT NULL,
    minimum_payment BIGINT NULL,
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    CONSTRAINT fk_expense_lines_category FOREIGN KEY (category_id) REFERENCES expense_categories(id) ON DELETE CASCADE,
    CONSTRAINT chk_balance CHECK (current_balance IS NULL OR current_balance >= 0),
    CONSTRAINT chk_min_payment CHECK (minimum_payment IS NULL OR minimum_payment >= 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_expense_lines_category ON expense_lines(category_id);

-- ---------------------------------------------------------------------------
-- Allocations
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS allocations (
    id              CHAR(36) PRIMARY KEY,
    expense_line_id CHAR(36) NOT NULL,
    paycheck_id     CHAR(36) NOT NULL,
    amount          BIGINT NOT NULL,
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    UNIQUE KEY allocations_expense_paycheck_unique (expense_line_id, paycheck_id),
    CONSTRAINT fk_allocations_expense FOREIGN KEY (expense_line_id) REFERENCES expense_lines(id) ON DELETE CASCADE,
    CONSTRAINT fk_allocations_paycheck FOREIGN KEY (paycheck_id) REFERENCES paychecks(id) ON DELETE CASCADE,
    CONSTRAINT chk_allocation_amount CHECK (amount > 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_allocations_paycheck ON allocations(paycheck_id);
CREATE INDEX idx_allocations_expense ON allocations(expense_line_id);

-- ---------------------------------------------------------------------------
-- Transactions
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS transactions (
    id              CHAR(36) PRIMARY KEY,
    month_id        CHAR(36) NOT NULL,
    date            DATE NOT NULL,
    amount          BIGINT NOT NULL,
    payee           VARCHAR(255),
    notes           TEXT,
    expense_line_id CHAR(36) NULL,
    paycheck_id     CHAR(36) NULL,
    created_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at      DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    CONSTRAINT fk_transactions_month FOREIGN KEY (month_id) REFERENCES months(id) ON DELETE CASCADE,
    CONSTRAINT fk_transactions_expense FOREIGN KEY (expense_line_id) REFERENCES expense_lines(id) ON DELETE SET NULL,
    CONSTRAINT fk_transactions_paycheck FOREIGN KEY (paycheck_id) REFERENCES paychecks(id) ON DELETE SET NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_transactions_month ON transactions(month_id);
CREATE INDEX idx_transactions_date ON transactions(date);
CREATE INDEX idx_transactions_paycheck ON transactions(paycheck_id);
CREATE INDEX idx_transactions_expense ON transactions(expense_line_id);

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
-- Seed data
-- ---------------------------------------------------------------------------
INSERT IGNORE INTO users (id, email, password_hash, full_name, timezone, currency)
VALUES (
    '11111111-1111-1111-1111-111111111111',
    'demo@paycheckzero.app',
    '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.G2oQ.Y5zqK8K2i',
    'Demo User',
    'America/Denver',
    'USD'
);

INSERT IGNORE INTO months (id, user_id, year_month, status)
VALUES (
    '22222222-2222-2222-2222-222222222222',
    '11111111-1111-1111-1111-111111111111',
    '2026-09-01',
    'draft'
);

INSERT IGNORE INTO expense_categories (id, month_id, name, sort_order) VALUES
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
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa11', '22222222-2222-2222-2222-222222222222', 'Other', 110);

INSERT IGNORE INTO income_lines (id, month_id, name, planned_amount, schedule_type, recurrence_rule) VALUES
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '22222222-2222-2222-2222-222222222222', 'Primary Job', 200000, 'recurring', 'FREQ=WEEKLY;INTERVAL=2;BYDAY=FR'),
    ('bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2', '22222222-2222-2222-2222-222222222222', 'Side Hustle', 40000, 'one_off', NULL);

INSERT IGNORE INTO paychecks (id, income_line_id, date, planned_amount, status) VALUES
    ('cccccccc-cccc-cccc-cccc-ccccccccccc1', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '2026-09-04', 200000, 'planned'),
    ('cccccccc-cccc-cccc-cccc-ccccccccccc2', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1', '2026-09-18', 200000, 'planned'),
    ('cccccccc-cccc-cccc-cccc-ccccccccccc3', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2', '2026-09-10', 40000, 'planned');

INSERT IGNORE INTO expense_lines (id, category_id, name, current_balance, minimum_payment) VALUES
    ('dddddddd-dddd-dddd-dddd-ddddddddddd1', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa3', 'Rent', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd2', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', 'Groceries', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd3', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa5', 'Restaurants', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd4', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa4', 'Gas', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd5', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaa10', 'Student Loan', 1250000, 35000),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd6', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1', 'Church Tithe', NULL, NULL),
    ('dddddddd-dddd-dddd-dddd-ddddddddddd7', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa2', 'Emergency Fund', NULL, NULL);

INSERT IGNORE INTO allocations (id, expense_line_id, paycheck_id, amount) VALUES
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
    ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee14', 'dddddddd-dddd-dddd-dddd-ddddddddddd7', 'cccccccc-cccc-cccc-cccc-ccccccccccc3', 25000);

INSERT IGNORE INTO transactions (id, month_id, date, amount, payee, notes, expense_line_id, paycheck_id) VALUES
    ('ffffffff-ffff-ffff-ffff-fffffffffff1', '22222222-2222-2222-2222-222222222222', '2026-09-05', -8750, 'Shell', 'Fill-up', 'dddddddd-dddd-dddd-dddd-ddddddddddd4', 'cccccccc-cccc-cccc-cccc-ccccccccccc1'),
    ('ffffffff-ffff-ffff-ffff-fffffffffff2', '22222222-2222-2222-2222-222222222222', '2026-09-06', -4500, 'Kroger', 'Weekly groceries', 'dddddddd-dddd-dddd-dddd-ddddddddddd2', 'cccccccc-cccc-cccc-cccc-ccccccccccc1'),
    ('ffffffff-ffff-ffff-ffff-fffffffffff3', '22222222-2222-2222-2222-222222222222', '2026-09-07', -1200, 'Coffee Shop', NULL, 'dddddddd-dddd-dddd-dddd-ddddddddddd3', NULL);

COMMIT;
