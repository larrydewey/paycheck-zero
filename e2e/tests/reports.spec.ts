import { test, expect, pz, waitForContent } from "./fixtures";

async function openCompare(page: import("@playwright/test").Page) {
  await page.getByRole("link", { name: "Compare", exact: true }).click();
  await waitForContent(page);
  await expect(page.locator("#mom")).toBeVisible();
}

const row = (page: import("@playwright/test").Page, table: string, key: string) => page.locator(`#${table} tr[data-row="${key}"]`);

test.describe("reports (seeded history)", () => {
  test.beforeEach(async ({ page, seed, login }) => {
    const s = await seed("history");
    await login();
    await pz.reports(page, s);
  });

  test("quick-win summary cards", async ({ page }) => {
    await expect(page.locator('[data-card="income"]')).toContainText("$4,000.00");
    await expect(page.locator('[data-card="income"]')).toContainText("Received $0.00 · 0 of 2 paychecks");
    await expect(page.locator('[data-card="expenses"]')).toContainText("Spent $137.20 (3%)");
    await expect(page.locator('[data-card="remaining"]')).toContainText("$0.00");
    await expect(page.locator('[data-card="variance"]')).toContainText("No paychecks received yet");
    await expect(page.locator('[data-card="trend"]')).toContainText("Under plan");
    await expect(page.locator('[data-card="trend"]')).toContainText("$3,862.80 left to spend");
  });

  test("donuts and gauges: where the plan goes, where the money went, debt-to-income, savings rate", async ({ page }) => {
    const planned = page.locator("#donut-planned");
    await expect(planned).toContainText("$4,000.00");
    await expect(planned.locator('[data-slice="Saving"]')).toContainText("$1,875.00 · 47%");
    await expect(page.locator("#donut-spent").locator('[data-slice="Food"]')).toContainText("$85.20 · 62%");
    const dti = page.locator("#gauge-dti");
    await expect(dti).toContainText("2%");
    await expect(dti).toContainText("Healthy");
    await expect(dti).toContainText("Total debt balance $3,200.00");
    await expect(page.locator("#gauge-savings")).toContainText("47%");
  });

  test("planned vs spent chart", async ({ page }) => {
    await expect(page.locator('[data-bar="Food"]')).toContainText("$85.20 of $600.00");
    await expect(page.locator('[data-bar="Uncategorized"]')).toHaveClass(/over/);
    await expect(page.locator('[data-bar="Giving"]')).toHaveCount(0);
  });

  test("month over month compares with August", async ({ page }) => {
    await openCompare(page);
    await expect(row(page, "mom", "income")).toContainText("$4,000.00$0.00$4,000.00$4,050.00".replace(/\$/g, "$"));
    await expect(row(page, "mom", "expenses")).toContainText("$137.20");
    await expect(row(page, "mom", "expenses")).toContainText("$1,337.20");
    await expect(row(page, "mom", "cat:Housing")).toContainText("$1,200.00");
  });

  test("year to date sums January through September", async ({ page }) => {
    await openCompare(page);
    await expect(page.getByText("Jan 1, 2026 through September 2026 (2 month(s) with data).")).toBeVisible();
    await expect(row(page, "ytd", "income")).toContainText("$8,000.00");
    await expect(row(page, "ytd", "income")).toContainText("$4,050.00");
    await expect(row(page, "ytd", "expenses")).toContainText("$1,474.40");
  });

  test("year over year compares with September 2025", async ({ page }) => {
    await openCompare(page);
    await expect(row(page, "yoy", "income")).toContainText("$3,000.00");
    await expect(row(page, "yoy", "cat:Food")).toContainText("$450.00");
    await expect(row(page, "yoy", "cat:Other")).toContainText("$1,500.00");
    await expect(row(page, "yoy", "cat:Giving")).toHaveCount(0);
  });
});

test("reports for a future month show plan and zero actuals", async ({ page, seed, login }) => {
  const s = await seed("basic", { today: "2026-01-15" });
  await login();
  await pz.reports(page, s);
  await expect(page.locator('[data-card="income"]')).toContainText("Received $0.00");
  await openCompare(page);
  await expect(page.getByText("No budget exists for August 2026")).toBeVisible();
  await expect(page.getByText("No budget exists for September 2025")).toBeVisible();
});
