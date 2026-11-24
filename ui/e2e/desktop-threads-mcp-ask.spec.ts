import { test } from '@playwright/test';
import { checkMcpAsk } from './threads-mcp-ask-fixture';

// An app's (MCP) approval in a thread's dock, at 1600 wide (threads-mcp-ask-fixture.ts).
test('an app approval reads as a sentence; its arguments are details', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1600, height: 900 });
  await checkMcpAsk(page, '1600');
});
