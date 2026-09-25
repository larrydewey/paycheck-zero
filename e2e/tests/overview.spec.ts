import { test, expect, pz, waitForContent } from "./fixtures";

test.describe("monthly overview", () => {
  test.beforeEach(async ({ page, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.overview(page, s);
  });

  test("category headers summarise Planned / Spent / Remaining", async ({ page }) => {
    const food = pz.category(page, "Food").locator(":scope > summary");
    await expect(food.locator("[data-col=planned]")).toContainText("$600.00");
    await expect(food.locator("[data-col=spent]")).toContainText("$85.20");
    await expect(food.locator("[data-col=remaining]")).toContainText("$514.80");
    await expect(page.locator('[data-card="income"]')).toContainText("$4,000.00");
    await expect(page.locator('[data-card="expenses"]')).toContainText("$2,125.00");
    await expect(page.locator('[data-card="remaining"]')).toContainText("$1,875.00");
  });

  test("editing Planned here updates allocations and summaries live", async ({ page }) => {
    const input = page.getByLabel("Total planned for Rent");
    await input.fill("1700");
    await input.press("Enter");
    await expect(pz.category(page, "Housing").locator("summary [data-col=planned]")).toContainText("$1,850.00");
    // Earliest paycheck with free money (Sep 4 had $400) is used first.
    await expect(pz.line(page, "Rent")).toContainText("Sep 4: $1,600.00 · Sep 18: $100.00");
    await expect(page.locator("#zero-status")).toContainText("$1,375.00 left");
  });

  test("each line expands to edit what every paycheck puts in (zeros included)", async ({ page }) => {
    const rent = pz.line(page, "Rent");
    await rent.getByText("By paycheck (1 funding)").click();
    const sep4 = page.getByLabel("Rent from the Sep 4 paycheck");
    const sep18 = page.getByLabel("Rent from the Sep 18 paycheck");
    await expect(sep4).toHaveValue("1200.00");
    await expect(sep18).toHaveValue("0.00");
    await sep18.fill("100");
    await sep18.press("Enter");
    await expect(pz.category(page, "Housing").locator(":scope > summary [data-col=planned]")).toContainText("$1,450.00");
    // The panel stays open after the re-render.
    await expect(page.getByLabel("Rent from the Sep 18 paycheck")).toHaveValue("100.00");
    await expect(rent).toContainText("Sep 4: $1,200.00 · Sep 18: $100.00");
  });

  test("adding a line from the month view can fund it from a chosen paycheck", async ({ page }) => {
    const input = page.getByLabel("New line in Food", { exact: true });
    await input.fill("Dining out");
    await page.getByLabel("Amount for the new line in Food").fill("60");
    await page.getByLabel("Paycheck that funds the new line in Food").selectOption({ label: "from Sep 18 ($1,475.00 left)" });
    await pz.category(page, "Food").getByRole("button", { name: "Add line" }).click();
    await expect(pz.line(page, "Dining out")).toContainText("Sep 18: $60.00");
    await expect(input).toHaveValue("");
  });

  test("asking for more than all paychecks have is refused", async ({ page }) => {
    const input = page.getByLabel("Total planned for Rent");
    await page.evaluate(() => document.querySelectorAll("[data-max-cents]").forEach((e) => e.removeAttribute("data-max-cents")));
    await input.fill("5000");
    await input.press("Enter");
    await expect(pz.toast(page)).toContainText("you're $1,925.00 short");
    await expect(input).toHaveValue("1200.00");
  });

  test("add, rename, reorder and delete categories and lines", async ({ page }) => {
    pz.acceptDialogs(page);
    await page.getByLabel("New category").fill("Pets");
    await page.getByRole("button", { name: "Add category" }).click();
    const pets = pz.category(page, "Pets");
    await expect(pets).toBeVisible();
    await pets.getByLabel("New line in Pets", { exact: true }).fill("Vet");
    await pets.getByRole("button", { name: "Add line" }).click();
    await expect(pz.line(page, "Vet")).toBeVisible();
    await pets.getByText("Edit Pets").click();
    await pets.getByLabel("Name of category Pets").fill("Animals");
    await pets.getByLabel("Name of category Pets").press("Enter");
    const animals = pz.category(page, "Animals");
    await expect(animals).toBeVisible();
    const before = await page.locator("details.category").evaluateAll((els) => els.map((e) => e.getAttribute("data-category")));
    expect(before[before.length - 1]).toBe("Animals");
    await animals.getByText("Edit Animals").click();
    await animals.getByRole("button", { name: "Move Animals up" }).click();
    await expect.poll(async () => (await page.locator("details.category").evaluateAll((els) => els.map((e) => e.getAttribute("data-category")))).slice(-2)).toEqual(["Animals", "Other"]);
    await pz.line(page, "Vet").getByRole("button", { name: "Delete Vet" }).click();
    await expect(pz.line(page, "Vet")).toHaveCount(0);
    await animals.getByText("Edit Animals").click();
    await animals.getByRole("button", { name: "Delete category Animals" }).click();
    await expect(animals).toHaveCount(0);
  });

  test("deleting a funded line un-assigns its money", async ({ page }) => {
    pz.acceptDialogs(page);
    await pz.line(page, "Rent").getByRole("button", { name: "Delete Rent" }).click();
    await expect(pz.toast(page)).toContainText("Deleted funding went back to its paychecks");
    await expect(page.locator("#zero-status")).toContainText("$3,075.00 left");
  });

  test("lines reorder within a category", async ({ page }) => {
    await pz.line(page, "Electric").getByRole("button", { name: "Move Electric up" }).click();
    await expect.poll(async () => pz.category(page, "Housing").locator("li.line").evaluateAll((els) => els.map((e) => e.getAttribute("data-line")))).toEqual(["Electric", "Rent"]);
  });

  test("drag and drop reorders lines and moves them between categories", async ({ page, isMobile }) => {
    test.skip(isMobile, "drag handles are a mouse enhancement; phones use the move buttons");
    const order = (cat: string) => pz.category(page, cat).locator("li.line").evaluateAll((els) => els.map((e) => e.getAttribute("data-line")));
    await pz.line(page, "Electric").locator(".grip").dragTo(pz.line(page, "Rent"), { targetPosition: { x: 10, y: 2 } });
    await expect.poll(() => order("Housing")).toEqual(["Electric", "Rent"]);
    // Playwright's synthetic drag can't start a second drag in the same page
    // session after a re-render, so reload between drags.
    await page.reload();
    await pz.line(page, "Gas").waitFor();
    await pz.line(page, "Gas").locator(".grip").dragTo(pz.category(page, "Food").locator(":scope > summary"));
    await expect.poll(() => order("Food")).toEqual(["Gas", "Groceries"]);
    await expect(pz.category(page, "Transportation").locator("li.line")).toHaveCount(0);
  });

  test("collapse state is remembered across reloads", async ({ page }) => {
    const food = pz.category(page, "Food");
    await expect(food).toHaveAttribute("open", "");
    await food.locator(":scope > summary").click();
    await expect(food).not.toHaveAttribute("open", "");
    await page.reload();
    await waitForContent(page);
    await expect(pz.category(page, "Food")).not.toHaveAttribute("open", "");
    await expect(pz.category(page, "Housing")).toHaveAttribute("open", "");
  });

  test("debt lines carry balance and minimum payment", async ({ page }) => {
    const visa = pz.line(page, "Visa");
    await expect(visa.getByLabel("Balance")).toHaveValue("3200.00");
    await visa.getByLabel("Minimum payment").fill("100");
    await visa.getByLabel("Minimum payment").press("Enter");
    await expect(visa).toContainText("$25.00 below the minimum payment");
  });

  test("locking a month that isn't at zero is blocked with the exact difference", async ({ page }) => {
    await page.getByRole("button", { name: "Lock month" }).click();
    await expect(pz.toast(page)).toContainText("You can't lock yet: $1,875.00 is still unassigned.");
  });
});

test("a balanced month locks and becomes read-only", async ({ page, seed, login }) => {
  const s = await seed("balanced");
  await login();
  await pz.overview(page, s);
  await expect(page.locator("#zero-status")).toContainText("Every dollar has a job");
  await page.getByRole("button", { name: "Lock month" }).click();
  await expect(pz.toast(page)).toContainText("Month locked.");
  await expect(page.getByText("Locked — planning is frozen")).toBeVisible();
  await expect(page.getByLabel("Total planned for Rent")).toHaveCount(0);
  await expect(page.getByLabel("New category")).toHaveCount(0);
});
