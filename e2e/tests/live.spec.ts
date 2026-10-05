import { test, expect, pz, waitForContent, EMAIL } from "./fixtures";

const PARTNER = "partner@paycheckzero.test";
const PARTNER_PW = "partner-password";

test("two people share a budget and see each other's changes live", async ({ page, browser, server, seed, login }) => {
  await seed("basic");
  await login();
  pz.acceptDialogs(page);

  // Owner adds a partner.
  await page.goto("/settings");
  await waitForContent(page);
  await page.locator("#member-email").fill(PARTNER);
  await page.locator("#member-password").fill(PARTNER_PW);
  await page.getByRole("button", { name: "Add person" }).click();
  await expect(pz.toast(page)).toContainText(`${PARTNER} can now sign in`);
  await expect(page.locator("#members")).toContainText(PARTNER);

  // Partner signs in on another device.
  const other = await browser.newContext({ baseURL: server.url });
  const p2 = await other.newPage();
  await p2.goto("/login");
  await p2.getByLabel("Email").fill(PARTNER);
  await p2.getByLabel("Password").fill(PARTNER_PW);
  await p2.getByRole("button", { name: "Sign in" }).click();
  await p2.waitForURL((u) => !u.pathname.startsWith("/login"));
  await p2.goto("/settings");
  await waitForContent(p2);
  await expect(p2.getByText(`You're sharing ${EMAIL}'s budget.`)).toBeVisible();

  // Owner watches the months list; partner adds a month; it appears without a reload.
  await page.goto("/months");
  await waitForContent(page);
  await expect(page.getByText("December 2026")).toHaveCount(0);
  await p2.goto("/months");
  await waitForContent(p2);
  await p2.getByRole("button", { name: "New month" }).first().click();
  await p2.getByLabel("Month", { exact: true }).fill("2026-12");
  await p2.getByRole("button", { name: "Create month" }).click();
  await expect(p2).toHaveURL(/\/income$/);
  await expect(page.getByText("December 2026").first()).toBeVisible();

  // Mid-edit (dialog open): the update waits until the owner is done.
  await page.getByRole("button", { name: "New month" }).first().click();
  await p2.goto("/months");
  await waitForContent(p2);
  await p2.getByRole("button", { name: "New month" }).first().click();
  await p2.getByLabel("Month", { exact: true }).fill("2027-01");
  await p2.getByRole("button", { name: "Create month" }).click();
  await expect(p2).toHaveURL(/\/income$/);
  await page.waitForTimeout(1000);
  await expect(page.getByText("January 2027")).toHaveCount(0);
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByText("January 2027").first()).toBeVisible();

  await other.close();
});
