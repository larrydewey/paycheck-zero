import AxeBuilder from "@axe-core/playwright";
import { test, expect, pz, waitForContent } from "./fixtures";

async function audit(page: import("@playwright/test").Page) {
  // Let entrance animations settle so contrast is measured on the final colours.
  await page.evaluate(() => Promise.all(document.getAnimations().filter((a) => a.effect?.getTiming().iterations !== Infinity).map((a) => a.finished.catch(() => null))));
  const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"]).analyze();
  const summary = r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => `${n.target.join(" ")} (${n.any[0]?.message ?? ""})`).slice(0, 3).join(", ")}`);
  expect(summary).toEqual([]);
}

test.describe("accessibility (WCAG 2.1 AA via axe)", () => {
  test("sign in and register", async ({ page, seed }) => {
    await seed("empty");
    await page.goto("/login");
    await audit(page);
    await page.goto("/register");
    await audit(page);
  });

  test("month pages", async ({ page, seed, login }) => {
    const s = await seed("history");
    await login();
    await audit(page); // paycheck view
    // Sheets
    await pz.lineSheet(page, "Groceries");
    await audit(page);
    await pz.closeSheet(page);
    await pz.openSheet(page, "Paycheck details");
    await audit(page);
    await pz.closeSheet(page);
    await pz.addTx(page);
    await audit(page);
    await pz.closeSheet(page);
    await pz.overview(page, s);
    await audit(page);
    await pz.income(page, s);
    await audit(page);
    await pz.transactions(page, s);
    await audit(page);
    await pz.reports(page, s);
    await audit(page);
  });

  test("months list, dialog, settings, variance and locked states", async ({ page, seed, login }) => {
    const s = await seed("locked");
    await login();
    await page.goto("/months");
    await waitForContent(page);
    await audit(page);
    await page.getByRole("button", { name: "New month" }).first().click();
    await audit(page);
    await page.keyboard.press("Escape");
    await page.goto("/settings");
    await waitForContent(page);
    await audit(page);
    await pz.paycheck(page, s, 0);
    await pz.openSheet(page, "Paycheck details");
    await pz.sheet(page).getByLabel("Amount actually received").fill("1900");
    await pz.sheet(page).getByRole("button", { name: "Record actual" }).click();
    await expect(page.locator("#variance-panel")).toBeVisible();
    await audit(page);
  });

  test("dark mode", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await page.emulateMedia({ colorScheme: "dark" });
    await login();
    await audit(page);
    await pz.openSheet(page, "Assign $400.00");
    await audit(page);
    await pz.closeSheet(page);
    await pz.lineSheet(page, "Groceries");
    await audit(page);
    await pz.closeSheet(page);
    await pz.overview(page, s);
    await audit(page);
    await pz.transactions(page, s);
    await audit(page);
    await pz.reports(page, s);
    await audit(page);
  });

  test("error toast and keyboard navigation", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.overview(page, s);
    await page.getByRole("button", { name: "Lock month" }).click();
    await expect(pz.toast(page)).toBeVisible();
    await audit(page);
    // Skip link is the first tab stop.
    await page.reload();
    await waitForContent(page);
    await page.keyboard.press("Tab");
    await expect(page.getByRole("link", { name: "Skip to content" })).toBeFocused();
  });
});

test.describe("accessibility: accounts, goals, transfers", () => {
  test("accounts, card and goal sheets pass axe in light and dark", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    for (const scheme of ["light", "dark"] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await pz.accounts(page, s);
      await audit(page);
      await pz.openSheet(page, "Visa details");
      await audit(page);
      await pz.closeSheet(page);
      await pz.openSheet(page, "Transfer", { exact: true });
      await audit(page);
      await pz.closeSheet(page);
      await pz.openSheet(page, "Add account");
      await audit(page);
      await pz.closeSheet(page);
      await pz.overview(page, s);
      await audit(page);
      await pz.openSheet(page, "Emergency fund goal");
      await audit(page);
      await pz.closeSheet(page);
      await pz.openSheet(page, "New goal");
      await audit(page);
      await pz.closeSheet(page);
      await page.goto(`/months/${s.months["2026-09"].id}/transactions?show=needs-line`);
      await waitForContent(page);
      await audit(page);
      await page.goto("/settings");
      await waitForContent(page);
      await audit(page);
    }
  });
});
