# PaycheckZero

**Paycheck-first zero-based budgeting application**

## Specification

**[PaycheckZero_Formal_Specification_v0.4.4.md](./PaycheckZero_Formal_Specification_v0.4.4.md)**

Single source of truth. Implementation must conform exactly.

### Tech Stack

| Layer        | Choice                                            |
|--------------|---------------------------------------------------|
| Language     | Rust (pure domain crate + web server)             |
| Frontend     | Datastar                                          |
| Database     | **SQLite primary** (STRICT); Postgres / MariaDB via config |
| Mobile (v1)  | Responsive web + PWA                              |
| Deployment   | Single binary + SQLite preferred; Docker optional |

### Database Setup

See **`db/README.md`**. Separate scripts:

- `db/sqlite/setup.sql`   ← primary, uses STRICT tables
- `db/postgres/setup.sql`
- `db/mariadb/setup.sql`

All include the v0.4.4 columns (`timezone`, `currency`, `archived_at`) and derived views for Planned / Spent / fully-allocated paychecks.

### Money presentation

- **Backend / DB / domain**: integer cents only
- **Frontend**: users see and type dollars and cents (e.g. 12.50). Conversion only at the UI boundary.

### Key Domain Rules

- Paychecks must be fully allocated
- Spent = sum of linked transactions (read-only)
- Past / current / future months supported
- Soft-delete (archive) + permanent delete with confirmation
- Inline editing, collapsible categories with summaries
- Playwright E2E required for every user-visible feature
