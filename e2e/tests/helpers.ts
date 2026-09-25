import { Page, expect } from '@playwright/test';

export const EMAIL = 'e2e@example.com';
export const PASSWORD = 'password123';

/** Sign in as the seeded user through the actual UI. */
export async function login(page: Page) {
  // Track session requests
  await page.addInitScript(() => {
    window._sessionResponses = [];
    const originalFetch = window.fetch;
    window.fetch = async (...args) => {
      const response = await originalFetch(...args);
      if (args[0] && typeof args[0] === 'string' && args[0].includes('/api/v1/ui/session')) {
        const text = await response.clone().text();
        window._sessionResponses.push({ url: args[0], status: response.status, body: text.slice(0, 500) });
      }
      return response;
    };
  });

  await page.goto('/');
  const form = page.locator('#auth-view form').first();
  await form.locator('input[name="email"]').fill(EMAIL);
  await form.locator('input[name="password"]').fill(PASSWORD);
  await form.locator('button[type="submit"]').click();
  // Wait for reload and dashboard
  await page.waitForLoadState('networkidle');
  await page.waitForTimeout(3000);
  
  // Check if datastar-ready fired and what actions ran
  const dsInfo = await page.evaluate(() => {
    return {
      dataSignals: document.getElementById('app')?.getAttribute('data-signals'),
      appHTML: document.getElementById('app')?.innerHTML.slice(0, 500),
    };
  });
  console.log('[login] dsInfo:', JSON.stringify(dsInfo));
  
  // Check session responses
  const sessionResponses = await page.evaluate(() => window._sessionResponses || []);
  console.log('[login] session responses:', JSON.stringify(sessionResponses));
  
  // Check console errors
  const errors = await page.evaluate(() => window._dsConsoleErrors || []);
  console.log('[login] console errors:', errors);
  
  const token = await page.evaluate(() => localStorage.getItem('pz.access'));
  console.log('[login] token after reload:', token);
  
  await expect(page.locator('#dashboard')).toBeVisible();
}

export function randomEmail() {
  return `e2e-${Date.now()}@example.com`;
}