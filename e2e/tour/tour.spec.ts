import { test, pz, waitForContent } from "../tests/fixtures";
import path from "node:path";

const OUT = process.env.PZ_TOUR_OUT ?? path.join(__dirname, "..", "tour-shots");

test("tour", async ({ page, seed, login }, info) => {
  test.setTimeout(120_000);
  let n = 0;
  const shot = async (name: string) => {
    n += 1;
    await page.waitForTimeout(150);
    await page.screenshot({ path: path.join(OUT, info.project.name, `${String(n).padStart(2, "0")}-${name}.png`), fullPage: true });
  };
  pz.acceptDialogs(page);

  await seed("empty");
  await page.goto("/register");
  await shot("register");
  await page.goto("/login");
  await shot("login");

  let s = await seed("basic");
  await login();
  await shot("paycheck");
  await page.getByLabel("Expense line").selectOption("__new");
  await shot("paycheck-fund-new");
  await page.getByLabel("Planned for Rent from this paycheck").fill("99999");
  await page.getByLabel("Planned for Rent from this paycheck").press("Enter");
  await shot("paycheck-client-guard");
  await page.getByRole("button", { name: "Lock month" }).click();
  await shot("paycheck-lock-error");
  await page.getByText("Paycheck details").click();
  await shot("paycheck-details");
  await pz.paycheck(page, s, 1);
  await shot("paycheck-2");
  await pz.overview(page, s);
  await shot("overview-draft");
  s = await seed("history");
  await login();
  await shot("paycheck-full");

  await pz.overview(page, s);
  await shot("overview");
  await pz.overview(page, s, "2026-08");
  await shot("overview-locked");
  await pz.income(page, s);
  await shot("income");
  await pz.transactions(page, s);
  await shot("transactions");
  await page.getByRole("button", { name: /Edit Shell/ }).click();
  await shot("transactions-edit");
  await pz.reports(page, s);
  await shot("reports");
  await pz.reports(page, s, "2025-09");
  await shot("reports-old");
  await page.goto("/months");
  await waitForContent(page);
  await shot("months");
  await page.getByRole("button", { name: "New month" }).first().click();
  await shot("months-new-dialog");
  await page.keyboard.press("Escape");
  await page.locator('.month-card[data-month="2025-09"]').getByText("Delete permanently").click();
  await shot("months-delete");
  await page.goto("/settings");
  await waitForContent(page);
  await shot("settings");

  await page.emulateMedia({ colorScheme: "dark" });
  await pz.overview(page, s);
  await shot("dark-overview");
  await pz.paycheck(page, s, 0);
  await shot("dark-paycheck");
  await page.emulateMedia({ colorScheme: "light" });

  s = await seed("locked");
  await login();
  await page.getByText("Paycheck details").click();
  await page.getByLabel("Amount actually received").fill("1950");
  await page.getByRole("button", { name: "Record actual" }).click();
  await page.locator("#variance-panel").waitFor();
  await shot("variance");
  await page.getByRole("button", { name: "Re-assign variance" }).click();
  await page.getByText("Variance re-assignment is open").waitFor();
  await shot("reassigning");

  await seed("user");
  await login();
  await shot("months-empty");
  await page.getByRole("button", { name: "New month" }).first().click();
  await page.getByLabel("Month", { exact: true }).fill("2026-09");
  await page.getByRole("button", { name: "Create month" }).click();
  await page.waitForURL(/income/);
  await waitForContent(page);
  await shot("income-empty");
  await page.getByRole("link", { name: "Month overview" }).click();
  await waitForContent(page);
  await shot("overview-empty");
  await page.getByRole("link", { name: "Transactions" }).click();
  await waitForContent(page);
  await shot("transactions-empty");
  await page.getByRole("link", { name: "Reports" }).click();
  await waitForContent(page);
  await shot("reports-empty");
});
