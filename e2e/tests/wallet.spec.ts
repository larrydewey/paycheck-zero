import { test, expect, pz, waitForContent } from "./fixtures";

const acct = (page: import("@playwright/test").Page, name: string) => page.locator(`li.account[data-account="${name}"]`);

test.describe("accounts and credit cards", () => {
  test("add accounts; balances follow tagged transactions", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.accounts(page, s);
    await expect(page.getByRole("heading", { name: "See your real balances" })).toBeVisible();
    await pz.openSheet(page, "Add an account");
    let sh = pz.sheet(page);
    await sh.getByLabel("Account name").fill("Checking");
    await sh.getByLabel("Balance right now").fill("2,000");
    await sh.getByRole("button", { name: "Add account" }).click();
    await expect(sh).toBeHidden();
    await expect(acct(page, "Checking").locator("[data-col=balance]")).toHaveText("$2,000.00");

    await pz.openSheet(page, "Add account");
    sh = pz.sheet(page);
    await sh.getByRole("radio", { name: "Credit card" }).check();
    await sh.getByLabel("Account name").fill("Visa");
    await sh.getByLabel("Amount owed right now").fill("300");
    await sh.getByLabel("Credit limit").fill("1000");
    await sh.getByRole("button", { name: "Add account" }).click();
    await expect(sh).toBeHidden();
    await expect(acct(page, "Visa").locator("[data-col=owed]")).toHaveText("$300.00");
    await expect(page.locator('[data-card="net"]')).toContainText("$1,700.00");

    // A grocery run on the card counts against Groceries and adds to what's owed.
    const form = await pz.addTx(page);
    await form.getByLabel("Amount", { exact: true }).fill("50");
    await form.getByLabel("Payee").fill("Aldi");
    await form.getByLabel("Account").selectOption({ label: "Visa" });
    await form.getByLabel("Expense line", { exact: true }).selectOption({ label: "Groceries" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    await expect(acct(page, "Visa").locator("[data-col=owed]")).toHaveText("$350.00");
    await expect(acct(page, "Visa").locator("[data-col=ready]")).toHaveText("$50.00");
    await pz.overview(page, s);
    await expect(pz.line(page, "Groceries").locator("[data-col=spent]")).toContainText("$135.20");
    // The next add remembers the account.
    const again = await pz.addTx(page);
    await expect(again.getByLabel("Account")).toHaveValue(/.+/);
    await expect(again.getByLabel("Account").locator("option:checked")).toHaveText("Visa");
  });

  test("reconcile a balance against the bank", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    await pz.accounts(page, s);
    await expect(acct(page, "Checking").locator("[data-col=balance]")).toHaveText("$2,410.00");
    await pz.openSheet(page, "Checking details");
    await pz.sheet(page).getByLabel("Balance in your bank app").fill("2,400.50");
    await pz.sheet(page).getByRole("button", { name: "Update" }).click();
    await expect(pz.toast(page)).toContainText("Adjusted by -$9.50 to match your bank.");
    await expect(acct(page, "Checking").locator("[data-col=balance]")).toHaveText("$2,400.50");
    await expect(acct(page, "Checking")).toContainText("Checked Sep 10");
    await pz.openSheet(page, "Checking details");
    await pz.sheet(page).getByRole("button", { name: "Update" }).click();
    await expect(pz.toast(page)).toContainText("It matches.");
  });

  test("pay a card: a transfer that isn't spending", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    await pz.accounts(page, s);
    const visa = acct(page, "Visa");
    await expect(visa.locator("[data-col=owed]")).toHaveText("$917.20");
    await expect(visa.locator("[data-col=ready]")).toHaveText("$85.20");
    await pz.openSheet(page, "Visa details");
    await expect(pz.sheet(page).locator("[data-col=carried]")).toHaveText("$832.00");
    await pz.sheet(page).getByRole("button", { name: "Pay $85.20" }).click();
    const sh = pz.sheet(page);
    await expect(sh.locator("#transfer-form")).toBeVisible();
    await expect(sh.getByLabel("From")).toHaveValue(s.accounts!["Checking"]);
    await expect(sh.getByLabel("To")).toHaveValue(s.accounts!["Visa"]);
    await expect(sh.getByLabel("Amount", { exact: true })).toHaveValue("85.20");
    await sh.getByRole("button", { name: "Save transfer" }).click();
    await expect(pz.toast(page)).toContainText("Transfer saved.");
    await expect(visa.locator("[data-col=owed]")).toHaveText("$832.00");
    await expect(visa.locator("[data-col=ready]")).toHaveCount(0);
    await expect(acct(page, "Checking").locator("[data-col=balance]")).toHaveText("$2,324.80");
    // Transfers show as a route and don't count as money out.
    await pz.transactions(page, s);
    const row = page.locator("#tx-list li.transfer");
    await expect(row).toContainText("Checking → Visa");
    await expect(row.locator(".tx-amt")).toHaveText("$85.20");
    await expect(page.getByText("Money out").locator("..")).toContainText("$137.20");
    // Safe to Spend is untouched.
    await pz.paycheck(page, s, 0);
    await expect(pz.sts(page)).toHaveText("$388.00");
  });

  test("paying down carried debt from a debt line counts against that line", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    await pz.accounts(page, s);
    await pz.openSheet(page, "Transfer", { exact: true });
    const sh = pz.sheet(page);
    await sh.getByLabel("From").selectOption({ label: "Checking" });
    await sh.getByLabel("To").selectOption({ label: "Visa" });
    await sh.getByLabel("Amount", { exact: true }).fill("75");
    await sh.getByText("Count it in your budget?").click();
    await sh.getByLabel("Budget line").selectOption({ label: "Visa" });
    await sh.getByRole("button", { name: "Save transfer" }).click();
    await expect(pz.toast(page)).toContainText("Transfer saved.");
    await expect(acct(page, "Visa").locator("[data-col=carried]")).toHaveText("$757.00");
    await pz.overview(page, s);
    await expect(pz.line(page, "Visa").locator("[data-col=spent]")).toContainText("$75.00");
    // Edit the transfer from the list.
    await pz.transactions(page, s);
    await page.locator("#tx-list li.transfer .tx-open").click();
    await expect(pz.sheet(page).getByRole("heading", { name: "Edit transfer" })).toBeVisible();
    await pz.sheet(page).getByLabel("Amount", { exact: true }).fill("80");
    await pz.sheet(page).getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.locator("#tx-list li.transfer .tx-amt")).toHaveText("$80.00");
  });

  test("an account with history is archived, not deleted", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    pz.acceptDialogs(page);
    await pz.accounts(page, s);
    await pz.openSheet(page, "Savings details");
    await pz.sheet(page).getByRole("button", { name: "Delete Savings" }).click();
    await expect(acct(page, "Savings")).toHaveCount(0);
    await pz.openSheet(page, "Visa details");
    await expect(pz.sheet(page).getByRole("button", { name: "Delete Visa" })).toHaveCount(0);
    await pz.sheet(page).getByRole("button", { name: "Archive" }).click();
    await expect(acct(page, "Visa")).toHaveCount(0);
    // Its transactions keep their history.
    await pz.transactions(page, s);
    await expect(page.locator("#tx-list li", { hasText: "Trader Joe's" })).toBeVisible();
  });
});

test.describe("transactions that need a line", () => {
  test("they're flagged everywhere and easy to sort", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await expect(page.locator(".btab-icon [data-badge=needs-line], .tab [data-badge=needs-line]").filter({ visible: true })).toHaveText("1");
    await expect(page.locator("#needs-line-alert")).toContainText("1 transaction(s) ($12.00) need a line.");
    await pz.overview(page, s);
    await page.locator("#needs-line-alert").getByRole("link", { name: "Sort them" }).click();
    await waitForContent(page);
    await expect(page).toHaveURL(/show=needs-line/);
    await expect(page.locator("#tx-list li.tx")).toHaveCount(1);
    const coffee = page.locator("#tx-list li.tx.needs-line", { hasText: "Coffee" });
    await expect(coffee).toContainText("Needs a line");
    const dlg = await pz.editTx(page, "Edit Coffee on Sep 7");
    await dlg.getByLabel("Expense line", { exact: true }).selectOption({ label: "Groceries" });
    await dlg.getByRole("button", { name: "Save" }).click();
    await expect(page.getByText("Every transaction has a line")).toBeVisible();
    await expect(page.locator("[data-badge=needs-line]").first()).toBeHidden();
  });

  test("the plan shows recent spending from the paycheck", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    const recent = page.locator("section.recent");
    await expect(recent).toContainText("Coffee");
    await expect(recent).toContainText("Trader Joe's");
    await recent.getByRole("button", { name: "Edit Trader Joe's on Sep 5" }).click();
    await expect(pz.sheet(page).getByRole("heading", { name: "Edit transaction" })).toBeVisible();
  });
});

test.describe("goals", () => {
  test("a savings goal shows where it stands and how to catch up", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    await pz.overview(page, s);
    const ef = page.locator('li.goal[data-goal="Emergency fund"]');
    await expect(ef.locator("[data-goal-status]")).toHaveText("Behind");
    await expect(ef).toContainText("$1,000.00 saved of $5,000.00 · 20%");
    await expect(ef).toContainText("This month $0.00 of $400.00 needed · by Jun 2027");
    await pz.openSheet(page, "Emergency fund goal");
    await pz.sheet(page).getByRole("button", { name: "Plan $400.00 more" }).click();
    await expect(ef.locator("[data-goal-status]")).toHaveText("On track");
    await expect(ef).toContainText("$1,400.00 saved of $5,000.00");
    await expect(pz.line(page, "Emergency Fund").locator("[data-col=planned]")).toHaveText("$400.00");
  });

  test("create, edit and delete goals; payoff goals follow the card", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    pz.acceptDialogs(page);
    await pz.overview(page, s);
    await pz.openSheet(page, "New goal");
    let sh = pz.sheet(page);
    await sh.getByLabel("Goal name").fill("Vacation");
    await sh.getByLabel("Target amount").fill("1200");
    await sh.getByLabel("Counts").selectOption({ label: "Savings (balance)" });
    await sh.getByRole("button", { name: "Create goal" }).click();
    await expect(pz.toast(page)).toContainText("Goal created.");
    const vac = page.locator('li.goal[data-goal="Vacation"]');
    // Savings already holds $5,000, so it's reached straight away.
    await expect(vac).toContainText("$5,000.00 saved of $1,200.00");
    await expect(vac.locator("[data-goal-status]")).toHaveText("Done");

    await pz.openSheet(page, "Vacation goal");
    sh = pz.sheet(page);
    await sh.getByLabel("Target amount").fill("8000");
    await sh.getByLabel("Reach it by (optional)").fill("2027-08");
    await sh.getByRole("button", { name: "Save", exact: true }).click();
    await expect(vac).toContainText("$5,000.00 saved of $8,000.00 · 62%");
    await expect(vac).toContainText("by Aug 2027");

    // Pay the card down; the payoff goal moves.
    const visaGoal = page.locator('li.goal[data-goal="Visa paid off"]');
    await expect(visaGoal).toContainText("$0.00 paid off of $917.20");
    await page.goto(`/months/${s.months["2026-09"].id}/accounts`);
    await waitForContent(page);
    await pz.openSheet(page, "Visa details");
    await pz.sheet(page).getByRole("button", { name: "Pay $85.20" }).click();
    await pz.sheet(page).getByRole("button", { name: "Save transfer" }).click();
    await expect(pz.toast(page)).toContainText("Transfer saved.");
    await pz.overview(page, s);
    await expect(visaGoal).toContainText("$85.20 paid off of $917.20");

    await pz.openSheet(page, "Vacation goal");
    await pz.sheet(page).getByRole("button", { name: "Delete Vacation" }).click();
    await expect(vac).toHaveCount(0);
  });
});

test.describe("appearance", () => {
  test("the theme button cycles device → light → dark and is remembered", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    const html = page.locator("html");
    const btn = page.getByRole("button", { name: /^Theme/ });
    await expect(html).not.toHaveAttribute("data-theme", /.+/);
    await btn.click();
    await expect(html).toHaveAttribute("data-theme", "light");
    await expect(btn).toHaveAccessibleName("Theme: light. Switch to dark");
    await btn.click();
    await expect(html).toHaveAttribute("data-theme", "dark");
    await page.reload();
    await waitForContent(page);
    await expect(html).toHaveAttribute("data-theme", "dark");
    const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    expect(bg).toBe("rgb(11, 18, 32)");
    await page.goto("/settings");
    await waitForContent(page);
    await expect(page.getByRole("radio", { name: "Dark" })).toBeChecked();
    await page.getByRole("radio", { name: "Match device" }).check();
    await expect(html).not.toHaveAttribute("data-theme", /.+/);
  });
});

test.describe("retirement and investments", () => {
  test("contributions, market growth and a Roth transfer from checking", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    pz.acceptDialogs(page);
    await pz.accounts(page, s);
    const k = acct(page, "401(k)");
    await expect(page.locator("#invested-accounts")).toContainText("Retirement & investments");
    await expect(k.locator("[data-col=balance]")).toHaveText("$42,300.00");
    await expect(k).toContainText("$300.00 in this year");
    await expect(page.locator('[data-card="invested"]')).toContainText("$42,300.00");
    await expect(page.locator('[data-card="net"]')).toContainText("$48,792.80");

    // Update from the statement: the difference is growth, not spending.
    await pz.openSheet(page, "401(k) details");
    let sh = pz.sheet(page);
    await sh.getByLabel("Balance on your latest statement").fill("43,000");
    await sh.getByRole("button", { name: "Update" }).click();
    await expect(pz.toast(page)).toContainText("Balance updated: +$700.00 growth.");
    await expect(k.locator("[data-col=growth]")).toHaveText("+$700.00 growth");

    // A payroll contribution.
    await pz.openSheet(page, "401(k) details");
    sh = pz.sheet(page);
    await sh.getByLabel("Amount", { exact: true }).fill("300");
    await sh.getByRole("button", { name: "Add contribution" }).click();
    await expect(pz.toast(page)).toContainText("Contribution added.");
    await expect(k.locator("[data-col=balance]")).toHaveText("$43,300.00");
    await expect(k).toContainText("$600.00 in this year");

    // Remove the growth entry again.
    await pz.openSheet(page, "401(k) details");
    await pz.sheet(page).getByRole("button", { name: /^Delete Market growth on/ }).click();
    await expect(k.locator("[data-col=balance]")).toHaveText("$42,600.00");

    // Contribute from checking: a transfer, counted as planned saving when linked.
    await pz.openSheet(page, "401(k) details");
    await pz.sheet(page).getByRole("button", { name: "Contribute from checking" }).click();
    sh = pz.sheet(page);
    await expect(sh.getByLabel("To")).toHaveValue(/.+/);
    await sh.getByLabel("Amount", { exact: true }).fill("100");
    await sh.getByText("Count it in your budget?").click();
    await sh.getByLabel("Budget line").selectOption({ label: "Emergency Fund" });
    await sh.getByRole("button", { name: "Save transfer" }).click();
    await expect(page.locator("#toasts")).toContainText("Transfer saved.");
    await expect(k.locator("[data-col=balance]")).toHaveText("$42,700.00");
    await expect(acct(page, "Checking").locator("[data-col=balance]")).toHaveText("$2,310.00");

    // Retirement accounts aren't offered for everyday spending.
    const form = await pz.addTx(page);
    await expect(form.getByLabel("Account").locator("option", { hasText: "401(k)" })).toHaveCount(0);
  });
});
