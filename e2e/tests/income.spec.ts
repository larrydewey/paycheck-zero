import { test, expect, pz, waitForContent } from "./fixtures";

test.describe("income lines", () => {
  test("biweekly income generates paychecks for the month", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.income(page, s);
    const add = page.locator("section.add-income");
    await add.getByLabel("Name").fill("Side gig");
    await add.getByLabel("Amount per paycheck").fill("250");
    await add.getByLabel("Schedule").selectOption("weekly");
    await add.getByLabel("A recent or upcoming payday").fill("2026-09-02");
    await add.getByRole("button", { name: "Add income" }).click();
    await expect(pz.toast(page)).toContainText("Income added with 5 paycheck(s).");
    const card = page.locator("section.income-line", { hasText: "Side gig" });
    await expect(card.locator("tbody tr")).toHaveCount(5);
    await expect(card).toContainText("Weekly on Wednesday");
    await expect(page.locator("#zero-status")).toContainText("$3,125.00 left");
  });

  test("one-off income on a specific date", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.income(page, s);
    const add = page.locator("section.add-income");
    await add.getByLabel("Name").fill("Tax refund");
    await add.getByLabel("Amount per paycheck").fill("640.25");
    await add.getByLabel("Schedule").selectOption("one_off");
    await add.getByLabel("Pay date").fill("2026-09-21");
    await add.getByRole("button", { name: "Add income" }).click();
    const card = page.locator("section.income-line", { hasText: "Tax refund" });
    await expect(card).toContainText("Sep 21");
    await expect(card).toContainText("$640.25");
  });

  test("dates outside the month are refused", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.income(page, s);
    const add = page.locator("section.add-income");
    await add.getByLabel("Name").fill("Bonus");
    await add.getByLabel("Amount per paycheck").fill("10");
    await add.getByLabel("Schedule").selectOption("one_off");
    await add.getByLabel("Pay date").evaluate((el: HTMLInputElement) => { el.removeAttribute("max"); el.value = "2026-10-01"; });
    await add.getByRole("button", { name: "Add income" }).click();
    await expect(pz.toast(page)).toContainText("Oct 1, 2026 isn't in this month.");
  });

  test("smart suggestion from history can be accepted and edited", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    await page.goto("/months?new=2026-10");
    await waitForContent(page);
    await page.getByRole("button", { name: "Create month" }).click();
    await expect(page).toHaveURL(/\/income$/);
    await waitForContent(page);
    await page.getByLabel("Name").pressSequentially("acm");
    const suggestion = page.getByRole("button", { name: /Acme Payroll · \$2,000\.00/ });
    await expect(suggestion).toBeVisible();
    await expect(suggestion).toContainText("(from September 2026)");
    await suggestion.click();
    await expect(page.getByLabel("Name")).toHaveValue("Acme Payroll");
    await expect(page.getByLabel("Amount per paycheck")).toHaveValue("2000.00");
    await expect(page.getByLabel("Schedule")).toHaveValue("biweekly");
    await expect(page.getByLabel("A recent or upcoming payday")).toHaveValue("2026-09-04");
    await page.getByLabel("Amount per paycheck").fill("2100");
    await page.getByRole("button", { name: "Add income" }).click();
    await expect(page).toHaveURL(/\/paychecks\//);
    await waitForContent(page);
    await expect(page.locator(".paycheck-strip .chip:not(.add)")).toHaveCount(3);
    await expect(pz.sts(page)).toHaveText("$2,100.00");
  });

  test("suggestions can be discarded", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    await page.goto("/months?new=2026-10");
    await waitForContent(page);
    await page.getByRole("button", { name: "Create month" }).click();
    await waitForContent(page);
    await page.getByLabel("Name").pressSequentially("Acme");
    await expect(page.getByRole("button", { name: /Acme Payroll/ })).toBeVisible();
    await page.getByRole("button", { name: "No thanks" }).click();
    await expect(page.getByRole("button", { name: /Acme Payroll/ })).toHaveCount(0);
  });

  test("mid-month paycheck can be added to an income line", async ({ page, seed, login }) => {
    const s = await seed("balanced");
    await login();
    await pz.income(page, s);
    const card = page.locator("section.income-line", { hasText: "Acme Payroll" });
    await card.getByText("Edit or add a paycheck").click();
    await card.getByLabel("Extra pay date").fill("2026-09-25");
    await card.getByRole("button", { name: "Add paycheck" }).click();
    await expect(pz.toast(page)).toContainText("Paycheck added.");
    await expect(card.locator("tbody tr")).toHaveCount(3);
    await expect(page.locator("#zero-status")).toContainText("$2,000.00 left to assign");
  });

  test("changing a schedule removes funded paychecks and says so", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.income(page, s);
    const card = page.locator("section.income-line", { hasText: "Acme Payroll" });
    await card.getByText("Edit or add a paycheck").click();
    await card.getByLabel("Schedule").selectOption("monthly");
    await card.getByLabel("Days of the month").fill("4");
    await card.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#toasts")).toContainText("Removed paycheck(s): Sep 18.");
    await expect(page.locator("#toasts")).toContainText("Funding was reduced for: Groceries, Electric, Visa.");
    await expect(page.locator("section.income-line tbody tr")).toHaveCount(1);
  });

  test("deleting an income line removes its paychecks", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    pz.acceptDialogs(page);
    await pz.income(page, s);
    const card = page.locator("section.income-line", { hasText: "Acme Payroll" });
    await card.getByText("Edit or add a paycheck").click();
    await card.getByRole("button", { name: "Delete income line" }).click();
    await expect(page.getByRole("heading", { name: "No income yet" })).toBeVisible();
    await expect(page.locator("#zero-status")).toContainText("No income yet");
    await expect(page.getByRole("button", { name: "Lock month" })).toHaveCount(0);
  });
});
