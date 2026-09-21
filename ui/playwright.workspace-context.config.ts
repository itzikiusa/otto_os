import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e', testMatch: 'desktop-workspace-context.spec.ts',
  use: { baseURL: 'http://localhost:5291', viewport: { width: 1100, height: 900 } },
  webServer: { command: 'npm run dev -- --port 5291 --strictPort', url: 'http://localhost:5291', reuseExistingServer: false },
});
