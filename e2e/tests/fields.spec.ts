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
    const amt = page.getByLabel("Amount", { exact: true });
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
    await page.getByRole("heading", { name: "Your budget, from this paycheck" }).hover();
    await expect(pz.sts(page)).toHaveText("$413.00");
  });

  test("add forms clear after a successful submit", async ({ page }) => {
    await page.getByRole("link", { name: "Month overview" }).click();
    const input = page.getByLabel("New line in Food", { exact: true });
    await input.fill("Snacks");
    await input.press("Enter");
    await expect(pz.line(page, "Snacks")).toBeVisible();
    await expect(input).toHaveValue("");
    await page.getByLabel("New category").fill("Pets");
    await page.getByRole("button", { name: "Add category" }).click();
    await expect(pz.category(page, "Pets")).toBeVisible();
    await expect(page.getByLabel("New category")).toHaveValue("");
  });

  test("a rejected add keeps what you typed", async ({ page }) => {
    await page.getByLabel("Expense line").selectOption({ label: "Electric" });
    const amt = page.getByLabel("Amount", { exact: true });
    await page.evaluate(() => document.querySelectorAll("[data-max-cents]").forEach((e) => e.removeAttribute("data-max-cents")));
    await amt.fill("999");
    await page.getByRole("button", { name: "Assign" }).click();
    await expect(pz.toast(page)).toContainText("over-allocate");
    await expect(amt).toHaveValue("999");
  });
});
