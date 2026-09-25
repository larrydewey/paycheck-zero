import { test, expect, pz } from "./fixtures";

test.describe("transactions", () => {
  test("empty state", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s, "2026-09");
    await expect(page.locator("#tx-list li.tx")).toHaveCount(3);
  });

  test("an expense linked to a line and tagged to a paycheck updates Spent and Safe to Spend", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    const form = page.locator("#add-tx");
    await form.getByLabel("Date").fill("2026-09-11");
    await form.getByLabel("Amount", { exact: true }).fill("20.80");
    await form.getByLabel("Payee").fill("Farmers market");
    await form.getByLabel("Expense line", { exact: true }).selectOption({ label: "Groceries" });
    await form.getByLabel("Paycheck", { exact: true }).selectOption({ label: "Sep 4 · Acme Payroll" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    await expect(page.locator("#tx-list")).toContainText("Farmers market");
    await expect(page.locator("#tx-list li", { hasText: "Farmers market" })).toContainText("Groceries · Sep 4 paycheck");
    await pz.paycheck(page, s, 0);
    await expect(pz.sts(page)).toHaveText("$388.00");
    await expect(pz.line(page, "Groceries").locator("[data-col=spent]")).toContainText("$106.00");
  });

  test("a line-only transaction does not touch Safe to Spend", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    const form = page.locator("#add-tx");
    await form.getByLabel("Amount", { exact: true }).fill("10");
    await form.getByLabel("Expense line", { exact: true }).selectOption({ label: "Gas" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    await pz.paycheck(page, s, 0);
    await expect(pz.sts(page)).toHaveText("$388.00");
    await expect(pz.line(page, "Gas").locator("[data-col=spent]")).toContainText("$50.00");
  });

  test("edit and delete a transaction", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    pz.acceptDialogs(page);
    await pz.transactions(page, s);
    await page.getByRole("button", { name: "Edit Shell on Sep 6" }).click();
    const dialog = page.getByRole("dialog", { name: "Edit transaction" });
    await dialog.getByLabel("Amount", { exact: true }).fill("55");
    await dialog.getByRole("radio", { name: "Income" }).check();
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#tx-list li", { hasText: "Shell" })).toContainText("$55.00");
    await expect(page.locator("#tx-list li", { hasText: "Shell" }).locator(".tx-amt")).toHaveClass(/pos/);
    await page.getByRole("button", { name: "Edit Coffee on Sep 7" }).click();
    await page.getByRole("dialog", { name: "Edit transaction" }).getByRole("button", { name: "Delete transaction" }).click();
    await expect(page.locator("#tx-list li", { hasText: "Coffee" })).toHaveCount(0);
  });

  test("income tagged to a paycheck reconciles what was actually received", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    const form = page.locator("#add-tx");
    await form.getByRole("radio", { name: "Income" }).check();
    await form.getByLabel("Amount", { exact: true }).fill("1,950");
    await form.getByLabel("Payee").fill("Acme deposit");
    await form.getByLabel("Paycheck", { exact: true }).selectOption({ label: "Sep 4 · Acme Payroll" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    await pz.income(page, s);
    const row = page.locator("section.income-line tbody tr", { hasText: "Sep 4" });
    await expect(row).toContainText("$1,950.00");
    await expect(row).toContainText("Received");
    await pz.paycheck(page, s, 0);
    await expect(page.locator("#variance-panel")).toContainText("Sep 4: planned $2,000.00, received $1,950.00.");
  });

  test("transactions still work in a locked month", async ({ page, seed, login }) => {
    const s = await seed("locked");
    await login();
    await pz.transactions(page, s);
    await expect(page.getByText("This month is locked. You can still record transactions.")).toBeVisible();
    const form = page.locator("#add-tx");
    await form.getByLabel("Amount", { exact: true }).fill("5");
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
