import { test, expect, pz } from "./fixtures";

const row = (page: import("@playwright/test").Page, table: string, key: string) => page.locator(`#${table} tr[data-row="${key}"]`);

test.describe("reports (seeded history)", () => {
  test.beforeEach(async ({ page, seed, login }) => {
    const s = await seed("history");
    await login();
    await pz.reports(page, s);
  });

  test("quick-win summary cards", async ({ page }) => {
    await expect(page.locator('[data-card="income"]')).toContainText("$4,000.00");
    await expect(page.locator('[data-card="income"]')).toContainText("Actual $0.00");
    await expect(page.locator('[data-card="expenses"]')).toContainText("Spent $137.20");
    await expect(page.locator('[data-card="remaining"]')).toContainText("$0.00");
    await expect(page.locator('[data-card="variance"]')).toContainText("Income -$4,000.00");
    await expect(page.locator('[data-card="variance"]')).toContainText("Spending -$3,862.80");
    await expect(page.locator('[data-card="trend"]')).toContainText("Under plan");
  });

  test("month over month compares with August", async ({ page }) => {
    await expect(row(page, "mom", "income")).toContainText("$4,000.00$0.00$4,000.00$4,050.00".replace(/\$/g, "$"));
    await expect(row(page, "mom", "expenses")).toContainText("$137.20");
    await expect(row(page, "mom", "expenses")).toContainText("$1,337.20");
    await expect(row(page, "mom", "cat:Housing")).toContainText("$1,200.00");
  });

  test("year to date sums January through September", async ({ page }) => {
    await expect(page.getByText("Jan 1, 2026 through September 2026 (2 month(s) with data).")).toBeVisible();
    await expect(row(page, "ytd", "income")).toContainText("$8,000.00");
    await expect(row(page, "ytd", "income")).toContainText("$4,050.00");
    await expect(row(page, "ytd", "expenses")).toContainText("$1,474.40");
  });

  test("year over year compares with September 2025", async ({ page }) => {
    await expect(row(page, "yoy", "income")).toContainText("$3,000.00");
    await expect(row(page, "yoy", "cat:Food")).toContainText("$450.00");
    await expect(row(page, "yoy", "cat:Other")).toContainText("$1,500.00");
  });
});

test("reports for a future month show plan and zero actuals", async ({ page, seed, login }) => {
  const s = await seed("basic", { today: "2026-01-15" });
  await login();
  await pz.reports(page, s);
  await expect(page.locator('[data-card="income"]')).toContainText("Actual $0.00");
  await expect(page.getByText("No budget exists for August 2026")).toBeVisible();
  await expect(page.getByText("No budget exists for September 2025")).toBeVisible();
});
