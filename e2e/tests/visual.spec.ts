import { test, expect } from '@playwright/test';
import { login } from './helpers';

test.describe('visual regression', () => {
  test('login page matches baseline', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('#auth-view')).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('login.png');
  });

  test('dashboard month view matches baseline', async ({ page }) => {
    await login(page);
    await expect(page.locator('#month-view')).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await expect(page).toHaveScreenshot('dashboard.png');
  });
});