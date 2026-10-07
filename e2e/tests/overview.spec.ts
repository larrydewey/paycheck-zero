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
    await expect(page.locator('[data-card="spent"]')).toContainText("$125.20");
    await expect(page.locator("#zero-status")).toContainText("$1,875.00 left to assign");
    await expect(page.locator('[data-card="left"]')).toContainText("Left to budget");
    await expect(page.locator('[data-card="left"]')).toContainText("$1,875.00");
  });

  test("editing Planned here updates allocations and summaries live", async ({ page }) => {
    const input = page.getByLabel("Total planned for Rent");
    await input.fill("1700");
    await input.press("Enter");
    await expect(pz.category(page, "Housing").locator("summary [data-col=planned]")).toContainText("$1,850.00");
    await expect(page.locator("#zero-status")).toContainText("$1,375.00 left");
    // Earliest paycheck with free money (Sep 4 had $400) is used first.
    await pz.lineSheet(page, "Rent");
    await expect(pz.sheet(page).getByLabel("Rent from the Sep 4 paycheck")).toHaveValue("$1,600.00");
    await expect(pz.sheet(page).getByLabel("Rent from the Sep 18 paycheck")).toHaveValue("$100.00");
  });

  test("each line's sheet edits what every paycheck puts in (zeros included)", async ({ page }) => {
    await pz.lineSheet(page, "Rent");
    const sh = pz.sheet(page);
    await expect(sh.getByText("Funded by")).toBeVisible();
    const sep4 = sh.getByLabel("Rent from the Sep 4 paycheck");
    const sep18 = sh.getByLabel("Rent from the Sep 18 paycheck");
    await expect(sep4).toHaveValue("$1,200.00");
    await expect(sep18).toHaveValue("$0.00");
    await sep18.fill("100");
    await sep18.press("Enter");
    // The sheet stays open with fresh numbers, ready for the next paycheck.
    await expect(page.locator("#sheet[open]")).toBeVisible();
    await expect(sh.locator("[data-col=planned]")).toHaveText("$1,300.00");
    await expect(sep18).toHaveValue("$100.00");
    await expect(pz.category(page, "Housing").locator(":scope > summary [data-col=planned]")).toContainText("$1,450.00");
    await expect(pz.line(page, "Rent").locator("[data-col=planned]")).toHaveText("$1,300.00");
  });

  test("move a line or category to any spot, in one step", async ({ page }) => {
    const names = (cat: string) => pz.category(page, cat).locator("li.line").evaluateAll((els) => els.map((e) => e.getAttribute("data-line")));
    expect(await names("Housing")).toEqual(["Rent", "Electric"]);
    // Within its category.
    await pz.lineSheet(page, "Electric");
    let sh = pz.sheet(page);
    await sh.getByRole("combobox", { name: "Position" }).selectOption({ label: "At the top" });
    await sh.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => names("Housing")).toEqual(["Electric", "Rent"]);
    // Into a chosen spot in another category.
    await pz.lineSheet(page, "Rent");
    sh = pz.sheet(page);
    await sh.getByLabel("Category", { exact: true }).selectOption({ label: "Food" });
    await sh.getByRole("combobox", { name: "Position" }).selectOption({ label: "At the top" });
    await sh.getByRole("button", { name: "Save", exact: true }).click();
    await expect.poll(() => names("Food")).toEqual(["Rent", "Groceries"]);
    expect(await names("Housing")).toEqual(["Electric"]);
    // A category jumps several places at once.
    const cats = () => page.locator("details.category").evaluateAll((els) => els.map((e) => e.getAttribute("data-category")));
    expect((await cats()).indexOf("Debt")).toBeGreaterThan(2);
    await pz.openSheet(page, "Edit Debt");
    sh = pz.sheet(page);
    await sh.getByRole("combobox", { name: "Position" }).selectOption({ label: "After Saving" });
    await sh.getByRole("button", { name: "Move", exact: true }).click();
    await expect.poll(async () => (await cats()).slice(0, 2)).toEqual(["Saving", "Debt"]);
  });

  test("adding a line from the month view can fund it from a chosen paycheck", async ({ page }) => {
    await pz.openSheet(page, "New line", { exact: true });
    const sh = pz.sheet(page);
    await sh.getByLabel("Line name").fill("Dining out");
    await sh.getByLabel("Category", { exact: true }).selectOption({ label: "Food" });
    await sh.getByLabel("Amount to plan").fill("60");
    await sh.getByLabel("From paycheck").selectOption({ label: "Sep 18 ($1,475.00 left)" });
    await sh.getByRole("button", { name: "Add line" }).click();
    await expect(sh).toBeHidden();
    await expect(pz.line(page, "Dining out").locator("[data-col=planned]")).toHaveText("$60.00");
    await pz.lineSheet(page, "Dining out");
    await expect(sh.getByLabel("Dining out from the Sep 18 paycheck")).toHaveValue("$60.00");
  });

  test("quick add a line by name inside a category", async ({ page }) => {
    const input = page.getByLabel("New line in Food", { exact: true });
    await input.fill("Snacks");
    await pz.category(page, "Food").getByRole("button", { name: "Add line" }).click();
    await expect(pz.line(page, "Snacks")).toBeVisible();
    await expect(input).toHaveValue("");
  });

  

  test("add, rename, reorder and delete categories and lines", async ({ page }) => {
    pz.acceptDialogs(page);
    const order = () => page.locator("details.category").evaluateAll((els) => els.map((e) => e.getAttribute("data-category")));
    await page.getByLabel("New category").fill("Pets");
    await page.getByRole("button", { name: "Add category" }).click();
    // A category with no lines waits as a chip until it gets its first line.
    await page.locator('[data-category-empty="Pets"]').click();
    const sh = pz.sheet(page);
    await expect(sh.getByLabel("Category", { exact: true })).toHaveValue(/.+/);
    await sh.getByLabel("Line name").fill("Vet");
    await sh.getByRole("button", { name: "Add line" }).click();
    await expect(sh).toBeHidden();
    const pets = pz.category(page, "Pets");
    await expect(pz.line(page, "Vet")).toBeVisible();
    await pz.openSheet(page, "Edit Pets");
    await sh.getByLabel("Category name").fill("Animals");
    await sh.getByRole("button", { name: "Save" }).click();
    await expect(sh).toBeHidden();
    const animals = pz.category(page, "Animals");
    await expect(animals).toBeVisible();
    await expect(pets).toHaveCount(0);
    expect((await order()).slice(-1)).toEqual(["Animals"]);
    await pz.openSheet(page, "Edit Animals");
    await sh.getByRole("button", { name: "Move Animals up" }).click();
    await expect.poll(async () => (await order()).indexOf("Animals")).toBeLessThan((await order()).length - 1);
    await pz.lineSheet(page, "Vet");
    await sh.getByRole("button", { name: "Delete Vet" }).click();
    await expect(pz.line(page, "Vet")).toHaveCount(0);
    await expect(page.locator('[data-category-empty="Animals"]')).toBeVisible();
    await page.locator('[data-category-empty="Animals"]').waitFor();
  });

  test("categories can be deleted from their sheet", async ({ page }) => {
    pz.acceptDialogs(page);
    await pz.openSheet(page, "Edit Transportation");
    await pz.sheet(page).getByRole("button", { name: "Delete category Transportation" }).click();
    await expect(pz.category(page, "Transportation")).toHaveCount(0);
  });

  test("deleting a funded line un-assigns its money", async ({ page }) => {
    pz.acceptDialogs(page);
    await pz.lineSheet(page, "Rent");
    await pz.sheet(page).getByRole("button", { name: "Delete Rent" }).click();
    await expect(pz.toast(page)).toContainText("Deleted funding went back to its paychecks");
    await expect(page.locator("#zero-status")).toContainText("$3,075.00 left");
  });

  test("lines reorder and move between categories from their sheet", async ({ page }) => {
    const order = (cat: string) => pz.category(page, cat).locator("li.line").evaluateAll((els) => els.map((e) => e.getAttribute("data-line")));
    await pz.lineSheet(page, "Electric");
    await pz.sheet(page).getByRole("button", { name: "Move Electric up" }).click();
    await expect.poll(() => order("Housing")).toEqual(["Electric", "Rent"]);
    await pz.lineSheet(page, "Gas");
    await pz.sheet(page).getByLabel("Category", { exact: true }).selectOption({ label: "Food" });
    await pz.sheet(page).getByRole("button", { name: "Save" }).last().click();
    await expect.poll(() => order("Food")).toContain("Gas");
    await expect(pz.category(page, "Transportation")).toHaveCount(0);
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
    // The budget shows what's owed beside the plan, per line and per category.
    const visa = pz.line(page, "Visa").locator("[data-debt]");
    await expect(visa).toHaveText("$3,200.00 owed · $3,125.00 after this month · ~43 months to go");
    await expect(pz.category(page, "Debt").locator("[data-col=cat-owed]")).toHaveText("$3,200.00 owed · $3,125.00 after this month");
    await pz.lineSheet(page, "Visa");
    await expect(pz.sheet(page)).toContainText("paid off in about 43 months at this pace, before interest");
    const sh = pz.sheet(page);
    await expect(sh.getByLabel("Balance")).toHaveValue("$3,200.00");
    await sh.getByLabel("Minimum payment").fill("100");
    await sh.getByLabel("Minimum payment").press("Enter");
    await expect(sh).toBeHidden();
    await pz.lineSheet(page, "Visa");
    await expect(sh).toContainText("$25.00 below the minimum payment");
  });

  test("locking a month that isn't at zero is blocked with the exact difference", async ({ page }) => {
    await page.getByRole("button", { name: "Lock month" }).click();
    await expect(pz.toast(page)).toContainText("You can't lock yet: $1,875.00 is still unassigned.");
  });

  test("a line can be planned past the month's income, which locks no month", async ({ page }) => {
    const input = page.getByLabel("Total planned for Rent");
    // Income is $4,000 and $2,125 is planned; this asks for $2,125 more.
    await input.fill("4250");
    await input.press("Enter");
    await expect(input).toHaveValue("$4,250.00");

    // The summary swaps "Left to budget" for "Over budget".
    const left = page.locator('[data-card="left"]');
    await expect(left).toContainText("Over budget");
    await expect(left).toContainText("$1,175.00");
    await expect(left.locator(".stat-value")).toHaveClass(/neg/);
    await expect(page.locator("#zero-status")).toContainText("Assigned $1,175.00 more than this month's income");
    await expect(page.locator("#zero-status")).toHaveClass(/danger/);

    // Locking is off until the plan fits the income again.
    const lock = page.getByRole("button", { name: "Lock month" });
    await expect(lock).toBeDisabled();
    await expect(lock).toHaveAttribute("title", "Trim $1,175.00 off the plan to lock this month.");

    // Trim the same line back and the month is lockable again.
    await input.fill("3075");
    await input.press("Enter");
    await expect(page.locator("#zero-status")).toContainText("Every dollar has a job");
    await expect(left).toContainText("Left to budget");
    await expect(lock).toBeEnabled();
    await lock.click();
    await expect(pz.toast(page)).toContainText("Month locked.");
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
