import { defineConfig, devices } from "@playwright/test";

/** Screenshot tour of every screen for UX review: `npx playwright test -c tour.config.ts`. */
export default defineConfig({
  testDir: "./tour",
  globalSetup: "./global-setup.ts",
  globalTeardown: "./global-teardown.ts",
  workers: 2,
  reporter: [["list"]],
  use: { timezoneId: "America/Chicago", locale: "en-US" },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "phone", use: { ...devices["Pixel 7"] } },
  ],
});
