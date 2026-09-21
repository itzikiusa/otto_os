import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e', testMatch: 'desktop-personal-documents.spec.ts',
  use: { baseURL: 'http://localhost:5288', viewport: { width: 800, height: 900 } },
  webServer: { command: 'npm run dev -- --port 5288 --strictPort', url: 'http://localhost:5288', reuseExistingServer: false },
});
