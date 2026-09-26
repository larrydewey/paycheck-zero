import { test, expect, pz } from "./fixtures";

test.describe("field behaviour", () => {
  test.beforeEach(async ({ seed, login }) => {
    await seed("basic");
    await login();
  });

  test("amount fields evaluate arithmetic (600 + 80 → 680.00)", async ({ page }) => {
    const input = page.getByLabel("Planned for Rent from this paycheck");
    await input.fill("600 + 80");
    await input.press("Enter");
    await expect(input).toHaveValue("680.00");
    await expect(pz.category(page, "Housing").locator("summary [data-col=this]")).toContainText("$680.00");
    const form = await pz.addTx(page);
    const amt = form.getByLabel("Amount", { exact: true });
    await amt.fill("(100+20)/3");
    await amt.blur();
    await expect(amt).toHaveValue("40.00");
  });

  test("the browser guard understands arithmetic too", async ({ page }) => {
    const input = page.getByLabel("Planned for Rent from this paycheck");
    await input.fill("1200 + 400.01");
    await input.press("Enter");
    await expect(pz.line(page, "Rent").locator(".field-error")).toHaveText("That's $0.01 more than this paycheck has left.");
  });

  test("inline edits save when the pointer leaves the row", async ({ page, isMobile }) => {
    test.skip(isMobile, "no hover on touch screens");
    const input = page.getByLabel("Planned for Gas from this paycheck");
    await input.click();
    await input.fill("75");
    await page.getByRole("heading", { name: "What this paycheck pays for" }).hover();
    await expect(pz.sts(page)).toHaveText("$413.00");
  });

  test("add forms clear after a successful submit", async ({ page }) => {
    await page.getByRole("link", { name: "Budget" }).first().click();
    const input = page.getByLabel("New line in Food", { exact: true });
    await input.fill("Snacks");
    await input.press("Enter");
    await expect(pz.line(page, "Snacks")).toBeVisible();
    await expect(input).toHaveValue("");
    await page.getByLabel("New category").fill("Pets");
    await page.getByRole("button", { name: "Add category" }).click();
    await expect(page.locator('[data-category-empty="Pets"]')).toBeVisible();
    await expect(page.getByLabel("New category")).toHaveValue("");
  });

  test("a rejected add keeps what you typed", async ({ page }) => {
    await pz.openSheet(page, "Assign $400.00");
    const sh = pz.sheet(page);
    await sh.getByLabel("Expense line").selectOption({ label: "Electric" });
    const amt = sh.getByLabel("Amount", { exact: true });
    await page.evaluate(() => document.querySelectorAll("[data-max-cents]").forEach((e) => e.removeAttribute("data-max-cents")));
    await amt.fill("999");
    await sh.getByRole("button", { name: "Assign", exact: true }).click();
    await expect(pz.toast(page)).toContainText("over-allocate");
    await expect(amt).toHaveValue("999");
  });
});
