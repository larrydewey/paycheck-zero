import { test, expect, pz, waitForContent } from "./fixtures";

test.describe("months", () => {
  test("empty state guides creating the first month", async ({ page, seed, login }) => {
    await seed("user");
    await login();
    await expect(page).toHaveURL(/\/months$/);
    await expect(page.getByRole("heading", { name: "No months yet" })).toBeVisible();
    await page.getByRole("button", { name: "New month" }).first().click();
    await expect(page.getByLabel("Copy structure only")).toBeDisabled();
    await page.getByLabel("Month", { exact: true }).fill("2026-09");
    await page.getByRole("button", { name: "Create month" }).click();
    await expect(page).toHaveURL(/\/income$/);
    await waitForContent(page);
    await expect(page.getByRole("heading", { name: "No income yet" })).toBeVisible();
  });

  test("any past or future month can be created and worked in", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    for (const ym of ["2019-02", "2031-12"]) {
      await page.goto("/months");
      await waitForContent(page);
      await page.getByRole("button", { name: "New month" }).first().click();
      await page.getByLabel("Month", { exact: true }).fill(ym);
      await page.getByRole("button", { name: "Create month" }).click();
      await expect(page).toHaveURL(/\/income$/);
      await waitForContent(page);
      await page.getByLabel("Name").fill("Pay");
      await page.getByLabel("Take-home per paycheck").fill("100");
      await page.getByLabel("How often").selectOption("monthly");
      await page.getByLabel("Days of the month").fill("31");
      await page.getByRole("button", { name: "Add income" }).click();
      await expect(page).toHaveURL(/\/paychecks\//);
      await waitForContent(page);
      await expect(pz.sts(page)).toHaveText("$100.00");
    }
    await page.goto("/months");
    await waitForContent(page);
    await expect(page.locator(".month-card")).toHaveCount(3);
    await expect(page.locator('.month-card[data-month="2019-02"]')).toContainText("$100.00 to assign");
  });

  test("duplicate month is refused with a clear message", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    await page.goto("/months");
    await waitForContent(page);
    await page.getByRole("button", { name: "New month" }).first().click();
    await page.getByLabel("Month", { exact: true }).fill("2026-09");
    await page.getByRole("button", { name: "Create month" }).click();
    await expect(pz.toast(page)).toContainText("You already have a budget for that month.");
  });

  test("copy structure only: lines copied, nothing planned", async ({ page, seed, login }) => {
    await seed("basic");
    await login();
    await page.goto("/months");
    await waitForContent(page);
    await page.getByRole("button", { name: "New month" }).first().click();
    await page.getByLabel("Month", { exact: true }).fill("2026-10");
    await page.getByLabel("Copy structure only").check();
    await expect(page.getByLabel("Copy from")).toHaveValue(/.+/);
    await page.getByRole("button", { name: "Create month" }).click();
    await expect(page).toHaveURL(/\/income$/);
    await page.getByRole("link", { name: "Budget", exact: true }).click();
    await waitForContent(page);
    await expect(pz.line(page, "Rent")).toBeVisible();
    await expect(page.getByLabel("Total planned for Rent")).toHaveValue("0.00");
    await expect(pz.line(page, "Rent")).not.toContainText("Target");
  });

  test("month switcher moves between months and offers to create missing ones", async ({ page, seed, login }) => {
    await seed("history");
    await login();
    await page.getByRole("link", { name: /Previous month \(August 2026\)/ }).click();
    await waitForContent(page);
    await expect(page.locator(".month-name")).toHaveText("August 2026");
    await page.getByRole("link", { name: /Next month \(September 2026\)/ }).click();
    await waitForContent(page);
    await page.getByRole("link", { name: /Next month \(October 2026\)/ }).click();
    await expect(page).toHaveURL(/\/months\?new=2026-10$/);
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page.getByLabel("Month", { exact: true })).toHaveValue("2026-10");
  });

  test("archive hides a month; restore brings it back", async ({ page, seed, login }) => {
    await seed("history");
    await login();
    await page.goto("/months");
    await waitForContent(page);
    const aug = page.locator('.month-card[data-month="2026-08"]');
    await aug.getByRole("button", { name: /Archive/ }).click();
    await expect(pz.toast(page)).toContainText("Month archived");
    await expect(aug).toHaveCount(0);
    await page.getByRole("link", { name: "Show archived months" }).click();
    await waitForContent(page);
    await expect(aug).toContainText("archived");
    await aug.getByRole("button", { name: /Restore/ }).click();
    await expect(pz.toast(page)).toContainText("Month restored.");
    await page.goto("/months");
    await waitForContent(page);
    await expect(aug).toBeVisible();
  });

  test("permanent delete needs the typed confirmation", async ({ page, seed, login }) => {
    await seed("history");
    await login();
    await page.goto("/months");
    await waitForContent(page);
    const old = page.locator('.month-card[data-month="2025-09"]');
    await old.getByText("Delete permanently").click();
    await expect(old).toContainText("cannot be undone");
    await old.getByLabel("Type 2025-09 to confirm").fill("2025-9");
    await old.getByRole("button", { name: "I understand — delete forever" }).click();
    await expect(pz.toast(page)).toContainText("Type the month exactly as shown");
    await old.getByText("Delete permanently").click();
    await old.getByLabel("Type 2025-09 to confirm").fill("2025-09");
    await old.getByRole("button", { name: "I understand — delete forever" }).click();
    await expect(pz.toast(page)).toContainText("Month permanently deleted.");
    await expect(old).toHaveCount(0);
  });

  test("the last month worked in is remembered", async ({ page, seed, login }) => {
    const s = await seed("history");
    await login();
    await pz.overview(page, s, "2026-08");
    await page.goto("/");
    await expect(page).toHaveURL(new RegExp(`/months/${s.months["2026-08"].id}/paychecks/`));
  });
});
