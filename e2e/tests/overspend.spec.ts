import { test, expect, pz } from "./fixtures";

test.describe("overspending warnings", () => {
  test("a transaction that overspends a line warns everywhere", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await expect(page.locator("#overspent-banner")).toHaveCount(0);
    await pz.transactions(page, s);
    const form = page.locator("#add-tx");
    await form.getByLabel("Amount", { exact: true }).fill("112.30");
    await form.getByLabel("Payee").fill("Gas station");
    await form.getByLabel("Expense line", { exact: true }).selectOption({ label: "Gas" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(page.locator("#toasts")).toContainText("Gas is now $52.30 over plan.");
    await expect(page.locator("#overspent-banner")).toContainText("1 line(s) over plan: Gas ($52.30 over)");
    await page.locator("#overspent-banner").getByRole("link", { name: "Review in Month overview" }).click();
    await expect(pz.line(page, "Gas")).toHaveClass(/over/);
    await expect(pz.line(page, "Gas")).toContainText("Over by $52.30");
    await expect(pz.category(page, "Transportation").locator(".cat-over")).toBeVisible();
    await pz.paycheck(page, s, 0);
    await expect(pz.line(page, "Gas")).toContainText("Over by $52.30");
  });

  test("negative Safe to Spend is called out", async ({ page, seed, login }) => {
    await seed("balanced");
    await login();
    await expect(pz.sts(page)).toHaveText("-$12.00");
    await expect(page.locator(".hero-warn")).toContainText("$12.00 of spending from this paycheck wasn't in its plan.");
  });
});
