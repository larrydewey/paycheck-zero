# PaycheckZero

**Paycheck-first zero-based budgeting.** You build each month by deciding, paycheck by paycheck, what every paycheck funds. Monthly totals are always derived from those decisions and must reach exactly zero before the month can be locked.

Specification: [`PaycheckZero_Formal_Specification_v0.4.4.md`](./PaycheckZero_Formal_Specification_v0.4.4.md) (frozen). Implementation notes, decisions and status: [`PROGRESS.md`](./PROGRESS.md).

## Quick start

```bash
cargo run -p paycheckzero-web          # http://127.0.0.1:8080, data in ./paycheckzero.db
```

Open the app, create the account, add your first paycheck, and start funding it. To budget with someone else, add them under **Settings › Account › Shared budget**. Each person signs in with their own email, and changes show up live on every open screen.

### Using it from your phone

The server listens only on this computer by default (`127.0.0.1`). To reach it from a phone on the same Wi-Fi:

```bash
PZ_BIND=0.0.0.0:8080 cargo run -p paycheckzero-web
# then on the phone open http://<this computer's IP>:8080   (e.g. http://192.168.1.20:8080)
```

- Allow port 8080 through the computer's firewall if the page doesn't load.
- Leave `PZ_SECURE_COOKIES` unset unless you serve over HTTPS. With it set, browsers drop the sign-in cookie on plain `http://`, and the login page will say so.
- If sign-in does nothing, the login page and the server log (`sign-in ok` / `sign-in failed` lines, with the browser's user agent) now show why.

### Production

```bash
cargo build --release -p paycheckzero-web
PZ_JWT_SECRET="$(openssl rand -hex 32)" PZ_SECURE_COOKIES=true PZ_BIND=0.0.0.0:8080 \
  ./target/release/paycheckzero
```

One ~11 MB binary with all web assets embedded, plus a SQLite file. Docker is optional:

```bash
docker build -t paycheckzero .
docker run -p 8080:8080 -v pz-data:/data -e PZ_JWT_SECRET=... paycheckzero
```

### Configuration (environment variables)

| Variable            | Default                              | Notes |
|---------------------|--------------------------------------|-------|
| `PZ_DATABASE_URL`   | `sqlite://paycheckzero.db?mode=rwc`  | Also `postgres://…` and `mysql://…` / `mariadb://…` |
| `PZ_BIND`           | `127.0.0.1:8080`                     | |
| `PZ_JWT_SECRET`     | random per start                     | ≥ 32 bytes; set it, or sessions end on restart |
| `PZ_SECURE_COOKIES` | `false`                              | Set `true` behind HTTPS |
| `PZ_TEST_MODE`      | `false`                              | Enables `/__test/*` (database reset). **Never in production.** |
| `RUST_LOG`          | `info`                               | |
| `PZ_DATA_KEY`       | from `PZ_DATA_KEY_FILE`              | 64 hex chars; encrypts stored bank tokens |
| `PZ_DATA_KEY_FILE`  | `paycheckzero.key`                   | Created on first start (mode 0600). Back it up with the database; without it, banks need reconnecting |
| `PZ_BANK_SYNC_HOURS`| `6`                                  | Background bank sync interval; `0` turns it off |

### Bank sync (optional)

Connect banks from the **Accounts** tab. Every provider is optional and you can mix them. Transactions are:
- imported once;
- matched to ones you typed in yourself;
- sorted into the line the same payee had last time;
- tagged to the current paycheck.

Card payments seen from both accounts become one transfer, and balances are reconciled to the bank's.

| Provider | What you need | Server settings |
|----------|---------------|-----------------|
| **SimpleFIN Bridge** | An account at [bridge.simplefin.org](https://bridge.simplefin.org) (about $15/year). Link your banks there, create a setup token, paste it in PaycheckZero | None |
| **Plaid** | A [Plaid](https://plaid.com) developer account (production access needs Plaid's approval) | `PZ_PLAID_CLIENT_ID`, `PZ_PLAID_SECRET`, `PZ_PLAID_ENV` (`sandbox`, `production`), optional `PZ_PLAID_COUNTRIES` (default `US`) |

The easiest way to turn Plaid on is **Settings → Bank providers**: enter the client ID and secret.

Saving checks the keys with Plaid. Changes take effect immediately, with no restart. Values saved there override the environment variables above.

Secrets are never shown again after saving. They, and bank access tokens, are stored encrypted with the data key.

## Architecture

| Crate / dir | Role |
|-------------|------|
| `core/`     | Pure Rust domain: `Month` aggregate, invariants, Safe-to-Spend, recurrence, variance, month copy, currency conversion, reports, suggestions. No web/DB dependencies. |
| `storage/`  | sqlx (`Any`) adapter for SQLite, PostgreSQL and MariaDB. Portable schema + migrations, diff-based saves with optimistic versioning. |
| `web/`      | Axum server: Datastar UI (HTML + SSE patches), REST API at `/api/v1`, JWT auth, CSV/JSON export, offline sync, PWA (manifest + service worker). |
| `e2e/`      | Playwright suite (Chromium, Firefox, mobile Safari, mobile Chrome) with axe accessibility checks and visual baselines. |

Money is integer cents everywhere except the UI boundary, where it is shown and typed as dollars and cents.

## Tests

```bash
make test          # clippy + cargo tests + full Playwright suite
cargo test --workspace
cd e2e && npm test # one command; builds the server, isolated server + DB per worker
```

- WebKit (mobile Safari) needs Ubuntu 24.04 system libraries. On other hosts, global setup automatically runs Playwright's official Docker image as a remote WebKit server. Docker must be available; set `PZ_WEBKIT=local` to skip that.
- Storage tests against other engines: `PZ_TEST_DATABASE_URL=postgres://… cargo test -p paycheckzero-storage`.
- E2E against another engine: `PZ_WORKERS=1 PZ_E2E_DATABASE_URL=postgres://… npm test`.
- Refresh visual baselines after intended UI changes: `npm run test:update`.
