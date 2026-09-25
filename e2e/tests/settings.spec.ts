import { test, expect, pz, waitForContent } from "./fixtures";

test("changing currency converts every amount with the given rate", async ({ page, seed, login }) => {
  const s = await seed("balanced");
  await login();
  pz.acceptDialogs(page);
  await page.goto("/settings");
  await waitForContent(page);
  await expect(page.getByText("Your currency is USD.")).toBeVisible();
  await page.getByLabel("New currency").selectOption("EUR");
  await page.getByLabel("Rate: 1 USD equals").fill("0.5");
  await page.getByRole("button", { name: "Convert all amounts" }).click();
  await expect(page).toHaveURL(/\/settings\?saved=currency$/);
  await waitForContent(page);
  await expect(page.getByText("Your currency is EUR.")).toBeVisible();
  await pz.paycheck(page, s, 0);
  await expect(page.locator('[data-stat="planned"]')).toHaveText("€1,000.00");
  await expect(pz.sts(page)).toHaveText("-€48.60");
  await expect(page.locator("#zero-status")).toContainText("Every dollar has a job");
});

test("a bad exchange rate is refused", async ({ page, seed, login }) => {
  await seed("basic");
  await login();
  pz.acceptDialogs(page);
  await page.goto("/settings");
  await waitForContent(page);
  await page.getByLabel("New currency").selectOption("GBP");
  await page.getByLabel("Rate: 1 USD equals").fill("-2");
  await page.getByRole("button", { name: "Convert all amounts" }).click();
  await expect(pz.toast(page)).toContainText("Enter a positive exchange rate like 0.92.");
});

test("timezone drives today (and the default paycheck)", async ({ page, seed, login }) => {
  await seed("basic");
  await login();
  await page.goto("/settings");
  await waitForContent(page);
  await expect(page.getByText("Today is Thursday, September 10, 2026 in your timezone.")).toBeVisible();
  await page.getByRole("combobox", { name: "Timezone" }).selectOption("Europe/Paris");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(pz.toast(page)).toContainText("Timezone saved.");
  await expect(page.getByRole("combobox", { name: "Timezone" })).toHaveValue("Europe/Paris");
});
