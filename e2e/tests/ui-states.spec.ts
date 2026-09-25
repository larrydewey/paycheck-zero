import { test, expect, pz } from "./fixtures";

test.describe("UI states", () => {
  // page.route must see the content fetches, so keep the service worker out of the way.
  test.use({ serviceWorkers: "block" });
  test("skeleton loader shows while content loads", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await page.route(/\/overview\/content/, async (route) => {
      await new Promise((r) => setTimeout(r, 800));
      await route.continue();
    });
    await page.goto(`/months/${s.months["2026-09"].id}/overview`);
    await expect(page.locator("main#content .skeleton")).toBeVisible();
    await expect(page.locator("main#content[aria-busy=true]")).toBeAttached();
    await expect(page.getByRole("heading", { name: "September 2026 overview" })).toBeVisible();
    await expect(page.locator("main#content .skeleton")).toHaveCount(0);
  });

  test("a failed load shows a recoverable error with retry", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    let fail = true;
    await page.route(/\/overview\/content/, async (route) => {
      if (fail) await route.fulfill({ status: 500, body: "boom" });
      else await route.continue();
    });
    await page.goto(`/months/${s.months["2026-09"].id}/overview`);
    await expect(page.getByRole("heading", { name: "We couldn't load this page" })).toBeVisible();
    fail = false;
    await page.getByRole("button", { name: "Try again" }).click();
    await expect(page.getByRole("heading", { name: "September 2026 overview" })).toBeVisible();
  });

  test("a new paycheck shows every line ready to fund", async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.income(page, s);
    const add = page.locator("section.add-income");
    await add.getByLabel("Name").fill("Gift");
    await add.getByLabel("Amount per paycheck").fill("50");
    await add.getByLabel("Schedule").selectOption("one_off");
    await add.getByLabel("Pay date").fill("2026-09-20");
    await add.getByRole("button", { name: "Add income" }).click();
    await page.locator("section.income-line", { hasText: "Gift" }).getByRole("link", { name: "Sep 20" }).click();
    await expect(page.getByLabel("Planned for Rent from this paycheck")).toHaveValue("0.00");
    await expect(page.getByRole("heading", { name: "Assign money from this paycheck" })).toBeVisible();
  });
});
