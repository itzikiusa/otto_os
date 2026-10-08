import { defineConfig, devices } from '@playwright/test';
import base from '../playwright.config';

const port = process.env.OTTO_E2E_PW_PORT ?? '5298';
const origin = process.env.OTTO_E2E_UI ?? `http://localhost:${port}`;

// The existing live-Rooms proxy forwards guest HTTP + WS to our isolated daemon.
export default defineConfig({
  ...base,
  testDir: '.', testMatch: 'desktop-room-games.spec.ts',
  globalSetup: './global-setup.ts', globalTeardown: './global-teardown.ts',
  workers: 1, fullyParallel: false, timeout: 90_000,
  reporter: [['list'], ['json', { outputFile: `/tmp/otto-room-games-results-${process.env.OTTO_E2E_SLOT ?? port}.json` }]],
  outputDir: `/tmp/otto-room-games-results-${process.env.OTTO_E2E_SLOT ?? port}`,
  use: {
    ...base.use, baseURL: origin, actionTimeout: 15_000, serviceWorkers: 'block',
    // Headless shell otherwise defaults to SwiftShader on macOS, making these
    // real 3D scenes CPU-rasterized and unsuitable for measuring native gameplay.
    launchOptions: { args: ['--disable-background-timer-throttling', '--disable-renderer-backgrounding',
      ...(process.platform === 'darwin' ? ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] : [])] },
  },
  webServer: {
    command: `npm run dev -- --config e2e/fixtures/rooms-live-vite.config.ts --port ${port} --strictPort`,
    url: origin, reuseExistingServer: false, timeout: 90_000,
  },
  projects: [{ name: 'room-games', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 1000 } } }],
});
