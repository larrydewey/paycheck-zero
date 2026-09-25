import { test, expect } from '@playwright/test';
import { login } from './helpers';

test.describe('months', () => {
  test('seeded months appear in the selector and switching merges the view', async ({ page }) => {
    await login(page);
    const selector = page.locator('#month-selector');
    await expect(selector.locator('button', { hasText: '2026-07' })).toBeVisible();
    await expect(selector.locator('button', { hasText: '2026-06' })).toBeVisible();

    await selector.locator('button', { hasText: '2026-06' }).click();
    await expect(page.locator('#month-view h3', { hasText: '2026-06' })).toBeVisible();

    await selector.locator('button', { hasText: '2026-07' }).click();
    await expect(page.locator('#month-view h3', { hasText: '2026-07' })).toBeVisible();
    await expect(page.locator('#month-view')).toContainText('$400.00'); // safe to spend
  });

  test('creates a new month from the UI', async ({ page }) => {
    await login(page);
    const form = page.locator('#month-selector form');
    await form.locator('input[name="year_month"]').fill('2026-09');
    await form.locator('button', { hasText: 'New month' }).click();
    // pzNewMonth reloads the shell; the dashboard re-renders with the new month.
    await expect(page.locator('#month-selector button', { hasText: '2026-09' })).toBeVisible();
    await expect(page.locator('#month-view h3', { hasText: '2026-09' })).toBeVisible();
  });
});