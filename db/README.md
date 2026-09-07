# PaycheckZero Database Setup

Specification version: **0.4.3**

Separate scripts are provided for each supported engine.

## Engines

| Engine     | Path                    | Notes                                      |
|------------|-------------------------|--------------------------------------------|
| **SQLite** (primary) | `sqlite/setup.sql` | Uses `STRICT` tables (SQLite 3.37+). Preferred for simple / no-Docker deploys. |
| PostgreSQL | `postgres/setup.sql`    | PostgreSQL 15+                             |
| MariaDB    | `mariadb/setup.sql`     | MariaDB 10.6+ or MySQL 8.0+                |

## Quick start (SQLite)

```bash
sqlite3 paycheckzero.db < sqlite/setup.sql
```

## What the schema includes (all engines)

- `users` with `timezone` and `currency`
- `months` with `archived_at` for soft-delete
- Full domain tables: income_lines, paychecks, expense_categories, expense_lines, allocations, transactions
- Views:
  - `v_expense_line_planned` (derived Planned)
  - `v_expense_line_spent` (derived Spent)
  - `v_paycheck_allocated` (including `is_fully_allocated`)
- Deterministic seed data for development and Playwright tests

## Money

All amounts are integer **cents** (or equivalent smallest currency unit). Floating-point money is forbidden.

## Soft-delete

- `months.archived_at IS NULL` → active
- `months.archived_at IS NOT NULL` → archived (hidden from default lists, restorable)
- Permanent delete = actual `DELETE` after strong confirmation

## Demo credentials

- Email: `demo@paycheckzero.app`
- Password: `Password123!`
