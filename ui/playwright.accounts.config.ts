import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e', testMatch: ['desktop-accounts.spec.ts', 'desktop-token-cleanup.spec.ts'],
  use: { baseURL: 'http://localhost:5293', viewport: { width: 1100, height: 900 } },
  webServer: { command: 'npm run dev -- --port 5293 --strictPort', url: 'http://localhost:5293', reuseExistingServer: false },
});
