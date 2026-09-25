import { test, expect, pz, waitForContent } from "./fixtures";

test.describe("paycheck view (primary)", () => {
  test.beforeEach(async ({ seed, login }) => {
    await seed("basic");
    await login();
  });

  test("shows Safe to Spend prominently with what the paycheck funds", async ({ page }) => {
    await expect(pz.sts(page)).toHaveText("$302.80");
    await expect(page.locator('[data-stat="unassigned"]')).toHaveText("$400.00");
    await expect(page.locator('[data-stat="tagged"]')).toHaveText("$97.20");
    await expect(page.locator('[data-stat="rolling"]')).toHaveText("$1,475.00");
    await expect(page.locator("#zero-status")).toContainText("$1,875.00 left to assign this month");
    await expect(pz.line(page, "Rent")).toBeVisible();
    await expect(pz.line(page, "Electric")).toHaveCount(0);
    const food = pz.category(page, "Food");
    await expect(food.locator("summary [data-col=this]")).toContainText("$300.00");
    await expect(food.locator("summary [data-col=planned]")).toContainText("$600.00");
    await expect(pz.line(page, "Groceries").locator("[data-col=remaining]")).toContainText("$514.80");
    await expect(pz.line(page, "Groceries")).toContainText("Split across 2 paychecks");
  });

  test("inline Planned edit updates Safe to Spend and category totals live", async ({ page }) => {
    const input = page.getByLabel("Planned for Rent from this paycheck");
    await input.fill("1,000.50");
    await input.press("Enter");
    await expect(pz.sts(page)).toHaveText("$502.30");
    await expect(pz.category(page, "Housing").locator("summary [data-col=planned]")).toContainText("$1,000.50");
    await expect(page.locator("#zero-status")).toContainText("$2,074.50 left");
    await expect(input).toHaveValue("1000.50");
  });

  test("setting a line to 0 removes the allocation", async ({ page }) => {
    const input = page.getByLabel("Planned for Gas from this paycheck");
    await input.fill("0");
    await input.press("Enter");
    await expect(pz.line(page, "Gas")).toHaveCount(0);
    await expect(pz.sts(page)).toHaveText("$402.80");
  });

  test("over-allocation is blocked in the browser before sending", async ({ page }) => {
    const input = page.getByLabel("Planned for Rent from this paycheck");
    await input.fill("1612.50");
    await input.press("Enter");
    await expect(pz.line(page, "Rent").locator(".field-error")).toHaveText("That's $12.50 more than this paycheck has left.");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await expect(pz.sts(page)).toHaveText("$302.80");
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
    await expect(pz.sts(page)).toHaveText("$302.80");
  });

  test("create a new line on the fly and fund it from this paycheck", async ({ page }) => {
    await page.getByLabel("Expense line").selectOption("__new");
    await page.getByLabel("New line name").fill("Phone");
    await page.getByLabel("Category", { exact: true }).selectOption({ label: "Personal" });
    await page.getByLabel("Amount", { exact: true }).fill("45");
    await page.getByRole("button", { name: "Assign" }).click();
    await expect(pz.toast(page)).toContainText("Assigned to Phone.");
    await expect(pz.line(page, "Phone")).toBeVisible();
    await expect(pz.sts(page)).toHaveText("$257.80");
  });

  test("assign blocks amounts above what's left", async ({ page }) => {
    await page.getByLabel("Expense line").selectOption({ label: "Electric" });
    await page.getByLabel("Amount", { exact: true }).fill("400.01");
    await page.getByRole("button", { name: "Assign" }).click();
    await expect(page.locator("#fund-form .field-error")).toHaveText("That's $0.01 more than this paycheck has left.");
  });

  test("fully assigning a paycheck clears the nudge and marks the chip", async ({ page }) => {
    await page.getByLabel("Expense line").selectOption({ label: "Emergency Fund" });
    await page.getByLabel("Amount", { exact: true }).fill("400");
    await page.getByRole("button", { name: "Assign" }).click();
    await expect(page.locator("#unassigned-nudge")).toHaveText("✓ This paycheck is fully assigned.");
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
    await expect(pz.line(page, "Groceries")).toContainText("Sep 4: $300.00 · Sep 18: $450.00");
  });

  test("inline rename of a line", async ({ page }) => {
    const name = page.getByLabel("Name of Gas");
    await name.fill("Fuel");
    await name.press("Enter");
    await expect(pz.line(page, "Fuel")).toBeVisible();
  });

  test("reducing a paycheck below its allocations cascades with a notice", async ({ page }) => {
    await page.getByText("Paycheck details").click();
    await page.getByLabel("Planned amount").fill("1500");
    await page.getByRole("button", { name: "Save" }).click();
    await expect(pz.toast(page)).toContainText("Funding was reduced for: Gas.");
    await expect(pz.toast(page)).toContainText("Re-balance the $1,475.00 difference");
    await expect(page.locator('[data-stat="assigned"]')).toHaveText("$1,500.00");
    await expect(pz.line(page, "Gas")).toHaveCount(0);
  });

  test("skipping a paycheck deletes its allocations", async ({ page }) => {
    pz.acceptDialogs(page);
    await page.getByText("Paycheck details").click();
    await page.getByRole("button", { name: "Skip this paycheck" }).click();
    await expect(page.getByText("This paycheck is skipped.")).toBeVisible();
    await expect(pz.sts(page)).toHaveText("$0.00");
    await expect(page.locator("#zero-status")).toContainText("$1,475.00 left to assign");
    await expect(page.locator(".chip.active")).toContainText("Skipped");
  });

  test("deleting a funded paycheck removes its allocations and moves on", async ({ page }) => {
    pz.acceptDialogs(page);
    await page.getByText("Paycheck details").click();
    await page.getByRole("button", { name: "Delete paycheck" }).click();
    await expect(page.getByRole("heading", { name: "Paycheck Sep 18 — Acme Payroll" })).toBeAttached();
    await waitForContent(page);
    await expect(page.locator(".paycheck-strip .chip:not(.add)")).toHaveCount(1);
    await expect(page.locator("#toasts")).toContainText("Removed paycheck(s): Sep 4.");
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
    await expect(page.getByRole("button", { name: "Assign" })).toHaveCount(0);
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
    await page.getByLabel("Amount per paycheck").fill("2000");
    await page.getByLabel("A recent or upcoming payday").fill("2026-10-02");
    await page.getByRole("button", { name: "Add income" }).click();
    await expect(page).toHaveURL(/\/paychecks\//);
    await waitForContent(page);
    const targets = page.locator("section.targets");
    await expect(targets).toContainText("Rent target $1,200.00 · $1,200.00 unfunded");
    await targets.locator("li", { hasText: "Rent" }).getByRole("button").click();
    await expect(pz.line(page, "Rent")).toBeVisible();
    await expect(pz.sts(page)).toHaveText("$800.00");
    expect(s.months["2026-09"].id).toBeTruthy();
  });
});
