import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e', testMatch: 'desktop-filesystem-permissions.spec.ts',
  use: { baseURL: 'http://localhost:5294', viewport: { width: 390, height: 844 } },
  webServer: { command: 'npm run dev -- --port 5294 --strictPort', url: 'http://localhost:5294', reuseExistingServer: false },
});
