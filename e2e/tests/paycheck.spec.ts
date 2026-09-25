import { test, expect } from '@playwright/test';
import { login } from './helpers';

/**
 * User story (spec §14.5): edit a paycheck, re-allocate, add an expense line,
 * spend against it, reach zero, and lock the month. Safe-to-spend must recompute
 * live after every inline edit.
 */
test.describe('paycheck worksheet', () => {
  test('full zero-to-lock flow with live safe-to-spend', async ({ page }) => {
    await login(page);

    // Seeded July: planned 90000, Rent allocated 50000, spent 4000.
    await page.locator('#month-view').getByLabel('Planned amount for Salary').fill('95000');
    await page.locator('#month-view').getByLabel('Planned amount for Salary').blur();
    await expect(page.locator('#month-view')).toContainText('$450.00'); // 95000 - 50000

    await page.locator('#month-view').getByLabel('Amount for Rent').fill('60000');
    await page.locator('#month-view').getByLabel('Amount for Rent').blur();
    await expect(page.locator('#month-view')).toContainText('$350.00'); // 95000 - 60000
    await expect(page.locator('#summary-cards')).toContainText('$600.00'); // planned expenses

    // Add a new expense line from the UI, then allocate the remainder to it.
    await page.locator('#add-line summary').click();
    const lineForm = page.locator('#add-line form');
    await lineForm.locator('input[name="name"]').fill('Internet');
    await lineForm.locator('select[name="category_id"]').selectOption({ label: 'Housing' });
    await lineForm.locator('button', { hasText: 'Add expense line' }).click();

    const internet = page.locator('#month-view').getByLabel('Amount for Internet');
    await expect(internet).toBeVisible();
    await internet.fill('35000');
    await internet.blur();
    await expect(page.locator('#month-view')).toContainText('$0.00'); // fully allocated

    // Lock: month must be zero; badge flips to locked.
    await page.locator('#month-actions button', { hasText: 'Lock month' }).click();
    await expect(page.locator('#month-view h3')).toContainText('locked');
  });

  test('adding a transaction updates a category live (negative = expense)', async ({ page }) => {
    await login(page);

    await page.locator('#add-tx summary').click();
    const form = page.locator('#add-tx form');
    await form.locator('select[name="expense_line_id"]').selectOption({ label: 'Rent' });
    await form.locator('input[name="amount"]').fill('-2500');
    await form.locator('input[name="date"]').fill('2026-07-05');
    await form.locator('button', { hasText: 'Add transaction' }).click();

    // Housing spent: 4000 seeded + 2500 = 6500.
    await expect(page.locator('#summary-cards')).toContainText('$65.00');
    await expect(page.locator('#month-view')).toContainText('$65.00');
  });
});