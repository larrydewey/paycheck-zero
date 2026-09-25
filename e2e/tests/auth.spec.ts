import { test, expect } from '@playwright/test';
import { EMAIL, PASSWORD, login, randomEmail } from './helpers';

test.describe('auth', () => {
  test('shows login when signed out and restores session on reload', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('#auth-view')).toBeVisible();

    await login(page);
    await expect(page.locator('#dashboard')).toBeVisible();

    await page.reload();
    // Tokens persist -> session restores without re-entering credentials.
    await expect(page.locator('#dashboard')).toBeVisible();
    const access = await page.evaluate(() => localStorage.getItem('pz.access'));
    expect(access).toBeTruthy();
  });

  test('registers a new account through the UI and lands on the dashboard', async ({ page }) => {
    const email = randomEmail();
    await page.goto('/');
    const form = page.locator('#auth-view form').nth(1);
    await form.locator('input[name="email"]').fill(email);
    await form.locator('input[name="password"]').fill(PASSWORD);
    await form.locator('button[type="submit"]').click();
    await expect(page.locator('#dashboard')).toBeVisible();
    const stored = await page.evaluate(() => localStorage.getItem('pz.access'));
    expect(stored).toBeTruthy();
  });

  test('logs out and clears tokens', async ({ page }) => {
    await login(page);
    await page.locator('#dashboard button', { hasText: 'Sign out' }).click();
    await expect(page.locator('#auth-view')).toBeVisible();
    const tokens = await page.evaluate(
      () => [localStorage.getItem('pz.access'), localStorage.getItem('pz.refresh')],
    );
    expect(tokens[0]).toBeNull();
    expect(tokens[1]).toBeNull();
  });

  test('rejects wrong password with an inline error', async ({ page }) => {
    await page.goto('/');
    const form = page.locator('#auth-view form').first();
    await form.locator('input[name="email"]').fill(EMAIL);
    await form.locator('input[name="password"]').fill('wrongpass');
    await form.locator('button[type="submit"]').click();
    await expect(page.locator('#auth-error')).toContainText(/invalid|failed|wrong/i);
  });
});