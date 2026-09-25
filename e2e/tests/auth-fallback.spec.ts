import { test, expect, EMAIL, PASSWORD, waitForContent } from "./fixtures";

test.describe("sign-in resilience", () => {
  test("sign-in works without JavaScript (plain form post)", async ({ browser, seed, server }) => {
    await seed("basic");
    const ctx = await browser.newContext({ javaScriptEnabled: false, baseURL: server.url });
    const page = await ctx.newPage();
    await page.goto("/login");
    await page.getByLabel("Email").fill(EMAIL);
    await page.getByLabel("Password").fill("wrong-password");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page).toHaveURL(/\/login\?error=INVALID_CREDENTIALS$/);
    await expect(page.getByRole("alert")).toHaveText("That email and password don't match.");
    expect(page.url()).not.toContain("password");
    await page.getByLabel("Email").fill(EMAIL);
    await page.getByLabel("Password").fill(PASSWORD);
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page).toHaveURL(/\/paychecks\//);
    await ctx.close();
  });

  test("a browser that drops the session cookie gets an explanation", async ({ page, seed }) => {
    await seed("basic");
    await page.goto("/?signed_in=1");
    await expect(page).toHaveURL(/error=COOKIE_BLOCKED/);
    await expect(page.getByRole("alert")).toContainText("didn't keep the sign-in cookie");
  });

  test("cross-site sign-in posts are refused", async ({ seed, server }) => {
    await seed("basic");
    const r = await fetch(server.url + "/ui/login", {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Origin: "https://evil.example" },
      body: `email=${encodeURIComponent(EMAIL)}&password=${PASSWORD}`,
      redirect: "manual",
    });
    expect(r.status).toBe(403);
  });

  test("the scripts-failed banner stays hidden when scripts run", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    await page.waitForTimeout(4500);
    await expect(page.locator("#script-banner")).toBeHidden();
    await waitForContent(page);
  });

  test("the scripts-failed banner appears when Datastar can't load", async ({ page, seed }) => {
    await seed("basic");
    await page.route(/datastar\.js/, (r) => r.abort());
    await page.goto("/login");
    await expect(page.locator("#script-banner")).toBeVisible({ timeout: 8000 });
  });
});
