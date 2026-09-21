import { defineConfig } from '@playwright/test';

// Component integration only: intercepted HTTP/WS, no daemon or git fixtures.
export default defineConfig({
  testDir: './e2e',
  testMatch: 'desktop-workflow-summarizer.spec.ts',
  use: { baseURL: 'http://localhost:5287', viewport: { width: 1000, height: 800 } },
  webServer: {
    command: 'npm run dev -- --port 5287 --strictPort',
    url: 'http://localhost:5287', reuseExistingServer: false,
  },
});
