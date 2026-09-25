import { test, expect, pz, waitForContent } from "./fixtures";

test.describe("offline & sync", () => {
  test("offline banner and queued transaction that syncs on reconnect", async ({ page, context, seed, login }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    await context.setOffline(true);
    await expect(page.locator("#offline-banner")).toBeVisible();
    const form = page.locator("#add-tx");
    await form.getByLabel("Amount").fill("7.25");
    await form.getByLabel("Payee").fill("Bakery");
    await form.getByLabel("Expense line").selectOption({ label: "Groceries" });
    await form.getByRole("button", { name: "Save transaction" }).click();
    await expect(page.locator("tr.pending", { hasText: "Bakery" })).toContainText("Pending sync");
    await expect(page.locator("#sync-banner")).toContainText("1 pending sync");
    await context.setOffline(false);
    await page.waitForURL(/transactions/);
    await waitForContent(page);
    await expect(page.locator("#tx-table tr", { hasText: "Bakery" })).not.toHaveClass(/pending/);
    await expect(page.locator("#toasts")).toContainText("1 offline change(s) synced.");
    await expect(page.locator("#sync-banner")).toBeHidden();
  });

  test("a conflicting offline edit is shown and can be resolved", async ({ page, context, seed, login, server }) => {
    const s = await seed("basic");
    await login();
    await pz.transactions(page, s);
    await context.setOffline(true);
    const row = page.locator("#tx-table tr", { hasText: "Shell" });
    await row.getByText("Edit").click();
    await row.getByLabel("Amount").fill("41");
    await row.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("#sync-banner")).toContainText("1 pending sync");
    // Meanwhile another device changes the same transaction.
    const token = await pz.apiToken(server);
    const txId = await row.getAttribute("data-tx");
    const r = await fetch(`${server.url}/api/v1/transactions/${txId}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify({ amount: -4500 }),
    });
    expect(r.ok).toBeTruthy();
    await context.setOffline(false);
    await page.waitForLoadState("load");
    await waitForContent(page);
    await expect(page.locator("#sync-banner")).toContainText("1 offline change(s) conflict with newer data.");
    await page.getByRole("button", { name: "Review" }).click();
    const dialog = page.getByRole("dialog", { name: "Changes that need your decision" });
    await expect(dialog).toContainText("Your offline change: 2026-09-06 · Shell · -$41.00");
    await expect(dialog).toContainText("Current version: 2026-09-06 · Shell · -$45.00");
    await dialog.getByRole("button", { name: "Keep mine" }).click();
    await page.waitForLoadState("load");
    await waitForContent(page);
    await expect(page.locator("#tx-table tr", { hasText: "Shell" })).toContainText("-$41.00");
    await expect(page.locator("#sync-banner")).toBeHidden();
  });

  test("pages viewed before going offline stay viewable", async ({ page, context, seed, login, browserName }) => {
    const s = await seed("basic");
    await login();
    await pz.overview(page, s);
    await page.evaluate(() => navigator.serviceWorker.ready);
    await page.reload();
    await waitForContent(page);
    if (browserName === "webkit") {
      // Playwright's WebKit offline emulation also blocks service-worker cache
      // hits, so assert offline readiness: the worker controls the page and
      // both the page shell and its content are cached.
      const ready = await page.evaluate(async (id) => {
        const cache = await caches.open("pz-pages-v1");
        const urls = (await cache.keys()).map((r) => new URL(r.url).pathname);
        return { controlled: !!navigator.serviceWorker.controller, urls };
      }, s.months["2026-09"].id);
      expect(ready.controlled).toBe(true);
      expect(ready.urls).toContain(`/months/${s.months["2026-09"].id}/overview`);
      expect(ready.urls).toContain(`/months/${s.months["2026-09"].id}/overview/content`);
      return;
    }
    await context.setOffline(true);
    await page.reload();
    await waitForContent(page);
    await expect(page.getByRole("heading", { name: "September 2026 overview" })).toBeVisible();
    await expect(page.locator("#offline-banner")).toBeVisible();
  });
});
