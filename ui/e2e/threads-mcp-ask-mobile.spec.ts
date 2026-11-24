import { test } from '@playwright/test';
import { checkMcpAsk } from './threads-mcp-ask-fixture';

// An app's (MCP) approval in a thread's dock, on a phone (threads-mcp-ask-fixture.ts).
test('phone: an app approval reads as a sentence; its arguments are details', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'iphone-portrait only');
  await checkMcpAsk(page, 'phone');
});
