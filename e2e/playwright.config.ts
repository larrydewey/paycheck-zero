import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 30_000,
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:3099',
    viewport: { width: 1280, height: 720 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
  // Fresh DB + server per run (serve.mjs wipes its working dir and re-seeds).
  // Set PW_REUSE=1 to attach to an already-running server instead.
  webServer: {
    command: 'npm run serve',
    url: 'http://127.0.0.1:3099/health',
    reuseExistingServer: !!process.env.PW_REUSE,
    timeout: 120_000,
  },
});