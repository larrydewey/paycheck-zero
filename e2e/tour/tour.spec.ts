import { test, pz, waitForContent } from "../tests/fixtures";
import path from "node:path";

const OUT = process.env.PZ_TOUR_OUT ?? path.join(__dirname, "..", "tour-shots");

test("tour", async ({ page, seed, login }, info) => {
  test.setTimeout(180_000);
  let n = 0;
  const shot = async (name: string, full = true) => {
    n += 1;
    await page.waitForTimeout(350);
    await page.screenshot({ path: path.join(OUT, info.project.name, `${String(n).padStart(2, "0")}-${name}.png`), fullPage: full });
  };
  const sheet = page.locator("#sheet");
  const closeSheet = async () => { await sheet.getByRole("button", { name: "Close" }).click(); };
  pz.acceptDialogs(page);

  await seed("empty");
  await page.goto("/register");
  await shot("register");

  let s = await seed("basic");
  await login();
  await shot("plan");
  await page.getByRole("button", { name: "Assign $400.00" }).click();
  await sheet.getByLabel("Expense line").waitFor();
  await shot("sheet-assign", false);
  await closeSheet();
  await page.getByRole("button", { name: "Groceries details" }).click();
  await sheet.getByText("Funded by").waitFor();
  await shot("sheet-line", false);
  await closeSheet();
  await page.getByRole("button", { name: "Paycheck details" }).click();
  await sheet.getByLabel("Amount actually received").waitFor();
  await shot("sheet-paycheck", false);
  await closeSheet();
  await page.getByRole("button", { name: "Add a transaction" }).click();
  await sheet.locator("#add-tx").waitFor();
  await shot("sheet-add-tx", false);
  await closeSheet();

  await pz.overview(page, s);
  await shot("budget");
  await pz.transactions(page, s);
  await shot("spending");
  await pz.income(page, s);
  await shot("income");

  s = await seed("history");
  await login();
  await shot("plan-full");
  await pz.reports(page, s);
  await shot("insights");
  await page.goto("/months");
  await waitForContent(page);
  await shot("months");
  await page.goto("/settings");
  await waitForContent(page);
  await shot("settings");

  await page.emulateMedia({ colorScheme: "dark" });
  await pz.paycheck(page, s, 0);
  await shot("dark-plan");
  await pz.overview(page, s);
  await shot("dark-budget");
  await page.emulateMedia({ colorScheme: "light" });

  s = await seed("locked");
  await login();
  await page.getByRole("button", { name: "Paycheck details" }).click();
  await sheet.getByLabel("Amount actually received").fill("1950");
  await sheet.getByRole("button", { name: "Record actual" }).click();
  await page.locator("#variance-panel").waitFor();
  await shot("variance");

  s = await seed("wallet");
  await login();
  await shot("w-plan");
  await pz.overview(page, s);
  await shot("w-budget");
  await pz.accounts(page, s);
  await shot("w-accounts");
  await page.getByRole("button", { name: "Visa details" }).click();
  await sheet.getByText("Check against your bank").waitFor();
  await shot("w-sheet-card", false);
  await closeSheet();
  await page.getByRole("button", { name: "Checking details" }).click();
  await sheet.getByText("Check against your bank").waitFor();
  await shot("w-sheet-checking", false);
  await closeSheet();
  await page.getByRole("button", { name: "Transfer", exact: true }).click();
  await sheet.locator("#transfer-form").waitFor();
  await shot("w-sheet-transfer", false);
  await closeSheet();
  await page.getByRole("button", { name: "Add account" }).click();
  await sheet.locator("#account-form").waitFor();
  await shot("w-sheet-add-account", false);
  await closeSheet();
  await pz.overview(page, s);
  await page.getByRole("button", { name: "Emergency fund goal" }).click();
  await sheet.getByText("Month by month").waitFor();
  await shot("w-sheet-goal", false);
  await closeSheet();
  await page.getByRole("button", { name: "New goal" }).click();
  await sheet.locator("#goal-form").waitFor();
  await shot("w-sheet-new-goal", false);
  await closeSheet();
  await page.goto(`/months/${s.months["2026-09"].id}/transactions?show=needs-line`);
  await waitForContent(page);
  await shot("w-needs-line");
  await pz.transactions(page, s);
  await shot("w-transactions");
  await page.getByRole("button", { name: /^Theme/ }).first().click();
  await page.getByRole("button", { name: /^Theme/ }).first().click();
  await pz.accounts(page, s);
  await shot("w-dark-accounts");
  await pz.overview(page, s);
  await shot("w-dark-budget");

  await seed("user");
  await login();
  await shot("months-empty");
});
