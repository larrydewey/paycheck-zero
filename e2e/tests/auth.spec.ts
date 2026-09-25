import { test, expect, EMAIL, PASSWORD, waitForContent, pz } from "./fixtures";

test.describe("auth", () => {
  test("unauthenticated visitors are sent to sign in", async ({ page, seed }) => {
    await seed("user");
    await page.goto("/months");
    await expect(page).toHaveURL(/\/login$/);
    await expect(page.getByRole("heading", { name: "Sign in" })).toBeVisible();
  });

  test("register → onboarding: add first paycheck → fund it", async ({ page, seed }) => {
    await seed("empty");
    await page.goto("/login");
    await page.getByRole("link", { name: "Create account" }).click();
    await page.getByLabel("Email").fill("new@example.com");
    await page.getByLabel("Password").fill("a-good-password");
    await page.getByRole("button", { name: "Create account" }).click();
    await expect(page).toHaveURL(/\/income\?welcome=1$/);
    await waitForContent(page);
    await expect(page.getByRole("heading", { name: "Welcome! Start with your paycheck." })).toBeVisible();
    await expect(page.getByRole("heading", { name: "September 2026 income" })).toBeVisible();
    // Add the first paycheck; we land on it, ready to fund.
    await page.getByLabel("Name").fill("Day Job");
    await page.getByLabel("Amount per paycheck").fill("1,500");
    await page.getByLabel("Schedule").selectOption("semi_monthly");
    await page.getByLabel("Days of the month").fill("1, 15");
    await page.getByRole("button", { name: "Add income" }).click();
    await expect(page).toHaveURL(/\/paychecks\//);
    await waitForContent(page);
    await expect(pz.sts(page)).toHaveText("$1,500.00");
    await expect(page.getByText("$1,500.00 of this paycheck is still unassigned")).toBeVisible();
  });

  test("registration is closed once the account exists", async ({ page, seed }) => {
    await seed("user");
    await page.goto("/register");
    await expect(page).toHaveURL(/\/login$/);
    await expect(page.getByRole("link", { name: "Create account" })).toHaveCount(0);
  });

  test("wrong password shows a clear message", async ({ page, seed }) => {
    await seed("user");
    await page.goto("/login");
    await page.getByLabel("Email").fill(EMAIL);
    await page.getByLabel("Password").fill("wrong-password");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByRole("alert")).toHaveText("That email and password don't match.");
  });

  test("sign in lands on the last month's current paycheck; sign out", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    // Today is Sep 10 → the Sep 4 paycheck is current.
    await expect(page).toHaveURL(new RegExp(`/paychecks/${s.months["2026-09"].paychecks[0]}$`));
    await expect(page.getByRole("heading", { name: "Paycheck Sep 4 — Acme Payroll" })).toBeAttached();
    await page.getByRole("link", { name: "Settings" }).click();
    await waitForContent(page);
    await page.getByRole("button", { name: "Sign out", exact: true }).click();
    await expect(page).toHaveURL(/\/login$/);
    await page.goto("/months");
    await expect(page).toHaveURL(/\/login$/);
  });

  test("expired access token is refreshed silently", async ({ page, seed, login }) => {
    await seed("basic", { access_ttl: 1 });
    await login();
    await page.waitForTimeout(2200);
    await page.getByRole("link", { name: "Month overview" }).click();
    await waitForContent(page);
    await expect(page.getByRole("heading", { name: "September 2026 overview" })).toBeVisible();
  });

  test("log out of all devices ends other sessions", async ({ browser, page, seed, login }) => {
    await seed("basic");
    await login();
    const other = await browser.newContext();
    const page2 = await other.newPage();
    await login(page2);
    await page.goto("/settings");
    await waitForContent(page);
    pz.acceptDialogs(page);
    await page.getByRole("button", { name: "Sign out of all devices" }).click();
    await expect(page).toHaveURL(/\/login$/);
    await page2.reload();
    await expect(page2).toHaveURL(/\/login$/);
    await other.close();
  });
});

export { PASSWORD };
