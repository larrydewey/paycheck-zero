import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { login } from './helpers';

test.describe('accessibility (axe)', () => {
  test('login page: no serious/critical violations', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('#auth-view')).toBeVisible();
    const results = await new AxeBuilder({ page }).analyze();
    const blockers = results.violations.filter((v) =>
      ['serious', 'critical'].includes(v.impact ?? ''),
    );
    expect(blockers).toEqual([]);
  });

  test('dashboard + month view: no serious/critical violations', async ({ page }) => {
    await login(page);
    await expect(page.locator('#month-view')).toBeVisible();
    const results = await new AxeBuilder({ page }).analyze();
    const blockers = results.violations.filter((v) =>
      ['serious', 'critical'].includes(v.impact ?? ''),
    );
    expect(blockers).toEqual([]);
  });
});