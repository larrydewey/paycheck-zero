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
    await expect(form.getByLabel("Linked budget line")).toHaveValue("line:Groceries");
    await form.getByLabel("Goal name").fill("Pantry stock");
    await form.getByLabel("Target amount").fill("1000");
    await form.getByRole("button", { name: "Create goal" }).click();
    await expect(pz.toast(page)).toContainText("Goal created.");
    await expect(pz.line(page, "Groceries").locator("[data-line-goal]")).toContainText("Pantry stock");
  });

  test("amounts save when you tab or tap away, not only on Enter", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    // Budget screen: a line's total.
    await pz.overview(page, s);
    const rent = page.getByLabel("Total planned for Rent");
    await rent.fill("1250");
    await rent.press("Tab");
    await expect(pz.category(page, "Housing").locator(":scope > summary [data-col=planned]")).toContainText("$1,400.00");
    await page.reload();
    await expect(page.getByLabel("Total planned for Rent")).toHaveValue("$1,250.00");
    // A paycheck's plan.
    await pz.paycheck(page, s, 0);
    const gas = page.getByLabel("Planned for Gas from this paycheck");
    await gas.fill("75");
    await gas.press("Tab");
    await expect(pz.line(page, "Gas").locator("[data-col=planned]")).toContainText("$75.00");
    await page.reload();
    await expect(page.getByLabel("Planned for Gas from this paycheck")).toHaveValue("$75.00");
    // A line's funding in its sheet.
    await pz.overview(page, s);
    await pz.lineSheet(page, "Groceries");
    const sep18 = pz.sheet(page).getByLabel("Groceries from the Sep 18 paycheck");
    await sep18.fill("250");
    await sep18.press("Tab");
    await expect(pz.sheet(page).locator("[data-col=planned]")).toHaveText("$550.00");
    await page.reload();
    await expect(pz.line(page, "Groceries").locator(".row-amount input[name=amount]")).toHaveValue("$550.00");
  });

  test("the goal form lists budget lines to link, and Settings shows the version", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.overview(page, s);
    await pz.openSheet(page, "New goal");
    const pick = pz.sheet(page).getByLabel("Linked budget line");
    await expect(pick.locator("optgroup[label=Food] option")).toHaveText(["Groceries"]);
    await pick.selectOption({ label: "Groceries" });
    await expect(pick).toHaveValue("line:Groceries");
    await pz.closeSheet(page);
    await page.goto("/settings");
    await expect(page.locator("[data-app-version]")).toContainText(/PaycheckZero \d+\.\d+\.\d+/);
  });
});
