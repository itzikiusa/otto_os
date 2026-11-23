import { test, expect } from '@playwright/test';
import { mcpThreads } from './threads-mcp-fixture';

// An approval-gated Otto tool called by a thread's agent asks IN the thread
// (the attention dock: "Claude wants to use otto · broadcast_message" · Deny /
// Allow once), and the waiting call then runs on that answer — no
// "pending_approval, resubmit after it is approved" reply.
const t = mcpThreads('desktop-browser');

test('an approval-gated Otto tool asks in the thread, and Allow once runs the waiting call', async ({ page }) => {
  const { thread, dock } = await t.askBroadcast(page, 'MCP approve', 'hello from the dock');
  await expect(dock.getByRole('button', { name: 'Deny', exact: true })).toBeVisible();
  await dock.getByRole('button', { name: 'Allow once', exact: true }).click();
  await expect(dock).toBeHidden({ timeout: 15_000 });
  // The SAME call finished on the answer: the agent got the tool's result.
  await expect(thread.getByText('mcp done')).toBeVisible({ timeout: 30_000 });
  const row = thread.locator('[data-item-type="dynamic_tool"]').first();
  await row.click();
  await expect(row).not.toContainText('pending_approval');
  await expect(row).not.toContainText('resubmit');
});
