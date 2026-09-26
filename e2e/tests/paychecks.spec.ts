import { test, expect, pz, waitForContent } from "./fixtures";

test.describe("paycheck view (primary)", () => {
  test.beforeEach(async ({ seed, login }) => {
    await seed("basic");
    await login();
  });

  test("shows Safe to Spend prominently with what the paycheck funds", async ({ page }) => {
    await expect(pz.sts(page)).toHaveText("$388.00");
    await expect(page.locator('[data-stat="unassigned"]')).toHaveText("$400.00");
    await expect(page.locator('[data-stat="tagged"]')).toHaveText("$97.20");
    await expect(page.locator('[data-stat="budget-left"]')).toHaveText("$1,514.80");
    await expect(page.locator('[data-stat="rolling"]')).toHaveText("$1,475.00");
    await expect(page.locator("#zero-status")).toContainText("$1,875.00 left to assign this month");
    await expect(pz.line(page, "Rent")).toBeVisible();
    // Every line shows, including ones this paycheck doesn't fund.
    await expect(page.getByLabel("Planned for Electric from this paycheck")).toHaveValue("0.00");
    // Categories without lines are one tap from their first line.
    await expect(page.locator('[data-category-empty="Giving"]')).toBeVisible();
    const food = pz.category(page, "Food");
    await expect(food.locator("summary [data-col=this]")).toContainText("$300.00");
    await expect(food.locator("summary [data-col=planned]")).toContainText("$600.00");
    await expect(pz.line(page, "Groceries").locator("[data-col=remaining]")).toContainText("$514.80");
    await pz.lineSheet(page, "Groceries");
    await expect(pz.sheet(page).getByLabel("Groceries from the Sep 4 paycheck")).toHaveValue("300.00");
    await expect(pz.sheet(page).getByLabel("Groceries from the Sep 18 paycheck")).toHaveValue("300.00");
  });

  test("inline Planned edit updates Safe to Spend and category totals live", async ({ page }) => {
    const input = page.getByLabel("Planned for Rent from this paycheck");
    await input.fill("1,000.50");
    await input.press("Enter");
    await expect(pz.sts(page)).toHaveText("$587.50");
    await expect(pz.category(page, "Housing").locator("summary [data-col=this]")).toContainText("$1,000.50");
    await expect(page.locator("#zero-status")).toContainText("$2,074.50 left");
    await expect(input).toHaveValue("1000.50");
  });

  test("setting a line to 0 removes the allocation", async ({ page }) => {
    const input = page.getByLabel("Planned for Gas from this paycheck");
    await input.fill("0");
    await input.press("Enter");
    await expect(pz.line(page, "Gas")).toHaveClass(/unfunded/);
    await expect(input).toHaveValue("0.00");
    await expect(pz.sts(page)).toHaveText("$488.00");
  });

  test("over-allocation is blocked in the browser before sending", async ({ page }) => {
    const input = page.getByLabel("Planned for Rent from this paycheck");
    await input.fill("1612.50");
    await input.press("Enter");
    await expect(pz.line(page, "Rent").locator(".field-error")).toHaveText("That's $12.50 more than this paycheck has left.");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await expect(pz.sts(page)).toHaveText("$388.00");
  });

  test("over-allocation is also rejected by the server with a human message", async ({ page }) => {
    // Remove the client guard to prove the server enforces the invariant.
    await page.evaluate(() => document.querySelectorAll("[data-max-cents]").forEach((e) => e.removeAttribute("data-max-cents")));
    const input = page.getByLabel("Planned for Rent from this paycheck");
    await input.fill("1612.50");
    await input.press("Enter");
    const toast = pz.toast(page);
    await expect(toast).toContainText("That would over-allocate the Sep 4 paycheck by $12.50.");
    await toast.getByText("Technical details").click();
    await expect(toast.locator("code")).toContainText("INVARIANT_VIOLATION");
    await expect(input).toHaveValue("1200.00");
    await expect(pz.sts(page)).toHaveText("$388.00");
  });

  test("create a new line on the fly and fund it from this paycheck", async ({ page }) => {
    await pz.openSheet(page, "Assign $400.00");
    const sh = pz.sheet(page);
    await sh.getByLabel("Expense line").selectOption("__new");
    await sh.getByLabel("New line name").fill("Phone");
    await sh.getByLabel("Category", { exact: true }).selectOption({ label: "Personal" });
    await sh.getByLabel("Amount", { exact: true }).fill("45");
    await sh.getByRole("button", { name: "Assign", exact: true }).click();
    await expect(sh).toBeHidden();
    await expect(pz.toast(page)).toContainText("Assigned to Phone.");
    await expect(pz.line(page, "Phone")).toBeVisible();
    await expect(pz.sts(page)).toHaveText("$343.00");
  });

  test("assign blocks amounts above what's left", async ({ page }) => {
    await pz.openSheet(page, "Assign $400.00");
    const sh = pz.sheet(page);
    await sh.getByLabel("Expense line").selectOption({ label: "Electric" });
    await sh.getByLabel("Amount", { exact: true }).fill("400.01");
    await sh.getByRole("button", { name: "Assign", exact: true }).click();
    await expect(page.locator("#fund-form .field-error")).toHaveText("That's $0.01 more than this paycheck has left.");
  });

  test("fully assigning a paycheck clears the nudge and marks the chip", async ({ page }) => {
    await pz.openSheet(page, "Assign $400.00");
    const sh = pz.sheet(page);
    await sh.getByLabel("Expense line").selectOption({ label: "Emergency Fund" });
    await sh.getByLabel("Amount", { exact: true }).fill("400");
    await sh.getByRole("button", { name: "Assign", exact: true }).click();
    await expect(page.getByRole("button", { name: /^Assign \$/ })).toHaveCount(0);
    await expect(page.locator("#unassigned-nudge")).toContainText("This paycheck is fully assigned.");
    await expect(page.locator(".chip.active")).toContainText("Fully assigned");
  });

  test("split one line across many paychecks", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.paycheck(page, s, 1);
    const input = page.getByLabel("Planned for Groceries from this paycheck");
    await input.fill("450");
    await input.press("Enter");
    await expect(pz.category(page, "Food").locator("summary [data-col=planned]")).toContainText("$750.00");
    await pz.overview(page, s);
    await expect(pz.line(page, "Groceries").locator("[data-col=planned]")).toHaveText("$750.00");
    await pz.lineSheet(page, "Groceries");
    await expect(pz.sheet(page).getByLabel("Groceries from the Sep 4 paycheck")).toHaveValue("300.00");
    await expect(pz.sheet(page).getByLabel("Groceries from the Sep 18 paycheck")).toHaveValue("450.00");
  });

  test("rename a line from its sheet", async ({ page }) => {
    await pz.lineSheet(page, "Gas");
    await pz.sheet(page).getByLabel("Line name").fill("Fuel");
    await pz.sheet(page).getByRole("button", { name: "Save" }).last().click();
    await expect(pz.sheet(page)).toBeHidden();
    await expect(pz.line(page, "Fuel")).toBeVisible();
  });

  test("reducing a paycheck below its allocations cascades with a notice", async ({ page }) => {
    await pz.openSheet(page, "Paycheck details");
    await pz.sheet(page).getByLabel("Planned amount").fill("1500");
    await pz.sheet(page).getByRole("button", { name: "Save" }).click();
    await expect(pz.toast(page)).toContainText("Funding was reduced for: Gas.");
    await expect(pz.toast(page)).toContainText("Re-balance the $1,475.00 difference");
    await expect(page.locator('[data-stat="assigned"]')).toHaveText("$1,500.00");
    await expect(page.getByLabel("Planned for Gas from this paycheck")).toHaveValue("0.00");
  });

  test("skipping a paycheck deletes its allocations", async ({ page }) => {
    pz.acceptDialogs(page);
    await pz.openSheet(page, "Paycheck details");
    await pz.sheet(page).getByRole("button", { name: "Skip this paycheck" }).click();
    await expect(page.getByText("This paycheck is skipped.")).toBeVisible();
    await expect(pz.sts(page)).toHaveText("$0.00");
    await expect(page.locator("#zero-status")).toContainText("$1,475.00 left to assign");
    await expect(page.locator(".chip.active")).toContainText("Skipped");
  });

  test("deleting a funded paycheck removes its allocations and moves on", async ({ page }) => {
    pz.acceptDialogs(page);
    await pz.openSheet(page, "Paycheck details");
    await pz.sheet(page).getByRole("button", { name: "Delete paycheck" }).click();
    await expect(page.getByRole("heading", { name: "Paycheck Sep 18 — Acme Payroll" })).toBeAttached();
    await waitForContent(page);
    await expect(page.locator(".paycheck-strip .chip:not(.add)")).toHaveCount(1);
    await expect(page.locator("#toasts")).toContainText("Removed paycheck(s): Sep 4.");
  });

  test("destructive actions ask first and can be cancelled", async ({ page }) => {
    await pz.openSheet(page, "Paycheck details");
    await pz.sheet(page).getByRole("button", { name: "Delete paycheck" }).click();
    const dialog = page.locator("#pz-confirm");
    await expect(dialog).toContainText("Delete this paycheck? All money it funds will be un-assigned.");
    await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
    await expect(page.locator(".paycheck-strip .chip:not(.add)")).toHaveCount(2);
  });

  test("the assign form is replaced by guidance once a paycheck is fully assigned", async ({ page }) => {
    await pz.openSheet(page, "Assign $400.00");
    const sh = pz.sheet(page);
    await sh.getByRole("button", { name: "Use all $400.00" }).click();
    await expect(sh.getByLabel("Amount", { exact: true })).toHaveValue("400.00");
    await sh.getByLabel("Expense line").selectOption({ label: "Emergency Fund" });
    await sh.getByRole("button", { name: "Assign", exact: true }).click();
    await expect(sh).toBeHidden();
    await expect(page.getByRole("button", { name: /^Assign \$/ })).toHaveCount(0);
    await expect(page.locator("#unassigned-nudge")).toContainText("To fund something else, lower another line first.");
  });

  test("one click gives 10% of the paycheck to Tithe", async ({ page }) => {
    await pz.openSheet(page, "Assign $400.00");
    await pz.sheet(page).getByRole("button", { name: "Give 10% ($200.00)" }).click();
    await expect(pz.toast(page)).toContainText("$200.00 assigned to Tithe.");
    await expect(pz.line(page, "Tithe")).toBeVisible();
    await expect(pz.category(page, "Giving").locator("summary [data-col=this]")).toContainText("$200.00");
    await pz.openSheet(page, "Assign $200.00");
    await expect(pz.sheet(page).getByText("10% given ($200.00)")).toBeVisible();
    await pz.closeSheet(page);
    await expect(pz.sts(page)).toHaveText("$188.00");
  });

  test("add a line (and category) right from the paycheck view", async ({ page }) => {
    const input = page.getByLabel("New line in Food", { exact: true });
    await input.fill("Dining out");
    await page.getByLabel("Amount from this paycheck for the new line in Food").fill("35");
    await pz.category(page, "Food").getByRole("button", { name: "Add line" }).click();
    await expect(page.getByLabel("Planned for Dining out from this paycheck")).toHaveValue("35.00");
    await expect(pz.sts(page)).toHaveText("$353.00");
    await expect(input).toHaveValue("");
    // A brand-new category and line, funded in one go.
    await pz.openSheet(page, "New line", { exact: true });
    const sh = pz.sheet(page);
    await sh.getByLabel("Line name").fill("Allowance");
    await sh.getByLabel("Category", { exact: true }).selectOption("__new");
    await sh.getByLabel("Category name").fill("Kids");
    await sh.getByLabel("Amount from this paycheck").fill("20");
    await sh.getByRole("button", { name: "Add line" }).click();
    await expect(sh).toBeHidden();
    await expect(pz.category(page, "Kids")).toBeVisible();
    await expect(page.getByLabel("Planned for Allowance from this paycheck")).toHaveValue("20.00");
    await expect(pz.sts(page)).toHaveText("$333.00");
  });

  test("filter to only the lines this paycheck funds (remembered)", async ({ page }) => {
    const toggle = page.getByRole("button", { name: "Only lines this paycheck funds" });
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "true");
    await expect(pz.line(page, "Electric")).toBeHidden();
    await expect(pz.category(page, "Debt")).toBeHidden();
    await expect(pz.line(page, "Rent")).toBeVisible();
    await page.reload();
    await expect(pz.line(page, "Electric")).toBeHidden();
    await page.getByRole("button", { name: "Only lines this paycheck funds" }).click();
    await expect(pz.line(page, "Electric")).toBeVisible();
  });

  test("navigating between paychecks", async ({ page }) => {
    await page.locator(".paycheck-strip .chip", { hasText: "Sep 18" }).click();
    await waitForContent(page);
    await expect(pz.sts(page)).toHaveText("$1,475.00");
    await expect(pz.line(page, "Visa")).toBeVisible();
  });
});

test.describe("paycheck view: locked and targets", () => {
  test("a locked month shows read-only planning", async ({ page, seed, login }) => {
    await seed("locked");
    await login();
    await expect(page.getByText("Locked — planning is frozen")).toBeVisible();
    await expect(page.getByLabel("Planned for Rent from this paycheck")).toHaveCount(0);
    await expect(page.getByRole("button", { name: /^Assign/ })).toHaveCount(0);
  });

  test("copied targets can be funded in one click", async ({ page, seed, login }) => {
    const s = await seed("balanced");
    await login();
    await page.goto("/months?new=2026-10");
    await waitForContent(page);
    await page.getByLabel("Copy structure + planned amounts").check();
    await page.getByRole("button", { name: "Create month" }).click();
    await expect(page).toHaveURL(/\/income$/);
    await waitForContent(page);
    await page.getByLabel("Name").fill("Acme Payroll");
    await page.getByLabel("Take-home per paycheck").fill("2000");
    await page.getByLabel("A recent or upcoming payday").fill("2026-10-02");
    await page.getByRole("button", { name: "Add income" }).click();
    await expect(page).toHaveURL(/\/paychecks\//);
    await waitForContent(page);
    await pz.openSheet(page, "Assign $2,000.00");
    const targets = pz.sheet(page).locator("section.targets");
    await expect(targets).toContainText("Rent target $1,200.00 · $1,200.00 unfunded");
    await targets.locator("li", { hasText: "Rent" }).getByRole("button").click();
    await expect(pz.line(page, "Rent")).toBeVisible();
    await expect(pz.sts(page)).toHaveText("$800.00");
    expect(s.months["2026-09"].id).toBeTruthy();
  });
});
