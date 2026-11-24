// MCP Control Plane landing (U2): it opens on what waits on a person — a real
// pending approval (otto.create_pr through /mcp/otto-tools/invoke) appears as
// an inbox row without a reload and Approve clears it — and otherwise on four
// sentences with one link each. Tools: the catalog is collapsed, searchable
// and windowed. An outage is one notice line with one Retry.
import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedGitRepo, seedWorkspace } from './seed';
import { enableMcpFixtureCatalog } from './mcp-catalog-fixture';

let base = '';
let workspaceId = '';
let repoId = '';

test.describe.configure({ mode: 'serial' });
test.use({ serviceWorkers: 'block' });

test.beforeAll(async () => {
  const { ctx, base: b } = await apiCtx();
  base = b;
  workspaceId = await seedWorkspace(ctx, base);
  await enableMcpFixtureCatalog(ctx, base);
  repoId = (await seedGitRepo(ctx, base, workspaceId)).repoId;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), workspaceId);
});

/** File a real pending approval: an agent-side otto.create_pr call. */
async function ask(title: string): Promise<string> {
  const { ctx } = await apiCtx();
  const r = await ctx.post(`${base}/api/v1/mcp/otto-tools/invoke`, {
    data: { tool: 'otto.create_pr', arguments: { repo_id: repoId, title, description: 'Body', source_branch: `e2e/${title.replace(/\W+/g, '-')}`, target_branch: 'main' } },
  });
  const body = (await r.json()) as { decision: string; approval_id?: string };
  await ctx.dispose();
  expect(body.decision).toBe('pending_approval');
  return body.approval_id!;
}

/** API calls the page makes (method + path), from now on. */
function apiCalls(page: Page): string[] {
  const calls: string[] = [];
  page.on('request', (q) => {
    const u = new URL(q.url());
    if (u.pathname.startsWith('/api/v1/') && u.pathname.includes('mcp')) calls.push(`${q.method()} ${u.pathname.replace(/workspaces\/[^/]+/, 'workspaces/:id')}${u.searchParams.get('status') ? `?status=${u.searchParams.get('status')}` : ''}`);
  });
  return calls;
}

test('opening MCP asks for what waits and four sentences, nothing catalog-wide', async ({ page }) => {
  // Nothing pending here, whatever another spec left waiting on the shared daemon.
  await page.route('**/api/v1/mcp/approvals?status=pending', (route) => route.fulfill({ json: [] }));
  const calls = apiCalls(page);
  await page.goto('/#/mcp');
  const summary = page.getByTestId('mcp-summary');
  await expect(summary).toContainText('Nothing waits on you.', { timeout: 30_000 });
  for (const id of ['mcp-sum-servers', 'mcp-sum-tools', 'mcp-sum-rules', 'mcp-sum-denials']) {
    await expect(summary.getByTestId(id).getByRole('button')).toHaveCount(1);
  }
  await expect(summary.getByTestId('mcp-sum-servers')).toContainText('No external MCP servers yet.');
  await expect.poll(() => [...new Set(calls)].sort()).toEqual([
    'GET /api/v1/mcp/approvals?status=denied',
    'GET /api/v1/mcp/approvals?status=pending',
    'GET /api/v1/mcp/auto-approve',
    'GET /api/v1/mcp/otto-server',
    'GET /api/v1/workspaces/:id/mcp/servers',
  ]);
  await summary.getByTestId('mcp-sum-denials').getByRole('button', { name: 'Open activity' }).click();
  await expect(page).toHaveURL(/#\/mcp\/activity/);
});

test('a new request appears without a reload, in words, and Approve clears it', async ({ page }) => {
  await page.goto('/#/mcp');
  await expect(page.getByTestId('mcp-home')).toBeVisible({ timeout: 30_000 });
  const title = `Home approval ${Date.now()}`;
  const id = await ask(title);
  // The mcp_approval_changed event brings it in (no poll while the socket is up).
  const row = page.getByTestId('mcp-home-approval').filter({ hasText: title });
  await expect(row).toBeVisible({ timeout: 10_000 });
  await expect(row).toContainText('Approval');
  await expect(row).toContainText('asks');
  await expect(row).toContainText('otto.create_pr');
  await expect(page.getByTestId('mcp-pending-badge')).toBeVisible();
  await row.getByRole('button', { name: 'Approve', exact: true }).click();
  await expect(row).toHaveCount(0);
  const { ctx } = await apiCtx();
  const list = (await (await ctx.get(`${base}/api/v1/mcp/approvals`)).json()) as { id: string; status: string }[];
  await ctx.dispose();
  expect(['approved', 'consumed']).toContain(list.find((a) => a.id === id)?.status);
});

test('Tools: the catalog opens collapsed, a search opens its matches, rows are windowed', async ({ page }) => {
  const { ctx } = await apiCtx();
  const total = ((await (await ctx.get(`${base}/api/v1/mcp/otto-server`)).json()) as { tools: unknown[] }).tools.length;
  await ctx.dispose();
  await page.goto('/#/mcp/otto');
  const catalog = page.getByTestId('mcp-tool-catalog');
  await expect(catalog.locator('.tc-cat').first()).toBeVisible({ timeout: 30_000 });
  await expect(catalog.locator('.tc-tool')).toHaveCount(0);
  await expect(page.getByTestId('mcp-tool-counts')).toContainText(new RegExp(`\\d+ of ${total} on · \\d+ can change things`));
  // Every tool matches "otto.": the list opens fully but mounts only a window.
  await catalog.locator('.filter').fill('otto.');
  await expect(catalog.locator('.tc-tool').first()).toBeVisible();
  const mounted = await catalog.locator('.tc-tool').count();
  expect(total).toBeGreaterThan(60);
  expect(mounted, `${mounted} of ${total} tool rows mounted`).toBeLessThan(total);
  await catalog.locator('.filter').fill('create_pr');
  await page.getByTestId('mcp-tool-details-otto.create_pr').click();
  const details = page.getByRole('dialog', { name: 'otto.create_pr' });
  await expect(details).toBeVisible();
  await expect(details.locator('input[data-testid="mcp-auto-otto.create_pr"]')).toBeVisible();
});

test('an outage is one notice line with one Retry', async ({ page }) => {
  await page.route(/\/api\/v1\/(mcp|workspaces\/[^/]+\/mcp)\//, (route) => route.fulfill({ status: 503, json: { code: 'upstream', message: 'Control plane down' } }));
  await page.goto('/#/mcp/otto');
  await expect(page.getByTestId('mcp-outage')).toHaveCount(1, { timeout: 30_000 });
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(1);
  await page.goto('/#/mcp');
  await expect(page.getByTestId('mcp-outage')).toHaveCount(1, { timeout: 30_000 });
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(1);
});
