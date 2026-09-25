import { test, expect, pz, waitForContent } from "./fixtures";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { Download } from "@playwright/test";

/** Saves a download locally (works for local and remote browsers). */
async function save(dl: Download) {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "pz-dl-")), dl.suggestedFilename());
  await dl.saveAs(file);
  return file;
}

test("CSV export is complete and deterministic", async ({ page, seed, login }) => {
  const s = await seed("basic");
  await login();
  await pz.overview(page, s);
  const read = async () => {
    const [dl] = await Promise.all([page.waitForEvent("download"), page.getByRole("link", { name: "Download CSV" }).click()]);
    expect(dl.suggestedFilename()).toBe("paycheckzero-2026-09.csv");
    return fs.readFileSync(await save(dl), "utf8");
  };
  const a = await read();
  const b = await read();
  expect(a).toBe(b);
  const lines = a.trim().split("\r\n");
  expect(lines[0]).toBe("record_type,month,date,income_line,category,expense_line,payee,notes,status,schedule,planned,actual,spent,remaining,amount,current_balance,minimum_payment,split_group");
  expect(lines[1]).toBe("month,2026-09,,,,,,,draft,,4000.00,,125.20,1875.00,2125.00,,,");
  expect(a).toContain("paycheck,2026-09,2026-09-04,Acme Payroll,,,,,planned,,2000.00,,,388.00,1600.00,,,");
  expect(a).toContain("expense_line,2026-09,,,Debt,Visa,,,,,75.00,,0.00,75.00,,3200.00,75.00,");
  expect(a).toContain("allocation,2026-09,2026-09-18,Acme Payroll,Food,Groceries,,,,,,,,,300.00,,,");
  expect(a).toContain("transaction,2026-09,2026-09-05,Acme Payroll 2026-09-04,Food,Groceries,Trader Joe's,,,,,,,,-85.20,,,");
  expect(lines.filter((l) => l.startsWith("transaction,"))).toHaveLength(3);
});

test("full snapshot downloads and restores", async ({ page, seed, login }) => {
  const s = await seed("basic");
  await login();
  await pz.overview(page, s);
  const [dl] = await Promise.all([page.waitForEvent("download"), page.getByRole("link", { name: "Download full snapshot (JSON)" }).click()]);
  const file = await save(dl);
  const snap = JSON.parse(fs.readFileSync(file, "utf8"));
  expect(snap.format).toBe("paycheckzero-month-snapshot");
  expect(snap.month.allocations).toHaveLength(6);
  // Permanently delete, then restore from the backup.
  await page.goto("/months");
  await waitForContent(page);
  const card = page.locator('.month-card[data-month="2026-09"]');
  await card.getByText("Delete permanently").click();
  await card.getByLabel("Type 2026-09 to confirm").fill("2026-09");
  await card.getByRole("button", { name: "I understand — delete forever" }).click();
  await expect(card).toHaveCount(0);
  await page.getByLabel("Snapshot file").setInputFiles(file);
  await page.getByRole("button", { name: "Restore" }).click();
  await expect(page).toHaveURL(/\/paychecks\//);
  await waitForContent(page);
  await expect(pz.sts(page)).toHaveText("$388.00");
});

test("restoring over an existing month needs the replace option", async ({ page, seed, login }) => {
  const s = await seed("basic");
  await login();
  await pz.overview(page, s);
  const [dl] = await Promise.all([page.waitForEvent("download"), page.getByRole("link", { name: "Download full snapshot (JSON)" }).click()]);
  const file = await save(dl);
  await page.goto("/months");
  await waitForContent(page);
  await page.getByLabel("Snapshot file").setInputFiles(file);
  await page.getByRole("button", { name: "Restore" }).click();
  await expect(pz.toast(page)).toContainText("You already have a budget for that month.");
  await page.getByLabel("Replace the month if it already exists").check();
  await page.getByRole("button", { name: "Restore" }).click();
  await expect(page).toHaveURL(/\/paychecks\//);
});
