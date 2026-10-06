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

One ~11 MB binary with all web assets embedded, plus a SQLite file. Docker is optional — see
[Docker](#docker-optional) below.

### Docker (optional)

The image wraps exactly that: one `paycheckzero` binary and a SQLite file in a volume.

**1. Create the session secret.** Without it the server invents a random one at every start
and signs everyone out on each restart.

```bash
export PZ_JWT_SECRET="$(openssl rand -hex 32)"
```

You can skip this by using a `.env` file instead — see
[keep the key outside Docker](#optional-keep-the-key-outside-docker).

**2. Start it.** Compose reads that variable from your shell and pulls the published
image — built for both amd64 and arm64 — from GitHub Container Registry.

```bash
docker compose up -d             # then open http://127.0.0.1:8080
```

To compile from source instead, uncomment `build: .` in `compose.yaml` and use
`docker compose up --build -d`. The first build compiles every dependency and takes a
while on a small machine; later builds reuse BuildKit's cache and recompile only what
changed. If the build runs out of memory, limit parallel jobs:
`docker compose build --build-arg CARGO_BUILD_JOBS=1`.

Create the account in the browser, add your first paycheck, and start funding it.

```bash
docker compose logs -f           # follow the log
docker compose ps                # status and port mapping
docker compose down              # stop; add -v to also delete the data volume
```

The published port is `127.0.0.1:8080`, so nothing on your network can reach it. To reach the
app from your phone, change that line to `"8080:8080"`, allow the port through the firewall, and
open `http://<this computer's IP>:8080`. Leave `PZ_SECURE_COOKIES` alone unless you serve over
HTTPS; set it to `true` in `compose.yaml` once you do.

**3. Keep the data key.** The volume holds `paycheckzero.db` and `paycheckzero.key`. Back up the
whole thing together:

```bash
docker run --rm -v paycheck-zero_paycheckzero-data:/data -v "$PWD":/backup debian:bookworm-slim \
  tar czf /backup/paycheckzero-backup.tgz -C /data .
```

Losing the key does not lose your transactions, but every connected bank has to be linked again.

The volume name is prefixed with the directory name, so it is
`paycheck-zero_paycheckzero-data`; confirm it with `docker volume ls`.

#### Optional: keep the key outside Docker

By default the server writes the key itself on first boot and keeps it in the volume. To hold it
somewhere else (a password manager, for instance), generate one and pass it in:

```bash
export PZ_DATA_KEY="$(openssl rand -hex 32)"     # 64 hex chars = 32 bytes
```

`PZ_DATA_KEY` takes precedence over `PZ_DATA_KEY_FILE`; setting both means the file is ignored.
Losing `PZ_DATA_KEY` makes every stored bank token undecryptable, so store it as carefully as a
password.

Rather than re-export on each new shell, put both in a `.env` file next to `compose.yaml`,
which Compose reads automatically. Start from the committed example:

```bash
cp .env.example .env      # then paste in the two openssl rand -hex 32 values
chmod 600 .env
docker compose up --build -d
```

`.env` is gitignored and `.env.example` is not, so the template is safe to commit. See the
comments in that file for every supported variable.

#### Enabling Plaid in Docker

Bank sync stays off until a client id and secret are set. Uncomment `PZ_PLAID_CLIENT_ID`,
`PZ_PLAID_SECRET` and `PZ_PLAID_ENV` in `compose.yaml`, then `docker compose up -d`. You can also
turn Plaid on later from **Settings → Bank providers** without touching the file.

#### Pre-built image (no build step)

Every version tag publishes a ready-to-run image to GitHub Container Registry,
built for both `linux/amd64` and `linux/arm64`, so it runs natively on Intel/AMD
and Apple Silicon. Pull it instead of compiling:

```bash
docker run -p 8080:8080 -v pz-data:/data \
  -e PZ_JWT_SECRET="$PZ_JWT_SECRET" \
  ghcr.io/larrydewey/paycheck-zero:0.1.0
```

Drop the tag to follow the newest release, or pin the digest (from
`docker buildx imagetools inspect`) when you want a build you can reproduce
exactly. Image contents and provenance are listed at
[github.com/larrydewey/paycheck-zero/pkgs/container/paycheck-zero](https://github.com/larrydewey/paycheck-zero/pkgs/container/paycheck-zero).

#### Without Compose

```bash
docker build -t paycheckzero .
docker run -p 8080:8080 -v pz-data:/data -e PZ_JWT_SECRET="$(openssl rand -hex 32)" paycheckzero
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
