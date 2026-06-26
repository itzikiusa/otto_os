import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage, expectNoHorizontalOverflow, expectContentHasHeight } from './helpers';

// End-to-end coverage for Mission Control / the work graph.
//
// Seeds — via the API — a real session and a swarm (project + tasks) into a
// fresh workspace, then triggers the work-graph BACKFILL so the projector
// materializes both as work items. This exercises the full route → policy →
// auth → projector → repo stack (which the Rust unit tests do not), then drives
// the UI page across the device matrix for layout + the core flows (list,
// detail, approval gate, graph view, live filter).
//
// LIVE-DATA NOTE: the isolated test daemon never spawns agent CLIs, so there are
// no real costs/tool-calls; we assert the projected items, their lifecycle, the
// approval flow, and the rendered layout — all deterministic after backfill.

let workspaceId = '';
let base = '';

async function wg(ctx: APIRequestContext, method: 'get' | 'post' | 'patch', path: string, data?: unknown) {
  const url = `${base}/api/v1/workspaces/${workspaceId}/workgraph${path}`;
  const r = await (method === 'get' ? ctx.get(url) : method === 'post' ? ctx.post(url, { data }) : ctx.patch(url, { data }));
  return r;
}

test.beforeAll(async () => {
  const a = await apiCtx();
  base = a.base;
  workspaceId = await seedWorkspace(a.ctx, base);
  await seedShellSession(a.ctx, base, workspaceId); // → a `session` work item
  // A MINIMAL swarm (bare swarm + one project, no preset org-tree / tasks / board
  // posts) → a `swarm` (project) work item. Kept light on purpose: the full
  // `seedSwarm` helper's ~25 writes + per-task swarm events, multiplied across 5
  // device projects on the shared E2E daemon, briefly overloads other pages.
  const sw = await (
    await a.ctx.post(`${base}/api/v1/workspaces/${workspaceId}/swarm/swarms`, {
      data: { name: 'E2E Swarm' },
    })
  ).json();
  await a.ctx.post(`${base}/api/v1/swarm/swarms/${sw.id}/projects`, {
    data: { name: 'E2E Project', goal_md: 'Ship Mission Control.' },
  });
  // Materialize the graph from the seeded source rows (idempotent, deterministic).
  const r = await a.ctx.post(`${base}/api/v1/workspaces/${workspaceId}/workgraph/backfill`);
  expect(r.ok(), `backfill → ${r.status()}`).toBeTruthy();
  await a.ctx.dispose();
});

// Activate the seeded workspace (so Mission Control loads it) and collapse the
// nav drawer (defaults open on a fresh phone profile and would cover the page).
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

test.describe('mission-control API (route + policy + projector)', () => {
  test('backfill projected the seeded session and swarm into work items', async () => {
    const { ctx } = await apiCtx();
    const summary = await (await wg(ctx, 'get', '/summary')).json();
    expect(summary.total).toBeGreaterThanOrEqual(2);
    expect(summary.by_kind.length).toBeGreaterThanOrEqual(2);

    const items = await (await wg(ctx, 'get', '/items?limit=200')).json();
    const kinds = new Set(items.map((i: { kind: string }) => i.kind));
    expect(kinds.has('session')).toBeTruthy();
    expect(kinds.has('swarm')).toBeTruthy();
    await ctx.dispose();
  });

  test('item detail carries the traceable spine (owner/goal/status/events)', async () => {
    const { ctx } = await apiCtx();
    const items = await (await wg(ctx, 'get', '/items?kind=swarm')).json();
    expect(items.length).toBeGreaterThanOrEqual(1);
    const detail = await (await wg(ctx, 'get', `/items/${items[0].id}`)).json();
    expect(detail.id).toBe(items[0].id);
    expect(detail).toHaveProperty('events');
    expect(detail).toHaveProperty('artifacts');
    expect(detail).toHaveProperty('approvals');
    expect(detail.status).toBeTruthy();
    expect(detail.risk_level).toBeTruthy();
    // A `created` audit event exists for every projected item.
    expect(detail.events.some((e: { event_type: string }) => e.event_type === 'created')).toBeTruthy();
    await ctx.dispose();
  });

  test('approval gate: request → needs_approval, decide → cleared', async () => {
    const { ctx } = await apiCtx();
    const items = await (await wg(ctx, 'get', '/items?kind=session')).json();
    const id = items[0].id;
    const ap = await (await wg(ctx, 'post', `/items/${id}/approvals`, { reason: 'ship it?' })).json();
    expect(ap.status).toBe('pending');

    let detail = await (await wg(ctx, 'get', `/items/${id}`)).json();
    expect(detail.needs_approval).toBeTruthy();
    expect(detail.pending_approvals).toBeGreaterThanOrEqual(1);

    const decided = await (await wg(ctx, 'post', `/approvals/${ap.id}/decide`, { decision: 'approved', note: 'ok' })).json();
    expect(decided.status).toBe('approved');

    detail = await (await wg(ctx, 'get', `/items/${id}`)).json();
    expect(detail.needs_approval).toBeFalsy();
    await ctx.dispose();
  });

  test('patch annotates risk/goal (the policy axis)', async () => {
    const { ctx } = await apiCtx();
    const items = await (await wg(ctx, 'get', '/items?kind=swarm')).json();
    const id = items[0].id;
    const patched = await (await wg(ctx, 'patch', `/items/${id}`, { risk_level: 'critical', goal: 'E2E goal override' })).json();
    expect(patched.risk_level).toBe('critical');
    expect(patched.goal).toBe('E2E goal override');
    await ctx.dispose();
  });

  test('graph view returns nodes (and edges when present)', async () => {
    const { ctx } = await apiCtx();
    const g = await (await wg(ctx, 'get', '/graph')).json();
    expect(Array.isArray(g.nodes)).toBeTruthy();
    expect(g.nodes.length).toBeGreaterThanOrEqual(2);
    expect(Array.isArray(g.edges)).toBeTruthy();
    await ctx.dispose();
  });
});

test.describe('mission-control UI', () => {
  test('renders a sized, non-overflowing page with seeded work', async ({ page }) => {
    await openPage(page, 'mission-control');
    await expectContentHasHeight(page);
    await expectNoHorizontalOverflow(page);
    await expect(page.getByRole('heading', { name: /mission control/i }).first()).toBeVisible();
    // The backfill (beforeAll) materialized at least the seeded session + swarm.
    await expect(page.locator('.wi-row').first()).toBeVisible({ timeout: 15_000 });
    expect(await page.locator('.wi-row').count()).toBeGreaterThanOrEqual(2);
  });

  test('opening a work item reveals its traceable detail (timeline)', async ({ page }) => {
    await openPage(page, 'mission-control');
    const row = page.locator('.wi-row').first();
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.click();
    // The detail panel (rendered only when an item is selected) shows the
    // traceable spine: the Timeline (audit) heading is always present.
    const detail = page.locator('.mc-detail');
    await expect(detail).toBeVisible({ timeout: 15_000 });
    await expect(detail.getByRole('heading', { name: /timeline/i })).toBeVisible();
  });

  test('switches to the graph view', async ({ page }) => {
    await openPage(page, 'mission-control');
    await page.getByRole('tab', { name: /graph/i }).click();
    await expect(page.locator('svg.graph-svg')).toBeVisible({ timeout: 15_000 });
  });
});
