import { test, expect, pz, waitForContent } from "./fixtures";

test.describe("Settings → Bank providers", () => {
  test.beforeEach(async ({ seed, login }) => {
    await seed("basic", { no_env_providers: true });
    await login();
  });

  test("with Plaid off, only SimpleFIN is offered, with a way to turn Plaid on", async ({ page }) => {
    await page.goto("/");
    await waitForContent(page);
    await page.getByRole("link", { name: "Accounts", exact: true }).click();
    await waitForContent(page);
    await pz.openSheet(page, "Connect a bank");
    await expect(page.getByRole("button", { name: "Continue with Plaid" })).toHaveCount(0);
    await expect(pz.sheet(page)).toContainText("SimpleFIN Bridge");
    await pz.sheet(page).getByRole("link", { name: "Turn it on in Settings" }).click();
    await waitForContent(page);
    await expect(page.locator('[data-provider="plaid"]')).toContainText("Off");
    await expect(page.locator('[data-provider="teller"]')).toHaveCount(0);
    const form = page.locator("#plaid-settings");
    await form.getByLabel("Client ID").fill("cid");
    await form.getByLabel("Secret").fill("sec");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#toasts")).toContainText("Plaid is on.");
    // Works straight away, no restart.
    await page.getByRole("link", { name: "Budget" }).first().click();
    await waitForContent(page);
    await page.getByRole("link", { name: "Accounts", exact: true }).click();
    await waitForContent(page);
    await pz.openSheet(page, "Connect a bank");
    await page.getByRole("button", { name: "Continue with Plaid" }).click();
    await expect(pz.sheet(page).getByRole("heading", { name: "Accounts at Test Bank" })).toBeVisible();
  });

  test("Plaid keys are checked before saving; secrets stay hidden", async ({ page }) => {
    pz.acceptDialogs(page);
    await page.goto("/settings");
    await waitForContent(page);
    const plaid = page.locator('[data-provider="plaid"]');
    const form = page.locator("#plaid-settings");
    await form.getByLabel("Client ID").fill("cid");
    await form.getByLabel("Secret").fill("wrong");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#toasts")).toContainText("Plaid didn't accept those keys.");
    await form.getByLabel("Secret").fill("sec");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#toasts")).toContainText("Plaid is on.");
    await expect(plaid).toContainText("On · sandbox");
    // The secret is never sent back; blank keeps it.
    await plaid.getByText("Change settings").click();
    await expect(form.getByLabel("Secret")).toHaveValue("");
    await expect(form.getByLabel("Secret")).toHaveAttribute("placeholder", "Saved (leave blank to keep)");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#toasts")).toContainText("Plaid is on.");
    // Turn it off again.
    await plaid.getByRole("button", { name: "Turn off" }).click();
    await expect(page.locator("#toasts")).toContainText("Provider turned off.");
    await expect(page.locator('[data-provider="plaid"]')).toContainText("Off");
  });
});
