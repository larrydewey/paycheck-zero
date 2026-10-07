import { test, expect, pz } from "./fixtures";

test.describe("transactions across months", () => {
  test("All months lists every month and a transaction can be counted in another month", async ({ page, seed, login }) => {
    const s = await seed("history");
    await login();
    await pz.transactions(page, s);
    await expect(page.locator("#tx-list")).not.toContainText("Landlord");
    await page.locator(".filter-chips").getByRole("link", { name: "All months" }).click();
    await expect(page).toHaveURL(/show=all-months$/);
    const row = page.locator("#tx-list li.tx", { hasText: "Landlord" });
    await expect(row).toBeVisible();
    await row.getByRole("button").click();
    const sheet = page.locator("#sheet[open]");
    await sheet.getByText("More options").click();
    await sheet.getByLabel("Count in month").selectOption({ label: "September 2026" });
    await sheet.getByRole("button", { name: "Move", exact: true }).click();
    await expect(pz.toast(page)).toContainText("Now counted in September 2026.");
    await expect(row).toContainText("Counted in September 2026");
    // The date stays in August; the line with the same name carries over.
    await pz.transactions(page, s);
    const moved = page.locator("#tx-list li.tx", { hasText: "Landlord" });
    await expect(moved).toContainText("Rent");
    await expect(page.locator("#tx-list")).toContainText("Sat, Aug 1");
  });

  test("a card payment marked as a transfer stops counting as spending", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    await pz.transactions(page, s);
    let form = await pz.addTx(page);
    await form.getByLabel("Amount", { exact: true }).fill("120");
    await form.getByLabel("Payee").fill("Visa payment");
    await form.getByLabel("Account").selectOption({ label: "Checking" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    form = await pz.addTx(page);
    await form.getByRole("radio", { name: "Income" }).check();
    await form.getByLabel("Amount", { exact: true }).fill("120");
    await form.getByLabel("Payee").fill("Payment received");
    await form.getByLabel("Account").selectOption({ label: "Visa" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(page.locator("#tx-list")).toContainText("Payment received");

    const sheet = await pz.editTx(page, /Visa payment/);
    await sheet.getByText("More options").click();
    await sheet.getByLabel("This is a payment to").selectOption({ label: "Visa" });
    await sheet.getByRole("button", { name: "Mark as transfer" }).click();
    await expect(pz.toast(page)).toContainText("Marked as a transfer. The same money arriving in the other account was merged into it.");
    await expect(page.locator("#tx-list li.tx.transfer", { hasText: "Visa payment" })).toBeVisible();
    await expect(page.locator("#tx-list")).not.toContainText("Payment received");
  });

  test("money back on a card linked to a line lowers its spending", async ({ page, seed, login }) => {
    const s = await seed("wallet");
    await login();
    await pz.overview(page, s);
    const spent = pz.category(page, "Food").locator(":scope > summary [data-col=spent]");
    await expect(spent).toContainText("$85.20");
    await pz.transactions(page, s);
    const form = await pz.addTx(page);
    await form.getByRole("radio", { name: "Income" }).check();
    await form.getByLabel("Amount", { exact: true }).fill("20");
    await form.getByLabel("Payee").fill("Grocery refund");
    await form.getByLabel("Account").selectOption({ label: "Visa" });
    await form.getByLabel("Expense line", { exact: true }).selectOption({ label: "Groceries" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(pz.toast(page)).toContainText("Transaction saved.");
    await pz.overview(page, s);
    await expect(spent).toContainText("$65.20");
  });

    test("long line pickers can be searched", async ({ page, seed, login, server }) => {
    const s = await seed("basic");
    const token = await pz.apiToken(server);
    const mid = s.months["2026-09"].id;
    const auth = { Authorization: `Bearer ${token}`, "Content-Type": "application/json" };
    const lines = (await (await fetch(`${server.url}/api/v1/months/${mid}/expense-lines`, { headers: auth })).json()) as { name: string; category_id: string }[];
    const food = lines.find((l) => l.name === "Groceries")!.category_id;
    for (const name of ["Coffee", "Restaurants", "Pet food"]) {
      await fetch(`${server.url}/api/v1/months/${mid}/expense-lines`, { method: "POST", headers: auth, body: JSON.stringify({ category_id: food, name }) });
    }
    await login();
    await pz.transactions(page, s);
    const form = await pz.addTx(page);
    const select = form.getByLabel("Expense line", { exact: true });
    await form.getByLabel("Search lines").fill("pet");
    await expect(select.locator("option")).toHaveText(["Not linked to a line", "Pet food"]);
    await expect(select.locator("option:checked")).toHaveText("Pet food");
    await form.getByLabel("Search lines").fill("");
    await expect(select.locator("option")).toHaveCount(10);
    await expect(select.locator("option:checked")).toHaveText("Pet food");
  });
});
