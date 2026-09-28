import { defineConfig } from '@playwright/test';
import base from './playwright.config';

// The room transport fixtures and four-peer media probe use Chromium. Keep
// their intended project name explicit instead of silently skipping the media
// case under the mobile-only projects in the general suite.
export default defineConfig({
  ...base,
  projects: [{
    name: 'desktop-light',
    testMatch: /(?:^|\/)(?:rooms|desktop-room-window)\.spec\.ts$/,
    use: { browserName: 'chromium', viewport: { width: 1280, height: 800 } },
  }],
});
