import { test as base, expect, Page, APIRequestContext } from "@playwright/test";
import { spawn, ChildProcess } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const ROOT = path.resolve(__dirname, "..", "..");
const BIN = path.join(ROOT, "target", "debug", process.platform === "win32" ? "paycheckzero.exe" : "paycheckzero");

export const EMAIL = "demo@paycheckzero.test";
export const PASSWORD = "correct-horse-battery";

export type Seed = "empty" | "user" | "basic" | "balanced" | "locked" | "history";

export interface SeedMonth {
  id: string;
  paychecks: string[];
  lines: Record<string, string>;
}

export interface SeedResult {
  months: Record<string, SeedMonth>;
}

type WorkerFixtures = { server: { url: string } };
type TestFixtures = {
  seed: (name: Seed, opts?: { today?: string; access_ttl?: number }) => Promise<SeedResult>;
  login: (page?: Page) => Promise<void>;
};

async function waitHealthy(url: string, proc: ChildProcess) {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (proc.exitCode !== null) throw new Error("server exited early");
    try {
      const r = await fetch(url + "/healthz");
      if (r.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("server did not start");
}

export const test = base.extend<TestFixtures, WorkerFixtures>({
  server: [
    async ({}, use, workerInfo) => {
      const port = 4300 + workerInfo.workerIndex;
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pz-e2e-"));
      const proc = spawn(BIN, [], {
        env: {
          ...process.env,
          PZ_BIND: `127.0.0.1:${port}`,
          // PZ_E2E_DATABASE_URL runs the suite against PostgreSQL/MariaDB
          // (use PZ_WORKERS=1: workers would share that database).
          PZ_DATABASE_URL: process.env.PZ_E2E_DATABASE_URL ?? `sqlite://${path.join(dir, "pz.db")}?mode=rwc`,
          PZ_TEST_MODE: "1",
          PZ_JWT_SECRET: "e2e-secret-e2e-secret-e2e-secret-e2e",
          RUST_LOG: "warn",
        },
        stdio: ["ignore", "inherit", "inherit"],
      });
      const url = `http://127.0.0.1:${port}`;
      await waitHealthy(url, proc);
      await use({ url });
      proc.kill();
      fs.rmSync(dir, { recursive: true, force: true });
    },
    { scope: "worker" },
  ],
  baseURL: async ({ server }, use) => {
    await use(server.url);
  },
  seed: async ({ server }, use) => {
    await use(async (name, opts = {}) => {
      const r = await fetch(server.url + "/__test/reset", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ seed: name, today: opts.today ?? "2026-09-10", access_ttl: opts.access_ttl }),
      });
      if (!r.ok) throw new Error("reset failed: " + (await r.text()));
      return (await r.json()) as SeedResult;
    });
  },
  login: async ({ page }, use) => {
    await use(async (p = page) => {
      await p.goto("/login");
      await p.getByLabel("Email").fill(EMAIL);
      await p.getByLabel("Password").fill(PASSWORD);
      await p.getByRole("button", { name: "Sign in" }).click();
      await p.waitForURL((u) => !u.pathname.startsWith("/login"));
      await waitForContent(p);
    });
  },
});

/** Waits until the SSE content has replaced the skeleton loader. */
export async function waitForContent(page: Page) {
  await expect(page.locator("main#content[aria-busy=false], main#content:not([aria-busy])").first()).toBeVisible();
}

/** Types into a money field and commits it (change + submit). */
export async function commitMoney(page: Page, locator: ReturnType<Page["locator"]>, value: string) {
  await locator.fill(value);
  await locator.press("Enter");
}

export async function api(request: APIRequestContext) {
  return request;
}

export { expect };

/** Page object helpers shared by the suites. */
export const pz = {
  async paycheck(page: Page, s: SeedResult, index = 0, ym = "2026-09") {
    const m = s.months[ym];
    await page.goto(`/months/${m.id}/paychecks/${m.paychecks[index]}`);
    await waitForContent(page);
  },
  async overview(page: Page, s: SeedResult, ym = "2026-09") {
    await page.goto(`/months/${s.months[ym].id}/overview`);
    await waitForContent(page);
  },
  async income(page: Page, s: SeedResult, ym = "2026-09") {
    await page.goto(`/months/${s.months[ym].id}/income`);
    await waitForContent(page);
  },
  async transactions(page: Page, s: SeedResult, ym = "2026-09") {
    await page.goto(`/months/${s.months[ym].id}/transactions`);
    await waitForContent(page);
  },
  async reports(page: Page, s: SeedResult, ym = "2026-09") {
    await page.goto(`/months/${s.months[ym].id}/reports`);
    await waitForContent(page);
  },
  line(page: Page, name: string) {
    return page.locator(`li.line[data-line="${name}"]`);
  },
  category(page: Page, name: string) {
    return page.locator(`details.category[data-category="${name}"]`);
  },
  sts(page: Page) {
    return page.locator("#safe-to-spend");
  },
  toast(page: Page) {
    return page.locator("#toasts [data-toast]").last();
  },
  acceptDialogs(page: Page) {
    page.on("dialog", (d) => d.accept());
  },
  async apiToken(server: { url: string }) {
    const r = await fetch(server.url + "/api/v1/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
    });
    return ((await r.json()) as { access_token: string }).access_token;
  },
};
