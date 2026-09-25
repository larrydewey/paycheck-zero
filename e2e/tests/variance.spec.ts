import { test, expect, pz } from "./fixtures";

test("variance on a locked month: record actual, re-assign, re-lock", async ({ page, seed, login }) => {
  const s = await seed("locked");
  await login();
  await page.getByText("Paycheck details").click();
  await page.getByLabel("Amount actually received").fill("1,950");
  await page.getByRole("button", { name: "Record actual" }).click();
  const panel = page.locator("#variance-panel");
  await expect(panel).toContainText("Sep 4: planned $2,000.00, received $1,950.00.");
  await expect(panel).toContainText("Net difference between actual and planned income: -$50.00.");
  await expect(panel).toContainText("Emergency Fund — $1,875.00 not yet spent");
  await panel.getByRole("button", { name: "Re-assign variance" }).click();
  await expect(page.getByText("Variance re-assignment is open")).toBeVisible();
  // Can't re-lock before the variance is applied.
  await page.getByRole("button", { name: "Done — re-lock month" }).click();
  await expect(pz.toast(page)).toContainText("The Sep 4 paycheck's actual still differs from its plan.");
  await page.locator("#variance-panel").getByRole("button", { name: "Use actual as planned" }).click();
  await expect(page.locator("#toasts")).toContainText("Funding was reduced for: Emergency Fund.");
  await expect(page.locator('[data-stat="planned"]')).toHaveText("$1,950.00");
  // Allocations are editable again while re-assigning.
  const rent = page.getByLabel("Planned for Rent from this paycheck");
  await rent.fill("1150");
  await rent.press("Enter");
  await expect(page.locator("#zero-status")).toContainText("$50.00 left");
  await rent.fill("1200");
  await rent.press("Enter");
  await expect(page.locator("#zero-status")).toContainText("Every dollar has a job");
  await page.getByRole("button", { name: "Done — re-lock month" }).click();
  await expect(pz.toast(page)).toContainText("Variance resolved. The month is locked again.");
  await expect(page.getByLabel("Planned for Rent from this paycheck")).toHaveCount(0);
  expect(s.months["2026-09"]).toBeTruthy();
});

test("variance in a draft month can be applied directly", async ({ page, seed, login }) => {
  await seed("balanced");
  await login();
  await page.getByText("Paycheck details").click();
  await page.getByLabel("Amount actually received").fill("2100");
  await page.getByRole("button", { name: "Record actual" }).click();
  await expect(page.locator("#variance-panel")).toContainText("Net difference between actual and planned income: $100.00.");
  await page.locator("#variance-panel").getByRole("button", { name: "Use actual as planned" }).click();
  await expect(page.locator("#zero-status")).toContainText("$100.00 left to assign");
  await expect(page.locator("#variance-panel")).toHaveCount(0);
});
