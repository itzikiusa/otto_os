import { defineConfig, devices } from '@playwright/test';
const port = process.env.OTTO_ROOM_UI_PORT ?? '5283';
const origin = `http://127.0.0.1:${port}`;
/** Synthetic room sockets only: no daemon, owner state, camera or microphone. */
export default defineConfig({
  testDir: './e2e', testMatch: ['rooms.spec.ts', 'room-recap.spec.ts'], timeout: 45000,
  use: {baseURL: origin, trace: 'retain-on-failure'},
  outputDir: process.env.OTTO_ROOM_UI_OUTPUT ?? '/tmp/otto-room-ui-results', reporter: 'list',
  webServer: {command: `npm run dev -- --host 127.0.0.1 --port ${port} --strictPort`, url: origin, reuseExistingServer: true},
  projects: [
    {name: 'desktop-light', use: {...devices['Desktop Chrome'], viewport: {width: 1280, height: 800}, colorScheme: 'light'}},
    {name: 'desktop-dark', use: {...devices['Desktop Chrome'], viewport: {width: 1280, height: 800}, colorScheme: 'dark'}},
    {name: 'phone', use: {...devices['iPhone 13'], defaultBrowserType: 'chromium'}},
  ],
});
