import { test, expect, pz, waitForContent } from "./fixtures";
import path from "node:path";

const data = (f: string) => path.join(__dirname, "data", f);

test.describe("Settings → Bank providers", () => {
  test.beforeEach(async ({ seed, login }) => {
    await seed("basic", { no_env_providers: true });
    await login();
  });

  test("turn Teller on from the app and connect a bank with it", async ({ page }) => {
    // With nothing set up, only SimpleFIN is offered, with a pointer to Settings.
    await page.goto("/");
    await waitForContent(page);
    await page.getByRole("link", { name: "Accounts", exact: true }).click();
    await waitForContent(page);
    await pz.openSheet(page, "Connect a bank");
    await expect(page.getByRole("button", { name: "Continue with Teller" })).toHaveCount(0);
    await pz.sheet(page).getByRole("link", { name: "Turn them on in Settings" }).click();
    await waitForContent(page);
    const teller = page.locator('[data-provider="teller"]');
    await expect(teller).toContainText("Off");

    const form = page.locator("#teller-settings");
    await form.getByLabel("Application ID").fill("app_test");
    await form.getByLabel("Environment").selectOption("development");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#toasts")).toContainText("Outside sandbox, Teller needs both the certificate and the private key.");

    await form.getByLabel("Certificate (certificate.pem)", { exact: true }).setInputFiles(data("not-a-cert.txt"));
    await expect(page.locator("#toasts")).toContainText("isn't a PEM certificate");
    await form.getByLabel("Certificate (certificate.pem)", { exact: true }).setInputFiles(data("certificate.pem"));
    await form.getByLabel("Private key (private_key.pem)", { exact: true }).setInputFiles(data("private_key.pem"));
    await expect(form.locator('[data-pem-name="ts-cert"]')).toHaveText("✓ certificate.pem");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#toasts")).toContainText("Teller is on.");
    await expect(teller).toContainText("On · development");
    await page.reload();
    await waitForContent(page);
    await expect(page.locator('[data-provider="teller"]')).toContainText("On · development");

    // It works straight away, no restart.
    await page.getByRole("link", { name: "Budget", exact: true }).click();
    await waitForContent(page);
    await page.getByRole("link", { name: "Accounts", exact: true }).click();
    await waitForContent(page);
    await pz.openSheet(page, "Connect a bank");
    await page.getByRole("button", { name: "Continue with Teller" }).click();
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
