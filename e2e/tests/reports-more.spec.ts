import { test, expect, pz } from "./fixtures";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { Download, Page } from "@playwright/test";

async function csvFrom(page: Page, click: () => Promise<void>) {
  const [dl] = await Promise.all([page.waitForEvent("download"), click()]);
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "pz-r-")), (dl as Download).suggestedFilename());
  await (dl as Download).saveAs(file);
  return { name: (dl as Download).suggestedFilename(), text: fs.readFileSync(file, "utf8") };
}

test.describe("reports: drill-down, trends, payees, exports", () => {
  test.beforeEach(async ({ page, seed, login }) => {
    const s = await seed("history");
    await login();
    await pz.reports(page, s);
  });

  test("category rows drill down to their lines", async ({ page }) => {
    await page.getByRole("link", { name: "Compare", exact: true }).click();
    const housing = page.locator("#mom").getByRole("button", { name: "Housing" });
    const rent = page.locator('#mom tr[data-row="line:Housing:Rent"]');
    await expect(rent).toBeHidden();
    await housing.click();
    await expect(housing).toHaveAttribute("aria-expanded", "true");
    await expect(rent).toBeVisible();
    await expect(rent).toContainText("$1,200.00");
    await housing.click();
    await expect(rent).toBeHidden();
  });

  test("trends over 3/6/12 months by category or line", async ({ page }) => {
    await page.getByRole("link", { name: "Trends" }).click();
    await expect(page.getByRole("heading", { name: "Last 6 months" })).toBeVisible();
    const head = page.locator("#trend thead");
    await expect(head).toContainText("Apr 2026");
    await expect(head).toContainText("Sep 2026");
    await expect(page.locator('#trend tr[data-row="income"]')).toContainText("$4,050.00");
    await page.getByLabel("Months", { exact: true }).selectOption("12");
    await page.getByLabel("Rows", { exact: true }).selectOption("line");
    await page.getByLabel("Show", { exact: true }).selectOption("planned");
    await page.getByRole("button", { name: "Apply" }).click();
    await expect(page).toHaveURL(/tab=trends&n=12&level=line&metric=planned/);
    await expect(page.getByRole("heading", { name: "Last 12 months" })).toBeVisible();
    await expect(page.locator("#trend thead")).toContainText("Oct 2025");
    await expect(page.locator('#trend tr[data-row="line:Housing › Rent"]')).toContainText("$1,200.00");
  });

  test("spending by payee for any date range", async ({ page }) => {
    await page.getByRole("link", { name: "Payees" }).click();
    await expect(page.locator('#payees tr[data-row="payee:Trader Joe\'s"]')).toContainText("$85.20");
    await page.getByLabel("From", { exact: true }).fill("2026-08-01");
    await page.getByLabel("To", { exact: true }).fill("2026-09-30");
    await page.getByRole("button", { name: "Apply" }).click();
    const tj = page.locator('#payees tr[data-row="payee:Trader Joe\'s"]');
    await expect(tj).toContainText("2");
    await expect(tj).toContainText("$170.40");
    await expect(page.locator("#payees tbody tr").first()).toContainText("Landlord");
  });

  test("every report downloads as CSV, plus transactions for a date range", async ({ page }) => {
    await page.getByRole("link", { name: "Compare", exact: true }).click();
    await expect(page.locator("#mom")).toBeVisible();
    const mom = await csvFrom(page, () => page.getByRole("link", { name: "Download CSV" }).first().click());
    expect(mom.name).toBe("mom-2026-09.csv");
    expect(mom.text.split("\r\n")[0]).toBe("row,category,line,Sep 2026 planned,Sep 2026 actual,Aug 2026 planned,Aug 2026 actual");
    expect(mom.text).toContain("line,Housing,Rent,1200.00,0.00,1200.00,1200.00");
    await page.getByRole("link", { name: "Export" }).click();
    for (const [label, file] of [["Year to date (CSV)", "ytd-2026-09.csv"], ["Year over year (CSV)", "yoy-2026-09.csv"], ["Trends (CSV)", "trends-6m-2026-09.csv"], ["Payees (CSV)", "payees-2026-09-01-to-2026-09-30.csv"]]) {
      const got = await csvFrom(page, () => page.getByRole("link", { name: label }).click());
      expect(got.name).toBe(file);
      expect(got.text.length).toBeGreaterThan(20);
    }
    await page.getByLabel("From", { exact: true }).fill("2025-09-01");
    await page.getByLabel("To", { exact: true }).fill("2026-09-30");
    const tx = await csvFrom(page, () => page.getByRole("button", { name: "Download CSV" }).click());
    expect(tx.name).toBe("transactions-2025-09-01-to-2026-09-30.csv");
    const lines = tx.text.trim().split("\r\n");
    expect(lines[0]).toBe("date,month,payee,category,line,paycheck,amount,notes,split_group");
    expect(lines[1]).toContain("2025-09-03,2025-09,Market,Food,Groceries,,-450.00");
    expect(lines).toHaveLength(1 + 1 + 4 + 3);
  });
});
