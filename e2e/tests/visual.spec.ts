import { test, expect, pz, waitForContent } from "./fixtures";

// Baselines live next to this file; refresh with `npm run test:update`.
test.describe("visual regression", () => {
  test.use({ serviceWorkers: "block" });

  test("sign in", async ({ page, seed }) => {
    await seed("user");
    await page.goto("/login");
    await expect(page).toHaveScreenshot("login.png", { fullPage: true });
  });

  test("paycheck view", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    await expect(page).toHaveScreenshot("paycheck.png", { fullPage: true });
  });

  test("monthly overview", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.overview(page, s);
    await expect(page).toHaveScreenshot("overview.png", { fullPage: true });
  });

  test("months list", async ({ page, seed, login }) => {
    await seed("history");
    await login();
    await page.goto("/months");
    await waitForContent(page);
    await expect(page).toHaveScreenshot("months.png", { fullPage: true });
  });

  test("reports", async ({ page, seed, login }) => {
    const s = await seed("history");
    await login();
    await pz.reports(page, s);
    await expect(page).toHaveScreenshot("reports.png", { fullPage: true });
  });
});
