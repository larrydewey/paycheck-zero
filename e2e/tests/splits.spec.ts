import { test, expect, pz } from "./fixtures";

test.describe("split transactions", () => {
  test("split one payment across lines and paychecks, then edit and delete it", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    pz.acceptDialogs(page);
    await pz.transactions(page, s);
    await page.getByText("Split a transaction").click();
    const form = page.locator("#new-split-form");
    await form.getByLabel("Total").fill("120");
    await form.getByLabel("Payee").fill("Target");
    await form.getByLabel("Part 1 line").selectOption({ label: "Groceries" });
    await form.getByLabel("Part 1 paycheck").selectOption({ label: "Sep 4 · Acme Payroll" });
    await form.getByLabel("Part 1 amount").fill("80");
    await form.getByLabel("Part 2 line").selectOption({ label: "Electric" });
    await form.getByLabel("Part 2 paycheck").selectOption({ label: "Sep 18 · Acme Payroll" });
    await form.getByLabel("Part 2 amount").fill("30");
    await expect(form.locator("[data-split-left]")).toContainText("$10.00");
    await form.getByRole("button", { name: "Save split" }).click();
    await expect(form.locator(".field-error")).toHaveText("The parts don't add up to the total yet.");
    await form.getByRole("button", { name: "+ Add part" }).click();
    await form.getByLabel("Part 3 line").selectOption({ label: "Gas" });
    await form.getByLabel("Part 3 amount").fill("10");
    await expect(form.locator("[data-split-left]")).toContainText("$0.00");
    await form.getByRole("button", { name: "Save split" }).click();
    await expect(pz.toast(page)).toContainText("Split saved.");
    const row = page.locator("#tx-list li.split", { hasText: "Target" });
    await expect(row).toContainText("-$120.00");
    await expect(row).toContainText("Split · Groceries $80.00 (Sep 4) · Electric $30.00 (Sep 18) · Gas $10.00");
    // Spent and Safe to Spend follow each part.
    await pz.paycheck(page, s, 0);
    await expect(pz.line(page, "Groceries").locator("[data-col=spent]")).toContainText("$165.20");
    await expect(pz.sts(page)).toHaveText("$388.00"); // Groceries spending is covered by its allocation
    // Edit: drop a part.
    await pz.transactions(page, s);
    await page.getByRole("button", { name: "Edit Target on Sep 10" }).click();
    const dlg = page.getByRole("dialog", { name: "Edit split transaction" });
    await dlg.getByLabel("Total").fill("110");
    await dlg.getByRole("button", { name: "Remove part 3" }).click();
    await dlg.getByRole("button", { name: "Save split" }).click();
    await expect(page.locator("#tx-list li.split", { hasText: "Target" })).toContainText("-$110.00");
    await page.getByRole("button", { name: "Edit Target on Sep 10" }).click();
    await page.getByRole("dialog", { name: "Edit split transaction" }).getByRole("button", { name: "Delete split" }).click();
    await expect(page.locator("#tx-list li.split")).toHaveCount(0);
  });
});
