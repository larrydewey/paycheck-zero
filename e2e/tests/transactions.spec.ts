import { test, expect, pz } from "./fixtures";

test.describe("transactions", () => {
  test("empty state", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s, "2026-09");
    await expect(page.locator("#tx-table tbody tr")).toHaveCount(3);
  });

  test("an expense linked to a line and tagged to a paycheck updates Spent and Safe to Spend", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    const form = page.locator("#add-tx");
    await form.getByLabel("Date").fill("2026-09-11");
    await form.getByLabel("Amount").fill("20.80");
    await form.getByLabel("Payee").fill("Farmers market");
    await form.getByLabel("Expense line").selectOption({ label: "Groceries" });
    await form.getByLabel("Paycheck").selectOption({ label: "Sep 4 · Acme Payroll" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    await expect(page.locator("#tx-table")).toContainText("Farmers market");
    await pz.paycheck(page, s, 0);
    await expect(pz.sts(page)).toHaveText("$282.00");
    await expect(pz.line(page, "Groceries").locator("[data-col=spent]")).toContainText("$106.00");
  });

  test("a line-only transaction does not touch Safe to Spend", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    const form = page.locator("#add-tx");
    await form.getByLabel("Amount").fill("10");
    await form.getByLabel("Expense line").selectOption({ label: "Gas" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    await pz.paycheck(page, s, 0);
    await expect(pz.sts(page)).toHaveText("$302.80");
    await expect(pz.line(page, "Gas").locator("[data-col=spent]")).toContainText("$50.00");
  });

  test("edit and delete a transaction", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    pz.acceptDialogs(page);
    await pz.transactions(page, s);
    const row = page.locator("#tx-table tr", { hasText: "Shell" });
    await row.getByText("Edit").click();
    await row.getByLabel("Amount").fill("55");
    await row.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#tx-table tr", { hasText: "Shell" })).toContainText("-$55.00");
    const row2 = page.locator("#tx-table tr", { hasText: "Coffee" });
    await row2.getByText("Edit").click();
    await row2.getByRole("button", { name: "Delete" }).click();
    await expect(page.locator("#tx-table tr", { hasText: "Coffee" })).toHaveCount(0);
  });

  test("transactions still work in a locked month", async ({ page, seed, login }) => {
    const s = await seed("locked");
    await login();
    await pz.transactions(page, s);
    await expect(page.getByText("This month is locked. You can still record transactions.")).toBeVisible();
    const form = page.locator("#add-tx");
    await form.getByLabel("Amount").fill("5");
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
  });

  test("empty transactions list shows guidance", async ({ page, seed, login }) => {
    await seed("user");
    await login();
    await page.getByRole("button", { name: "New month" }).first().click();
    await page.getByLabel("Month", { exact: true }).fill("2026-11");
    await page.getByRole("button", { name: "Create month" }).click();
    await expect(page).toHaveURL(/\/income$/);
    await page.getByRole("link", { name: "Transactions" }).click();
    await expect(page.getByRole("heading", { name: "No transactions yet" })).toBeVisible();
  });
});
