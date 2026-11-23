import { test, expect } from '@playwright/test';
import { fileURLToPath } from 'node:url';
import { mcpThreads } from './threads-mcp-fixture';

const PROBE = fileURLToPath(new URL('./fixtures/mcp-probe.mjs', import.meta.url));

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

test('thread ⋯ ▸ MCP servers lists what the agent has, each with a word and why', async ({ page }) => {
  const { ctx, base, wsId } = t.state;
  const made = await ctx.post(`${base}/api/v1/workspaces/${wsId}/mcp-servers`, {
    data: { name: 'probe-missing', command: '/nonexistent/otto-e2e-probe', enabled: true },
  });
  expect(made.ok(), await made.text()).toBeTruthy();
  const { thread } = await t.send(page, 'MCP list', 'hello');
  await expect(thread.getByText('echo: hello')).toBeVisible({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Thread and view actions' }).click();
  await page.getByRole('menuitem', { name: 'MCP servers' }).click();
  const sheet = page.getByTestId('runtime-mcp-servers');
  await expect(sheet).toBeVisible();
  const row = (name: string) => sheet.locator(`[data-testid="runtime-mcp-server"][data-name="${name}"]`);
  await expect(row('otto')).toContainText('Connected');
  await expect(row('otto')).toContainText('Otto');
  await expect(row('probe-missing')).toContainText('Failed');
  await expect(row('probe-missing')).toContainText('This workspace');
  await expect(row('own-docs')).toContainText('Needs sign-in');
  await expect(row('own-docs')).toContainText('Claude’s own config');
  // Problems first.
  await expect(sheet.locator('[data-testid="runtime-mcp-server"]').first()).toHaveAttribute('data-name', /probe-missing|own-docs/);
});

test('an MCP tool card reads "<server> · <tool>" with its arguments and a link result as a link', async ({ page }) => {
  const { ctx, base, wsId } = t.state;
  const made = await ctx.post(`${base}/api/v1/workspaces/${wsId}/mcp-servers`, {
    data: { name: 'probe', command: process.execPath, args: [PROBE], enabled: true },
  });
  expect(made.ok(), await made.text()).toBeTruthy();
  const { thread } = await t.send(page, 'MCP card', 'mcp probe probe_link {"why":"e2e"}');
  await expect(thread.getByText('mcp done')).toBeVisible({ timeout: 30_000 });
  const row = thread.locator('[data-item-type="dynamic_tool"]').first();
  await expect(row).toContainText('probe · probe_link');
  await row.click();
  await expect(row).toContainText('"why": "e2e"');
  await expect(row).toContainText('Link: report.pdf — https://example.com/report.pdf (The probe report)');
  await expect(row).not.toContainText('"type":"resource_link"');
});
