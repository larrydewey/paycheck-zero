import { test, expect, pz } from "./fixtures";

test.describe("line sheet", () => {
  test("funding edits keep the sheet open, and money moves between paychecks in one step", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.overview(page, s);
    await pz.lineSheet(page, "Groceries");
    const sh = pz.sheet(page);
    const sep4 = sh.getByLabel("Groceries from the Sep 4 paycheck");
    const sep18 = sh.getByLabel("Groceries from the Sep 18 paycheck");
    await sep4.fill("200");
    await sep4.press("Enter");
    await expect(page.locator("#sheet[open]")).toBeVisible();
    await expect(sh.locator(".funding-list li", { hasText: "Sep 4" })).toContainText("$500.00 left");
    await sh.getByText("Move between paychecks").click();
    await sh.getByLabel("From", { exact: true }).selectOption({ index: 0 });
    await sh.getByLabel("Amount to move").fill("50");
    await sh.getByRole("button", { name: "Move", exact: true }).click();
    await expect(pz.toast(page)).toContainText("Moved $50.00 from the Sep 4 paycheck to the Sep 18 paycheck.");
    await expect(sep4).toHaveValue("$150.00");
    await expect(sep18).toHaveValue("$350.00");
    await expect(pz.line(page, "Groceries").locator(".row-amount input[name=amount]")).toHaveValue("$500.00");
    // More than is there is refused with a clear message.
    await sh.getByText("Move between paychecks").click();
    await sh.getByLabel("Amount to move").fill("5000");
    await sh.getByRole("button", { name: "Move", exact: true }).click();
    await expect(page.locator("#toasts")).toContainText("Only $150.00 is assigned there to move.");
  });

  test("goals show on the budget and can be linked from a line", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    await pz.overview(page, s);
    await expect(pz.line(page, "Emergency Fund").locator("[data-line-goal]")).toContainText("Emergency fund");
    await expect(pz.line(page, "Visa").locator("[data-line-goal]")).toHaveCount(0);
    await pz.lineSheet(page, "Visa");
    const sh = pz.sheet(page);
    await sh.getByLabel("Existing goal").selectOption({ label: "Visa paid off" });
    await sh.getByRole("button", { name: "Link", exact: true }).click();
    await expect(pz.toast(page)).toContainText("The goal now follows the Visa line.");
    await expect(pz.line(page, "Visa").locator("[data-line-goal]")).toContainText("Visa paid off");
  });

  test("a new goal started from a line follows it", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.overview(page, s);
    await pz.lineSheet(page, "Groceries");
    await pz.sheet(page).getByRole("button", { name: "New goal for this line" }).click();
    const form = page.locator("#sheet #goal-form");
    await expect(form).toBeVisible();
    await expect(form.getByLabel("Counts")).toHaveValue("line:Groceries");
    await form.getByLabel("Goal name").fill("Pantry stock");
    await form.getByLabel("Target amount").fill("1000");
    await form.getByRole("button", { name: "Create goal" }).click();
    await expect(pz.toast(page)).toContainText("Goal created.");
    await expect(pz.line(page, "Groceries").locator("[data-line-goal]")).toContainText("Pantry stock");
  });
});
