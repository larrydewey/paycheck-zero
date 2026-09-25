import { test, expect } from '@playwright/test';

test('probe page boot', async ({ page }) => {
  const logs: string[] = [];
  page.on('console', m => {
    const text = m.text().slice(0, 500);
    logs.push(`[console:${m.type()}] ${text}`);
    if (m.type() === 'error') {
      // Capture errors for later
      page.evaluate(() => {
        window._dsConsoleErrors = window._dsConsoleErrors || [];
        window._dsConsoleErrors.push(text);
      });
    }
  });
  page.on('pageerror', e => logs.push(`[pageerror] ${e.stack || e.message}`));
  page.on('requestfailed', r => logs.push(`[reqfail] ${r.url().slice(0, 150)} ${r.failure()?.errorText ?? ''}`));

  // Intercept the SSE session response to log its raw body
  const responsePromises: { url: string; promise: Promise<string> }[] = [];
  page.on('response', async (r) => {
    if (/(datastar|\.js)/.test(r.url())) {
      logs.push(`[resp] ${r.status()} ${r.url().slice(0, 150)}`);
    }
    if (/\/api\/v1\/ui\/session/.test(r.url())) {
      logs.push(`[resp:sse] ${r.status()} ${r.url()}`);
      try {
        const body = await r.text();
        logs.push(`[sse body] ${body.slice(0, 600)}`);
      } catch (e) {
        logs.push(`[sse body err] ${e}`);
      }
    }
  });

  await page.goto('/', { waitUntil: 'networkidle', timeout: 15000 }).catch(() => {});
  // Wait for datastar-ready event or a reasonable timeout
  await page.waitForFunction(() => {
    return new Promise(resolve => {
      document.addEventListener('datastar-ready', () => resolve(true));
      setTimeout(() => resolve(false), 5000);
    });
  });
  await page.waitForTimeout(2000);
  
  // Log any console errors
  const consoleErrors = await page.evaluate(() => {
    return window._dsConsoleErrors || [];
  });
  if (consoleErrors.length > 0) {
    logs.push(`[console-errors] ${JSON.stringify(consoleErrors)}`);
  }

  logs.push(`[script] ${await page.evaluate(() => {
    const s = document.querySelector('script[type=module]');
    return s ? (s as HTMLScriptElement).src : 'NONE';
  })}`);
  logs.push(`[app] ${(await page.evaluate(() => document.getElementById('app')?.innerHTML)).slice(0, 600)}`);

  // Check for Datastar presence
  const dsInfo = await page.evaluate(() => {
    return {
      datastarReady: typeof window.dispatchEvent === 'function',
      hasDatastar: 'datastar' in window || '_ds' in window,
      dataStarEvent: typeof CustomEvent !== 'undefined',
      scriptLoaded: document.querySelector('script[type=module]')?.src,
    };
  });
  logs.push(`[dsinfo] ${JSON.stringify(dsInfo)}`);

  console.log(logs.join('\n'));
  await expect(page.locator('#auth-view')).toBeVisible();
});