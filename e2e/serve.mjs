// E2E harness: fresh DB, boot the release server on PORT (default 3099),
// seed a demo user + two months via the public API, then stay alive.
// Playwright's webServer config manages lifecycle ("reset + seed per run").

import { spawn } from 'node:child_process';
import { rmSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = path.dirname(fileURLToPath(import.meta.url));
const runDir = path.join(dir, '.run');
const binary = path.join(dir, '..', 'target', 'release', 'paycheckzero-web');
const port = process.env.PORT || '3099';
const api = `http://127.0.0.1:${port}`;

async function waitHealth(ms = 120000) {
  const start = Date.now();
  for (;;) {
    try {
      const res = await fetch(`${api}/health`);
      if (res.ok) return;
    } catch (_) { /* not up yet */ }
    if (Date.now() - start > ms) throw new Error(`server not healthy on ${api}`);
    await new Promise((r) => setTimeout(r, 250));
  }
}

async function apiPost(pathname, body, token) {
  const res = await fetch(api + pathname, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify(body),
  });
  if (![200, 201, 202, 204, 409].includes(res.status)) {
    throw new Error(`${pathname} -> ${res.status}: ${await res.text()}`);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null;
}

async function apiGet(pathname, token) {
  const res = await fetch(api + pathname, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!res.ok) throw new Error(`${pathname} -> ${res.status}`);
  return res.json();
}

async function seedMonth(token, ym, salary, rentCents, txCents, incomeLineName = 'Salary') {
  const m = await apiPost('/api/v1/months', { year_month: `${ym}-01` }, token);
  const cats = await apiGet(`/api/v1/months/${m.id}/categories`, token);
  const housing = cats.find((c) => c.name === 'Housing');
  const rent = await apiPost(
    `/api/v1/months/${m.id}/expense-lines`,
    { category_id: housing.id, name: 'Rent' },
    token,
  );
  const il = await apiPost(
    `/api/v1/months/${m.id}/income-lines`,
    { name: incomeLineName, planned_amount: salary, schedule_type: 'recurring' },
    token,
  );
  const pc = await apiPost(
    `/api/v1/months/${m.id}/paychecks`,
    { income_line_id: il.id, date: `${ym}-01`, planned_amount: salary },
    token,
  );
  await apiPost(
    `/api/v1/paychecks/${pc.id}/allocations`,
    { expense_line_id: rent.id, amount: rentCents },
    token,
  );
  if (txCents) {
    await apiPost(
      `/api/v1/months/${m.id}/transactions`,
      { date: `${ym}-03`, amount: -txCents, expense_line_id: rent.id },
      token,
    );
  }
  return m.id;
}

async function seed() {
  await apiPost('/api/v1/auth/register', { email: 'e2e@example.com', password: 'password123' });
  const login = await apiPost('/api/v1/auth/login', { email: 'e2e@example.com', password: 'password123' });
  const token = login.access_token;
  // 2026-07: Salary 90000, Rent 50000 allocated, -4000 spent (safe-to-spend $400).
  await seedMonth(token, '2026-07', 90000, 50000, 4000);
  // 2026-06: leaner month for the month-switcher test.
  await seedMonth(token, '2026-06', 80000, 50000, 0);
}

rmSync(runDir, { recursive: true, force: true });
mkdirSync(runDir, { recursive: true });

const server = spawn(binary, [], {
  cwd: runDir,
  env: {
    ...process.env,
    PORT: String(port),
    PAYCHECKZERO_JWT_SECRET: 'e2e-secret-0123456789abcdefghijklmnop',
  },
  stdio: 'inherit',
});
process.on('exit', () => {
  try { server.kill('SIGTERM'); } catch (_) { /* already dead */ }
});
server.on('exit', () => process.exit(0));

await waitHealth();
await seed();
console.log(`E2E seed READY on ${api}`);
setInterval(() => {}, 1 << 30); // stay alive until killed