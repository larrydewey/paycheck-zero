import { test, expect, pz } from "./fixtures";

test.describe("split transactions (one form for plain and split)", () => {
  test("split a transaction from the same fields, edit it, and merge it back", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    pz.acceptDialogs(page);
    await pz.transactions(page, s);
    const form = page.locator("#add-tx");
    // Starts as a normal transaction…
    await form.getByLabel("Amount", { exact: true }).fill("120");
    await form.getByLabel("Payee").fill("Target");
    await form.getByLabel("Expense line", { exact: true }).selectOption({ label: "Groceries" });
    await form.getByLabel("Paycheck", { exact: true }).selectOption({ label: "Sep 4 · Acme Payroll" });
    // …and the same fields become part 1 of a split.
    await form.getByRole("button", { name: "Split into parts" }).click();
    await expect(form.getByLabel("Part 1 line")).toHaveValue(s.months["2026-09"].lines["Groceries"]);
    await expect(form.getByLabel("Part 1 amount")).toHaveValue("120");
    await form.getByLabel("Part 1 amount").fill("80");
    await form.getByLabel("Part 2 line").selectOption({ label: "Electric" });
    await form.getByLabel("Part 2 paycheck").selectOption({ label: "Sep 18 · Acme Payroll" });
    await form.getByLabel("Part 2 amount").fill("30");
    await expect(form.locator("[data-split-left]")).toContainText("$10.00");
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(form.locator(".tx-parts .field-error")).toHaveText("The parts don't add up to the total yet.");
    await form.getByRole("button", { name: "+ Add part" }).click();
    await form.getByLabel("Part 3 line").selectOption({ label: "Gas" });
    await form.getByLabel("Part 3 amount").fill("10");
    await expect(form.locator("[data-split-left]")).toContainText("$0.00");
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Split saved.");
    // The add form is back to a plain transaction.
    await expect(form.getByLabel("Expense line", { exact: true })).toBeVisible();
    await expect(form.getByLabel("Amount", { exact: true })).toHaveValue("");
    const row = page.locator("#tx-list li.split", { hasText: "Target" });
    await expect(row).toContainText("-$120.00");
    await expect(row).toContainText("Split · Groceries $80.00 (Sep 4) · Electric $30.00 (Sep 18) · Gas $10.00");
    await pz.paycheck(page, s, 0);
    await expect(pz.line(page, "Groceries").locator("[data-col=spent]")).toContainText("$165.20");
    // Edit: drop a part.
    await pz.transactions(page, s);
    await page.getByRole("button", { name: "Edit Target on Sep 10" }).click();
    let dlg = page.getByRole("dialog", { name: "Edit split transaction" });
    await dlg.getByLabel("Amount", { exact: true }).fill("110");
    await dlg.getByRole("button", { name: "Remove part 3" }).click();
    await dlg.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#tx-list li.split", { hasText: "Target" })).toContainText("-$110.00");
    // Merge back into a plain transaction by removing parts down to one.
    await page.getByRole("button", { name: "Edit Target on Sep 10" }).click();
    dlg = page.getByRole("dialog", { name: "Edit split transaction" });
    await dlg.getByRole("button", { name: "Remove part 2" }).click();
    await expect(dlg.getByLabel("Expense line", { exact: true })).toBeVisible();
    await dlg.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#tx-list li.split")).toHaveCount(0);
    await expect(page.locator("#tx-list li", { hasText: "Target" })).toContainText("Groceries");
    await expect(page.locator("#tx-list li", { hasText: "Target" })).toContainText("-$110.00");
  });

  test("a plain transaction can be turned into a split when editing", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    await page.getByRole("button", { name: "Edit Shell on Sep 6" }).click();
    const dlg = page.getByRole("dialog", { name: "Edit transaction" });
    await dlg.getByRole("button", { name: "Split into parts" }).click();
    await dlg.getByLabel("Part 1 amount").fill("25");
    await dlg.getByLabel("Part 2 line").selectOption({ label: "Groceries" });
    await dlg.getByLabel("Part 2 amount").fill("15");
    await dlg.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#tx-list li.split", { hasText: "Shell" })).toContainText("Split · Gas $25.00 · Groceries $15.00");
  });
});
