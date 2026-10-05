import { defineConfig, devices } from '@playwright/test';
import { DESKTOP_GATE_SPECS, MOBILE_GATE_SPECS, gateMatcher } from './e2e/gate-specs';

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
// CI's blocking functional job (ci.yml `e2e-functional`) sets this: the perf
// specs belong to the perf-gates job (desktop-webkit, scaled budgets), so the
// functional shards skip them instead of re-running wall-clock budgets on Chromium.
const FUNCTIONAL_ONLY = process.env.OTTO_E2E_FUNCTIONAL_ONLY === '1';
// Desktop specs assume the ≥1025px 3-pane shell; most do not self-skip at
// phone/tablet width, so the mobile projects never pick them up.
const MOBILE_IGNORE = /desktop-.*\.spec\.ts/;

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  globalTeardown: './e2e/global-teardown.ts',
  timeout: 45_000,
  expect: { timeout: 10_000 },
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  // No retries anywhere: a retry turns a real flake into a silent pass. CI also
  // fails on any `flaky` count in the JSON report (scripts/e2e-flaky-check.mjs)
  // in case a spec opts back into retries via test.describe.configure.
  retries: 0,
  workers: process.env.CI ? 2 : 4,
  reporter: [
    ['list'],
    ['html', { open: 'never', outputFolder: `e2e/.report-${SLOT}` }],
    // Machine-readable summary for CI's flaky-count gate (outside the html
    // folder, which the html reporter wipes on start).
    ['json', { outputFile: `e2e/.results-${SLOT}.json` }],
  ],
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
    { name: 'iphone-portrait', testIgnore: MOBILE_IGNORE, use: { ...devices['iPhone 14 Pro Max'], storageState: STATE } },
    { name: 'iphone-landscape', testIgnore: MOBILE_IGNORE, use: { ...devices['iPhone 14 Pro Max landscape'], storageState: STATE } },
    { name: 'ipad-portrait', testIgnore: MOBILE_IGNORE, use: { ...devices['iPad Pro 11'], storageState: STATE } },
    { name: 'ipad-landscape', testIgnore: MOBILE_IGNORE, use: { ...devices['iPad Pro 11 landscape'], storageState: STATE } },
    { name: 'iphone-se', testIgnore: MOBILE_IGNORE, use: { ...devices['iPhone SE'], storageState: STATE } },
    // Desktop BROWSER (non-Tauri): exercises the ≥1025px 3-pane shell (the
    // remote-desktop path). testMatch restricts it to the desktop-* spec so the
    // mobile specs don't run at desktop width.
    {
      name: 'desktop-browser',
      testMatch: /desktop-.*\.spec\.ts/,
      testIgnore: FUNCTIONAL_ONLY ? /desktop-.*perf.*\.spec\.ts/ : undefined,
      use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 }, storageState: STATE },
    },
    // The BLOCKING smoke gate (ci.yml `e2e-gate`): a green-history subset of
    // the specs above, listed in e2e/gate-specs.ts. Same devices/viewports as
    // desktop-browser / iphone-portrait, so a gate spec behaves identically in
    // the advisory shards.
    {
      name: 'desktop-gate',
      testMatch: gateMatcher(DESKTOP_GATE_SPECS),
      use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 }, storageState: STATE },
    },
    {
      name: 'iphone-gate',
      testMatch: gateMatcher(MOBILE_GATE_SPECS),
      use: { ...devices['iPhone 14 Pro Max'], storageState: STATE },
    },
    // Desktop WEBKIT: the perf gates (desktop-*perf*) on the engine closest to
    // the app's WKWebView, where style/layout/paint dominate (r3-10-02).
    // `npx playwright test --project=desktop-webkit`. Chromium-only probes
    // (long tasks, CDP heap) skip themselves here; see e2e/perf.ts.
    {
      name: 'desktop-webkit',
      testMatch: /desktop-.*perf.*\.spec\.ts/,
      // No service worker: once sw.js claims the page, WebKit's fetches go
      // through it and `page.route` mocks (most perf fixtures) never fire.
      use: {
        ...devices['Desktop Safari'],
        viewport: { width: 1280, height: 800 },
        storageState: STATE,
        serviceWorkers: 'block',
      },
    },
  ],
});
