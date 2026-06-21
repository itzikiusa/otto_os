import { defineConfig, devices } from '@playwright/test';

// Mobile/tablet E2E suite. Runs the real UI (Vite dev server) against an
// ISOLATED throwaway daemon spun up in global-setup (temp data dir + temp port)
// so tests never touch the user's real sessions/DBs.
//
// SLOT ISOLATION: every port + the auth dir is keyed off OTTO_E2E_SLOT so
// multiple agents can run their own page's suite in parallel without colliding.
// Defaults (slot 0) keep the normal single-run workflow working unchanged.

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const PW_PORT = process.env.OTTO_E2E_PW_PORT ?? '5173';
const UI = process.env.OTTO_E2E_UI ?? `http://localhost:${PW_PORT}`;
const STATE = `e2e/.auth-${SLOT}/state.json`;

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  globalTeardown: './e2e/global-teardown.ts',
  timeout: 45_000,
  expect: { timeout: 10_000 },
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 2 : 4,
  reporter: [['list'], ['html', { open: 'never', outputFolder: `e2e/.report-${SLOT}` }]],
  use: {
    baseURL: UI,
    storageState: STATE,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: `npm run dev -- --port ${PW_PORT} --strictPort`,
    url: UI,
    reuseExistingServer: !process.env.CI,
    timeout: 90_000,
  },
  projects: [
    { name: 'iphone-portrait', use: { ...devices['iPhone 14 Pro Max'], storageState: STATE } },
    { name: 'iphone-landscape', use: { ...devices['iPhone 14 Pro Max landscape'], storageState: STATE } },
    { name: 'ipad-portrait', use: { ...devices['iPad Pro 11'], storageState: STATE } },
    { name: 'ipad-landscape', use: { ...devices['iPad Pro 11 landscape'], storageState: STATE } },
    { name: 'iphone-se', use: { ...devices['iPhone SE'], storageState: STATE } },
    // Desktop BROWSER (non-Tauri): exercises the ≥1025px 3-pane shell (the
    // remote-desktop path). testMatch restricts it to the desktop-* spec so the
    // mobile specs don't run at desktop width.
    {
      name: 'desktop-browser',
      testMatch: /desktop-.*\.spec\.ts/,
      use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 }, storageState: STATE },
    },
  ],
});
