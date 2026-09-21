import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e', testMatch: 'desktop-network-profiles.spec.ts',
  use: { baseURL: 'http://localhost:5291', viewport: { width: 390, height: 844 } },
  webServer: { command: 'npm run dev -- --port 5291 --strictPort', url: 'http://localhost:5291', reuseExistingServer: false },
});
