//! Portable schema (spec §8). One script serves SQLite, PostgreSQL and
//! MariaDB: identifiers and dates are VARCHAR text, money is BIGINT cents,
//! foreign keys are table-level, and `year_month` is double-quoted (MariaDB
//! runs with `ANSI_QUOTES`).
//!
//! Deviations from the §8 reference DDL, all additive or for portability:
//! - UUID / DATE / TIMESTAMPTZ are stored as VARCHAR text (ISO formats).
//! - `users` gains `timezone`, `currency`, `token_version`, `last_month_id`
//!   (spec §13.5, §13.9, §3).
//! - `months` gains `reassigning`, `archived_at`, `version` (§2.9, §2.12,
//!   optimistic concurrency).
//! - `expense_categories.kind`, `expense_lines.sort_order` /
//!   `target_amount`, and a `position` column on ordered children.
//! - `refresh_tokens` and `sync_ops` support auth and offline sync.

/// Ordered migrations: (version, statements).
pub const MIGRATIONS: &[(i64, &[&str])] = &[(1, V1), (2, V2), (3, V3), (4, V4), (5, V5)];

/// v5: bank sync. Imported transactions and linked accounts keep the bank's
/// id; `bank_links` holds connections (tokens encrypted by the web layer);
/// `bank_seen` remembers every bank transaction already handled so a sync
/// never imports one twice, even after it was merged into a transfer.
const V5: &[&str] = &[
    "ALTER TABLE transactions ADD COLUMN external_id VARCHAR(100)",
    "ALTER TABLE accounts ADD COLUMN external_id VARCHAR(100)",
    "ALTER TABLE accounts ADD COLUMN link_id VARCHAR(36)",
    r#"CREATE TABLE bank_links (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    user_id         VARCHAR(36) NOT NULL,
    provider        VARCHAR(20) NOT NULL,
    enrollment_id   VARCHAR(100) NOT NULL,
    institution     VARCHAR(200) NOT NULL,
    access_token    VARCHAR(1000) NOT NULL,
    status          VARCHAR(20) NOT NULL,
    last_error      VARCHAR(1000),
    import_from     VARCHAR(10) NOT NULL,
    last_sync       VARCHAR(40),
    cursor          VARCHAR(4000),
    created_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE bank_seen (
    user_id         VARCHAR(36) NOT NULL,
    external_id     VARCHAR(100) NOT NULL,
    created_at      VARCHAR(40) NOT NULL,
    PRIMARY KEY (user_id, external_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    "CREATE INDEX idx_transactions_external ON transactions(external_id)",
];

/// v4: retirement and investment accounts, with payroll contributions and
/// market growth as adjustment kinds. CHECK constraints can't be altered
/// portably, so both tables are rebuilt and renamed (renames carry the
/// foreign key along in SQLite, PostgreSQL and MariaDB).
const V4: &[&str] = &[
    r#"CREATE TABLE accounts_v4 (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    user_id         VARCHAR(36) NOT NULL,
    name            VARCHAR(100) NOT NULL,
    kind            VARCHAR(16) NOT NULL CHECK (kind IN ('checking', 'savings', 'cash', 'credit_card', 'retirement', 'investment')),
    sort_order      BIGINT NOT NULL DEFAULT 0,
    archived        BIGINT NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    credit_limit    BIGINT CHECK (credit_limit IS NULL OR credit_limit >= 0),
    apr_bp          BIGINT CHECK (apr_bp IS NULL OR apr_bp >= 0),
    minimum_payment BIGINT CHECK (minimum_payment IS NULL OR minimum_payment >= 0),
    reconciled_on   VARCHAR(10),
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    "INSERT INTO accounts_v4 (id, user_id, name, kind, sort_order, archived, credit_limit, apr_bp, minimum_payment, reconciled_on, created_at, updated_at) SELECT id, user_id, name, kind, sort_order, archived, credit_limit, apr_bp, minimum_payment, reconciled_on, created_at, updated_at FROM accounts",
    r#"CREATE TABLE account_adjustments_v4 (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    account_id      VARCHAR(36) NOT NULL,
    date            VARCHAR(10) NOT NULL,
    amount          BIGINT NOT NULL,
    kind            VARCHAR(12) NOT NULL CHECK (kind IN ('opening', 'reconcile', 'contribution', 'growth')),
    created_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts_v4(id) ON DELETE CASCADE
)"#,
    "INSERT INTO account_adjustments_v4 (id, account_id, date, amount, kind, created_at) SELECT id, account_id, date, amount, kind, created_at FROM account_adjustments",
    "DROP TABLE account_adjustments",
    "DROP TABLE accounts",
    "ALTER TABLE accounts_v4 RENAME TO accounts",
    "ALTER TABLE account_adjustments_v4 RENAME TO account_adjustments",
    "CREATE INDEX idx_accounts_user ON accounts(user_id)",
    "CREATE INDEX idx_adjustments_account ON account_adjustments(account_id)",
];

/// v3: accounts (bank, cash, credit cards), their balance adjustments,
/// goals, and the account / transfer target on transactions.
const V3: &[&str] = &[
    "ALTER TABLE transactions ADD COLUMN account_id VARCHAR(36)",
    "ALTER TABLE transactions ADD COLUMN transfer_account_id VARCHAR(36)",
    r#"CREATE TABLE accounts (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    user_id         VARCHAR(36) NOT NULL,
    name            VARCHAR(100) NOT NULL,
    kind            VARCHAR(16) NOT NULL CHECK (kind IN ('checking', 'savings', 'cash', 'credit_card')),
    sort_order      BIGINT NOT NULL DEFAULT 0,
    archived        BIGINT NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    credit_limit    BIGINT CHECK (credit_limit IS NULL OR credit_limit >= 0),
    apr_bp          BIGINT CHECK (apr_bp IS NULL OR apr_bp >= 0),
    minimum_payment BIGINT CHECK (minimum_payment IS NULL OR minimum_payment >= 0),
    reconciled_on   VARCHAR(10),
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE account_adjustments (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    account_id      VARCHAR(36) NOT NULL,
    date            VARCHAR(10) NOT NULL,
    amount          BIGINT NOT NULL,
    kind            VARCHAR(10) NOT NULL CHECK (kind IN ('opening', 'reconcile')),
    created_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE goals (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    user_id         VARCHAR(36) NOT NULL,
    name            VARCHAR(100) NOT NULL,
    kind            VARCHAR(10) NOT NULL CHECK (kind IN ('save', 'payoff')),
    target_amount   BIGINT NOT NULL CHECK (target_amount >= 0),
    target_month    VARCHAR(10),
    line_name       VARCHAR(100),
    account_id      VARCHAR(36),
    start_month     VARCHAR(10) NOT NULL,
    starting_amount BIGINT NOT NULL DEFAULT 0 CHECK (starting_amount >= 0),
    sort_order      BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    "CREATE INDEX idx_accounts_user ON accounts(user_id)",
    "CREATE INDEX idx_adjustments_account ON account_adjustments(account_id)",
    "CREATE INDEX idx_goals_user ON goals(user_id)",
];

/// v2: split transactions (parts of one payment share a group id).
const V2: &[&str] = &[
    "ALTER TABLE transactions ADD COLUMN split_group VARCHAR(36)",
    "CREATE INDEX idx_transactions_split ON transactions(split_group)",
];

const V1: &[&str] = &[
    r#"CREATE TABLE users (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    email           VARCHAR(255) NOT NULL UNIQUE,
    password_hash   VARCHAR(255) NOT NULL,
    timezone        VARCHAR(64) NOT NULL,
    currency        VARCHAR(3) NOT NULL,
    token_version   BIGINT NOT NULL DEFAULT 0,
    last_month_id   VARCHAR(36),
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL
)"#,
    r#"CREATE TABLE refresh_tokens (
    token_hash      VARCHAR(64) NOT NULL PRIMARY KEY,
    user_id         VARCHAR(36) NOT NULL,
    expires_at      VARCHAR(40) NOT NULL,
    created_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE months (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    user_id         VARCHAR(36) NOT NULL,
    "year_month"    VARCHAR(10) NOT NULL,
    status          VARCHAR(10) NOT NULL CHECK (status IN ('draft', 'locked')),
    reassigning     BIGINT NOT NULL DEFAULT 0 CHECK (reassigning IN (0, 1)),
    archived_at     VARCHAR(40),
    version         BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    UNIQUE (user_id, "year_month"),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE income_lines (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    month_id        VARCHAR(36) NOT NULL,
    name            VARCHAR(100) NOT NULL,
    planned_amount  BIGINT NOT NULL CHECK (planned_amount >= 0),
    schedule_type   VARCHAR(10) NOT NULL CHECK (schedule_type IN ('one_off', 'recurring')),
    recurrence_rule VARCHAR(1000),
    position        BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (month_id) REFERENCES months(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE paychecks (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    income_line_id  VARCHAR(36) NOT NULL,
    date            VARCHAR(10) NOT NULL,
    planned_amount  BIGINT NOT NULL CHECK (planned_amount >= 0),
    actual_amount   BIGINT CHECK (actual_amount IS NULL OR actual_amount >= 0),
    status          VARCHAR(10) NOT NULL CHECK (status IN ('planned', 'received', 'skipped')),
    position        BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    UNIQUE (income_line_id, date),
    FOREIGN KEY (income_line_id) REFERENCES income_lines(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE expense_categories (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    month_id        VARCHAR(36) NOT NULL,
    name            VARCHAR(100) NOT NULL,
    sort_order      BIGINT NOT NULL DEFAULT 0,
    kind            VARCHAR(10) NOT NULL DEFAULT 'standard' CHECK (kind IN ('standard', 'debt')),
    position        BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (month_id) REFERENCES months(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE expense_lines (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    category_id     VARCHAR(36) NOT NULL,
    name            VARCHAR(100) NOT NULL,
    sort_order      BIGINT NOT NULL DEFAULT 0,
    current_balance BIGINT CHECK (current_balance IS NULL OR current_balance >= 0),
    minimum_payment BIGINT CHECK (minimum_payment IS NULL OR minimum_payment >= 0),
    target_amount   BIGINT CHECK (target_amount IS NULL OR target_amount >= 0),
    position        BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (category_id) REFERENCES expense_categories(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE allocations (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    expense_line_id VARCHAR(36) NOT NULL,
    paycheck_id     VARCHAR(36) NOT NULL,
    amount          BIGINT NOT NULL CHECK (amount > 0),
    position        BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    UNIQUE (expense_line_id, paycheck_id),
    FOREIGN KEY (expense_line_id) REFERENCES expense_lines(id) ON DELETE CASCADE,
    FOREIGN KEY (paycheck_id) REFERENCES paychecks(id) ON DELETE CASCADE
)"#,
    r#"CREATE TABLE transactions (
    id              VARCHAR(36) NOT NULL PRIMARY KEY,
    month_id        VARCHAR(36) NOT NULL,
    date            VARCHAR(10) NOT NULL,
    amount          BIGINT NOT NULL,
    payee           VARCHAR(200),
    notes           VARCHAR(2000),
    expense_line_id VARCHAR(36),
    paycheck_id     VARCHAR(36),
    position        BIGINT NOT NULL DEFAULT 0,
    created_at      VARCHAR(40) NOT NULL,
    updated_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (month_id) REFERENCES months(id) ON DELETE CASCADE,
    FOREIGN KEY (expense_line_id) REFERENCES expense_lines(id) ON DELETE SET NULL,
    FOREIGN KEY (paycheck_id) REFERENCES paychecks(id) ON DELETE SET NULL
)"#,
    r#"CREATE TABLE sync_ops (
    op_id           VARCHAR(64) NOT NULL PRIMARY KEY,
    user_id         VARCHAR(36) NOT NULL,
    result          VARCHAR(4000) NOT NULL,
    created_at      VARCHAR(40) NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
)"#,
    r#"CREATE INDEX idx_months_user_year ON months(user_id, "year_month")"#,
    "CREATE INDEX idx_paychecks_date ON paychecks(date)",
    "CREATE INDEX idx_allocations_paycheck ON allocations(paycheck_id)",
    "CREATE INDEX idx_allocations_expense ON allocations(expense_line_id)",
    "CREATE INDEX idx_transactions_month ON transactions(month_id)",
    "CREATE INDEX idx_income_lines_month ON income_lines(month_id)",
    "CREATE INDEX idx_categories_month ON expense_categories(month_id)",
    "CREATE INDEX idx_lines_category ON expense_lines(category_id)",
];

/// Tables in child-first order (for resets).
pub const TABLES_CHILD_FIRST: &[&str] = &[
    "bank_seen",
    "bank_links",
    "goals",
    "account_adjustments",
    "accounts",
    "sync_ops",
    "transactions",
    "allocations",
    "paychecks",
    "expense_lines",
    "income_lines",
    "expense_categories",
    "months",
    "refresh_tokens",
    "users",
];
