import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { mcpThreads } from './threads-mcp-fixture';

// On a phone the same card asks in the thread with full-size answers, and
// Deny reaches the waiting call at once ("denied", not a resubmit hint).
const t = mcpThreads('iphone-portrait');

test('Deny on a phone answers the card in the thread and the agent hears "denied"', async ({ page }) => {
  const { thread, dock } = await t.askBroadcast(page, 'MCP deny', 'phone ask');
  await expectNoHorizontalOverflow(page);
  const deny = dock.getByRole('button', { name: 'Deny', exact: true });
  await expectFullyInViewport(page, deny, 'Deny');
  expect((await deny.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await deny.tap();
  await expect(dock).toBeHidden({ timeout: 15_000 });
  await expect(thread.getByText('mcp done')).toBeVisible({ timeout: 30_000 });
  const row = thread.locator('[data-item-type="dynamic_tool"]').first();
  await row.click();
  await expect(row).toContainText('denied');
});
