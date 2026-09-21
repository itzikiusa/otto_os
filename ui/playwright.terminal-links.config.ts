import { defineConfig } from '@playwright/test';
export default defineConfig({
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }, { name: 'webkit', use: { browserName: 'webkit' } }],
  testDir: './e2e', testMatch: 'desktop-terminal-links.spec.ts', workers: 1,
  use: { baseURL: 'http://localhost:5296', viewport: { width: 1150, height: 850 } },
  webServer: { command: 'npm run dev -- --port 5296 --strictPort', url: 'http://localhost:5296', reuseExistingServer: false },
});
