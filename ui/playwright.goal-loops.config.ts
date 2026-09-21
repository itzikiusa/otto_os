import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e', testMatch: 'desktop-goal-loop-verification.spec.ts',
  use: { baseURL: 'http://localhost:5289', viewport: { width: 1100, height: 900 } },
  webServer: { command: 'npm run dev -- --port 5289 --strictPort', url: 'http://localhost:5289', reuseExistingServer: false },
});
