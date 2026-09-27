import { defineConfig, devices } from '@playwright/test';
import base from '../playwright.config';
const port = process.env.OTTO_E2E_PW_PORT ?? '5296';
const origin = process.env.OTTO_E2E_UI ?? `http://localhost:${port}`;
export default defineConfig({
  ...base,
  use: {...base.use, baseURL: origin, actionTimeout: 15000},
  testDir: '.', testMatch: 'desktop-rooms-live.spec.ts',
  globalSetup: './global-setup.ts', globalTeardown: './global-teardown.ts',
  workers: 1, timeout: 90000, reporter: 'list',
  outputDir: '/tmp/otto-rooms-live-results',
  webServer: {command: `npm run dev -- --config e2e/fixtures/rooms-live-vite.config.ts --port ${port} --strictPort`, url: origin, reuseExistingServer: false},
  projects: [{name: 'rooms-live', use: {...devices['Desktop Chrome'], viewport: {width: 1280, height: 800}}}],
});
