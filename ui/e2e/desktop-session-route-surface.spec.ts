import { test, expect, type APIRequestContext } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { runBarCommand } from './helpers';

// A session opens where it lives and is listed once (sessionBuckets.ts
// `inThreadsScope` / `sessionRoute`):
//   • ⌘K "Focus <thread title>" lands on `#/threads/<id>`, not a terminal tab
//     in Agents (navigateToSession used to route every session to agents/…);
//   • a worker delegated by a thread (no `meta.surface` of its own) inherits
//     the thread's page: it is in the Threads sidebar and NOT in the Agents
//     navigator (it used to be in both).
// Inert shell PTYs, so no coding CLI is needed.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
let lead = '';
let worker = '';
let classic = '';

async function seed(title: string, meta: Record<string, unknown>): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: dir, meta: { origin: 'e2e', nested_provider: 'claude', ...meta } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-route-surface-'));
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Route Surface WS', root_path: dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
  lead = await seed('Route Lead Thread', { surface: 'threads' });
  // `delegated_by` is stamped by the daemon from the creating agent's token
  // (a body value is dropped), so the seed links the worker the other way the
  // rule reads: `parent_session_id`.
  worker = await seed('Route Delegated Worker', { parent_session_id: lead });
  classic = await seed('Route Classic Terminal', {});
});

test.afterAll(async () => {
  for (const id of [worker, classic, lead]) {
    if (id && ctx) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
  }
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('route-surface-init')) return;
    sessionStorage.setItem('route-surface-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '1');
    localStorage.removeItem('otto_threads_v1');
    localStorage.removeItem('otto_sidebar_groups_collapsed');
  }, wsId);
});

test('⌘K "Focus <thread>" opens the thread in Threads; a terminal session still opens in Agents', async ({ page }) => {
  await page.goto('/#/agents');
  const nav = page.locator('.navigator');
  await expect(nav).toBeVisible({ timeout: 30_000 });
  await expect(nav.getByText('Route Classic Terminal').first()).toBeVisible({ timeout: 15_000 });

  await runBarCommand(page, 'Focus Route Lead Thread');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/threads/${lead}`);
  await expect(page.getByTestId('threads-page')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('threads-breadcrumb')).toContainText('Route Lead Thread');

  await runBarCommand(page, 'Focus Route Classic Terminal');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/agents/${classic}`);
});

test('a worker delegated by a thread is listed once — in Threads, not in Agents — and opens there', async ({ page }) => {
  await page.goto(`/#/threads/${lead}`);
  const sidebar = page.getByTestId('threads-sidebar');
  await expect(sidebar.getByTestId(`thread-row-${lead}`)).toBeVisible({ timeout: 30_000 });
  await expect(sidebar.getByTestId(`thread-row-${worker}`)).toHaveCount(1);

  await page.goto('/#/agents');
  const nav = page.locator('.navigator');
  await expect(nav.getByText('Route Classic Terminal').first()).toBeVisible({ timeout: 30_000 });
  await expect(nav.getByText('Route Delegated Worker')).toHaveCount(0);
  await expect(nav.getByText('Route Lead Thread')).toHaveCount(0);

  await runBarCommand(page, 'Focus Route Delegated Worker');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe(`#/threads/${worker}`);
});
