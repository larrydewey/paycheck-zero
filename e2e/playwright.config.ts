import { defineConfig, devices } from "@playwright/test";

/**
 * PaycheckZero E2E (spec §13.1). Run everything with `npm test` from e2e/.
 * Each worker starts its own server + SQLite file; every test resets the
 * database and loads a deterministic seed with a pinned "today".
 */
export default defineConfig({
  testDir: "./tests",
  globalSetup: "./global-setup.ts",
  globalTeardown: "./global-teardown.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: process.env.PZ_WORKERS ? Number(process.env.PZ_WORKERS) : 4,
  timeout: 30_000,
  expect: {
    timeout: 7_000,
    toHaveScreenshot: { maxDiffPixelRatio: 0.02, animations: "disabled", caret: "hide" },
  },
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    timezoneId: "America/Chicago",
    locale: "en-US",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    {
      name: "mobile-safari",
      use: {
        ...devices["iPhone 13"],
        // Set by global setup when WebKit runs in Playwright's Docker image.
        connectOptions: process.env.PZ_WEBKIT_WS ? { wsEndpoint: process.env.PZ_WEBKIT_WS } : undefined,
      },
    },
    { name: "mobile-chrome", use: { ...devices["Pixel 7"] } },
  ],
});
