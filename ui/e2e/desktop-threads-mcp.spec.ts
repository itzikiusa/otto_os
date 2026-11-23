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
  // The inbox says what the thread waits on, like any other approval.
  await expect(page.getByRole('complementary', { name: 'Threads' }).getByRole('button', { name: /MCP approve/ }).first()).toContainText('Approval');
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
  // The details card's row says it in one line and opens the same list
  // (the card heads the right panel when it doesn't float).
  const detailsRow = page.getByTestId('details-mcp').first();
  if (!(await detailsRow.isVisible())) await page.getByRole('button', { name: 'Right panel' }).click();
  await expect(detailsRow).toContainText(/MCP · \d+ servers · \d+ needs? attention/);
  await detailsRow.click();
  const sheet = page.getByTestId('runtime-mcp-servers');
  await expect(sheet).toBeVisible();
  await page.getByRole('button', { name: 'Done' }).click();
  await page.getByRole('button', { name: 'Thread and view actions' }).click();
  await page.getByRole('menuitem', { name: 'MCP servers' }).click();
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

test('a server enabled mid-thread says "MCP servers changed", and Restart agent applies it', async ({ page }) => {
  const { ctx, base, wsId } = t.state;
  const { thread } = await t.send(page, 'MCP changed', 'hello');
  await expect(thread.getByText('echo: hello')).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByTestId('runtime-mcp-changed')).toHaveCount(0);
  const made = await ctx.post(`${base}/api/v1/workspaces/${wsId}/mcp-servers`, {
    data: { name: 'late-probe', command: process.execPath, args: [PROBE], enabled: true },
  });
  expect(made.ok(), await made.text()).toBeTruthy();
  await page.reload();
  const line = thread.getByTestId('runtime-mcp-changed');
  await expect(line).toContainText('MCP servers changed', { timeout: 30_000 });
  await line.getByRole('button', { name: 'Restart agent to apply' }).click();
  await expect(line).toHaveCount(0, { timeout: 30_000 });
  // The restarted agent has it, and the conversation goes on.
  await page.getByRole('button', { name: 'Thread and view actions' }).click();
  await page.getByRole('menuitem', { name: 'MCP servers' }).click();
  await expect(page.locator('[data-testid="runtime-mcp-server"][data-name="late-probe"]')).toContainText('Connected');
});

test('a fresh context (/clear) and a revive keep the thread\'s MCP tools', async ({ page }) => {
  const { ctx, base, wsId } = t.state;
  const made = await ctx.post(`${base}/api/v1/workspaces/${wsId}/mcp-servers`, {
    data: { name: 'clear-probe', command: process.execPath, args: [PROBE], enabled: true },
  });
  expect(made.ok(), await made.text()).toBeTruthy();
  const { id, thread } = await t.send(page, 'MCP clear', 'hello');
  await expect(thread.getByText('echo: hello')).toBeVisible({ timeout: 30_000 });
  const call = (text: string, extra: Record<string, unknown> = {}) =>
    ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text: `mcp clear-probe probe_echo {"text":"${text}"}`, ...extra } });
  // /clear: a new provider conversation, a new process — same servers.
  expect((await call('after clear', { fresh_context: true })).ok()).toBeTruthy();
  await expect(thread.getByText('probe says: after clear', { exact: false })).toHaveCount(0);
  const rows = thread.locator('[data-item-type="dynamic_tool"]');
  await expect(rows.filter({ hasText: 'clear-probe · probe_echo' }).first()).toBeVisible({ timeout: 30_000 });
  await rows.filter({ hasText: 'clear-probe · probe_echo' }).first().click();
  await expect(thread.getByText('probe says: after clear', { exact: false }).first()).toBeVisible({ timeout: 30_000 });
  // Close + reopen (the agent resumes): still there.
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`)).ok()).toBeTruthy();
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${id}/reopen`)).ok()).toBeTruthy();
  expect((await call('after reopen')).ok()).toBeTruthy();
  await expect(rows.filter({ hasText: 'clear-probe · probe_echo' }).nth(1)).toBeVisible({ timeout: 30_000 });
  await rows.filter({ hasText: 'clear-probe · probe_echo' }).nth(1).click();
  await expect(thread.getByText('probe says: after reopen', { exact: false }).first()).toBeVisible({ timeout: 30_000 });
});

test('switching the thread to another provider hands it the same MCP servers', async ({ page }) => {
  const { ctx, base, wsId } = t.state;
  const made = await ctx.post(`${base}/api/v1/workspaces/${wsId}/mcp-servers`, {
    data: { name: 'switch-probe', command: process.execPath, args: [PROBE], enabled: true },
  });
  expect(made.ok(), await made.text()).toBeTruthy();
  const { id, thread } = await t.send(page, 'MCP switch', 'hello');
  await expect(thread.getByText('echo: hello')).toBeVisible({ timeout: 30_000 });
  const sw = await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text: 'after switch', provider: 'codex' } });
  expect(sw.ok(), await sw.text()).toBeTruthy();
  await expect(thread.getByText('after switch').first()).toBeVisible({ timeout: 30_000 });
  await expect.poll(async () => {
    const r = await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).json();
    return r.provider;
  }, { timeout: 30_000 }).toBe('codex');
  await page.getByRole('button', { name: 'Thread and view actions' }).click();
  await page.getByRole('menuitem', { name: 'MCP servers' }).click();
  const sheet = page.getByTestId('runtime-mcp-servers');
  await expect(sheet.locator('[data-name="otto"]')).toBeVisible();
  await expect(sheet.locator('[data-name="switch-probe"]')).toBeVisible();
  // Codex's list, not Claude's: the fake Claude's own `own-docs` is gone.
  await expect(sheet.locator('[data-name="own-docs"]')).toHaveCount(0);
});
