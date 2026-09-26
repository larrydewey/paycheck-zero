import { test, expect, pz, waitForContent } from "./fixtures";

const acct = (page: import("@playwright/test").Page, name: string) => page.locator(`li.account[data-account="${name}"]`);

async function connect(page: import("@playwright/test").Page) {
  await pz.openSheet(page, "Connect a bank");
  await expect(pz.sheet(page)).toContainText("PaycheckZero never sees your password");
  await page.getByRole("button", { name: "Continue with Plaid" }).click();
  await expect(pz.sheet(page).getByRole("heading", { name: "Accounts at Test Bank" })).toBeVisible();
}

test.describe("bank sync", () => {
  test("connect a bank: accounts, balances and sorted transactions arrive", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.accounts(page, s);
    await connect(page);
    const sh = pz.sheet(page);
    await expect(sh.locator("#map-p_chk")).toHaveValue("new");
    await sh.getByRole("button", { name: "Import" }).click();
    await expect(page.locator("#toasts")).toContainText(
      "Synced Test Bank: 5 new transaction(s), 1 sorted into lines, 2 matched one you'd entered, 1 transfer(s) paired, 1 paycheck(s) recognized."
    );
    await expect(page.locator("#toasts")).toContainText("2 imported transaction(s) need a line.");
    await expect(acct(page, "Checking ••1234").locator("[data-col=balance]")).toHaveText("$2,410.00");
    await expect(acct(page, "Checking ••1234")).toContainText("Synced Sep 10");
    await expect(acct(page, "Sapphire ••9876").locator("[data-col=owed]")).toHaveText("$250.00");
    await expect(page.locator('li.bank-link[data-link="Test Bank"]')).toContainText("Synced");

    await pz.transactions(page, s);
    const list = page.locator("#tx-list");
    await expect(list.locator("li.tx", { hasText: "Shell" })).toHaveCount(1);
    await expect(list.locator("li.tx", { hasText: "Coffee" })).toHaveCount(1);
    await expect(list.locator("li.tx", { hasText: "Netflix" })).toHaveCount(0);
    await expect(list.locator("li.tx", { hasText: "Old" })).toHaveCount(0);
    await expect(list.locator("li.tx", { hasText: "Trader Joe's" }).filter({ hasText: "-$23.50" })).toContainText("Groceries");
    await expect(list.locator("li.transfer")).toContainText("Checking ••1234 → Sapphire ••9876");
    await expect(list.locator("li.tx.needs-line", { hasText: "Target" })).toBeVisible();
    // The payroll deposit recorded what the Sep 4 paycheck actually paid.
    await pz.income(page, s);
    await expect(page.locator("section.income-line tbody tr", { hasText: "Sep 4" })).toContainText("$1,950.00");
  });

  test("syncing again adds only what's new", async ({ page, seed, login, server }) => {
    const s = await seed("basic");
    await login();
    await pz.accounts(page, s);
    await connect(page);
    await pz.sheet(page).getByRole("button", { name: "Import" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced Test Bank");
    await pz.openSheet(page, "Test Bank connection");
    await pz.sheet(page).getByRole("button", { name: "Sync now" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced Test Bank: 0 new transaction(s).");
    await pz.bank(server, { extra: true });
    await pz.openSheet(page, "Test Bank connection");
    await pz.sheet(page).getByRole("button", { name: "Sync now" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced Test Bank: 1 new transaction(s)");
    await expect(acct(page, "Checking ••1234").locator("[data-col=balance]")).toHaveText("$2,395.00");
  });

  test("a bank that needs you to sign in again says so and reconnects", async ({ page, seed, login, server }) => {
    const s = await seed("basic");
    await login();
    await pz.accounts(page, s);
    await connect(page);
    await pz.sheet(page).getByRole("button", { name: "Import" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced Test Bank");
    await pz.bank(server, { disconnected: true });
    await pz.openSheet(page, "Test Bank connection");
    await pz.sheet(page).getByRole("button", { name: "Sync now" }).click();
    await expect(page.locator("#toasts")).toContainText("The bank needs you to sign in again.");
    await expect(page.locator('li.bank-link[data-link="Test Bank"] [data-link-status=reconnect]')).toBeVisible();
    await pz.bank(server, { disconnected: false });
    await pz.openSheet(page, "Test Bank connection");
    await page.getByRole("button", { name: "Sign in again" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced Test Bank: 0 new transaction(s).");
    await expect(page.locator('li.bank-link[data-link="Test Bank"] [data-link-status=active]')).toBeVisible();
  });

  test("use accounts you already have, skip others, and disconnect", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    pz.acceptDialogs(page);
    await pz.accounts(page, s);
    await connect(page);
    const sh = pz.sheet(page);
    await sh.locator("#map-p_chk").selectOption({ label: "Use my Checking account" });
    await sh.locator("#map-p_card").selectOption({ label: "Don't import" });
    await sh.locator("#map-p_401k").selectOption({ label: "Don't import" });
    await sh.getByRole("button", { name: "Import" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced Test Bank");
    await expect(page.locator("li.account")).toHaveCount(4);
    await expect(acct(page, "Checking")).toContainText("Synced Sep 10");
    await expect(acct(page, "Checking").locator("[data-col=balance]")).toHaveText("$2,410.00");
    await expect(acct(page, "Sapphire ••9876")).toHaveCount(0);
    // Shell was already on Checking: matched, not duplicated.
    await pz.transactions(page, s);
    await expect(page.locator("#tx-list li.tx", { hasText: "Shell" })).toHaveCount(1);
    await pz.accounts(page, s);
    await pz.openSheet(page, "Test Bank connection");
    await pz.sheet(page).getByRole("button", { name: "Disconnect" }).click();
    await expect(page.locator("#bank-links")).toHaveCount(0);
    await expect(acct(page, "Checking")).toContainText("Checked Sep 10");
  });

  test("settings show bank sync status", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    await page.goto("/settings");
    await waitForContent(page);
    await expect(page.locator('[data-provider="teller"]')).toHaveCount(0);
    await expect(page.locator('[data-provider="plaid"]')).toContainText("On · sandbox");
    await expect(page.locator('[data-provider="plaid"]')).toContainText("Set by the server's environment variables.");
    await expect(page.locator('[data-provider="simplefin"]')).toContainText("Always available.");
  });

  test("SimpleFIN: paste a setup token", async ({ page, seed, login, server }) => {
    const s = await seed("basic");
    await login();
    await pz.accounts(page, s);
    await pz.openSheet(page, "Connect a bank");
    const sh = pz.sheet(page);
    await sh.getByText("Paste a SimpleFIN setup token").click();
    await sh.getByLabel("Setup token").fill("not a token");
    await sh.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.locator("#toasts")).toContainText("doesn't look like a SimpleFIN setup token");
    const token = Buffer.from(`${server.url}/__test/simplefin/claim/abc`).toString("base64");
    await sh.getByLabel("Setup token").fill(token);
    await sh.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(sh.getByRole("heading", { name: "Accounts at SF Credit Union" })).toBeVisible();
    // No account types from SimpleFIN: the card is recognised by its name.
    await expect(sh.locator("#map-sf_card")).toContainText("Add as a new Credit card account");
    await sh.getByRole("button", { name: "Import" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced SF Credit Union: 2 new transaction(s)");
    await expect(acct(page, "SF Checking").locator("[data-col=balance]")).toHaveText("$980.25");
    await expect(acct(page, "SF Visa Card").locator("[data-col=owed]")).toHaveText("$310.40");
    await pz.transactions(page, s);
    await expect(page.locator("#tx-list li.tx", { hasText: "Chipotle" })).toContainText("SF Checking");
  });

  test("Plaid: retirement accounts are recognized and pending charges wait", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.accounts(page, s);
    await connect(page);
    const sh = pz.sheet(page);
    await expect(sh.locator("#map-p_401k")).toContainText("Add as a new Retirement account");
    await expect(sh.locator("#map-p_card")).toContainText("Add as a new Credit card account");
    await sh.getByRole("button", { name: "Import" }).click();
    await expect(page.locator("#toasts")).toContainText("Synced Test Bank");
    await expect(acct(page, "401k ••1111").locator("[data-col=balance]")).toHaveText("$52,000.00");
    await expect(page.locator("#invested-accounts")).toContainText("401k ••1111");
    await pz.transactions(page, s);
    await expect(page.locator("#tx-list li.tx", { hasText: "Netflix" })).toHaveCount(0);
  });
});
