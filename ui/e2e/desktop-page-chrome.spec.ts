import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Shared page chrome (desktop-browser).
//
// Every top-level module page renders exactly ONE <PageHeader> — the unified
// toolbar row with the page title — at one fixed height, so moving between
// modules never shifts the chrome. The Agents page is the exception: its
// session TabBar is the top row there.
//
// The header's actions collapse lowest-priority-first into a "⋯" menu (the
// global, viewport-clamped ctxMenu) instead of wrapping; and list/detail pages
// open on an item (restored or first) rather than an empty "pick one" pane.
// ─────────────────────────────────────────────────────────────────────────────

const V1 = '/api/v1';

const ROUTES = [
  'home',
  'history',
  'run-with-otto',
  'mission-control',
  'connections',
  'swarm',
  'loops',
  'proof',
  'git',
  'product',
  'vault',
  'canvas',
  'design',
  'design/learned',
  'design/brand',
  'browser',
  'aws',
  'kubernetes',
  'api',
  'mcp',
  'workflows',
  'scheduled-tasks',
  'personal-agents',
  'skills-eval',
  'insights',
  'usage',
  'walkthroughs',
  'settings/appearance',
  'settings/users',
  'brokers',
];

let wsId = '';
let packA = '';
let packB = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  // Two proof packs → the auto-select / restore checks.
  for (const title of ['Chrome proof A', 'Chrome proof B']) {
    const r = await ctx.post(`${base}${V1}/workspaces/${wsId}/proof-packs`, {
      data: { work_item_kind: 'manual', work_item_id: `wi-chrome-${title.slice(-1)}-${Date.now()}`, title },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    const id = (await r.json()).id as string;
    if (title.endsWith('A')) packA = id;
    else packB = id;
  }
  // A workflow → its editor header carries the longest action row.
  const wf = await ctx.post(`${base}${V1}/workspaces/${wsId}/workflows`, {
    data: {
      name: 'Chrome overflow workflow',
      description: 'e2e',
      graph: { nodes: [{ id: 'a', kind: 'agent', name: 'a', x: 0, y: 0, params: null }], edges: [] },
    },
  });
  expect(wf.ok(), await wf.text()).toBeTruthy();
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function gotoRoute(page: Page, route: string): Promise<void> {
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
}

test('every top-level page renders exactly one PageHeader at one height', async ({ page }) => {
  test.setTimeout(300_000);
  const heights = new Map<string, number>();
  for (const route of ROUTES) {
    await gotoRoute(page, route);
    const header = page.locator('[data-testid="page-header"]');
    await expect(header.first(), `${route}: page header`).toBeVisible({ timeout: 20_000 });
    await expect(header, `${route}: exactly one page header`).toHaveCount(1);
    const title = (await header.locator('h1').first().textContent())?.trim() ?? '';
    expect(title.length, `${route}: header title`).toBeGreaterThan(0);
    // The fixed bar row (tabs placed "below" add a second row under it).
    const row = await header.locator('.ph-row').boundingBox();
    expect(row, `${route}: header row box`).not.toBeNull();
    heights.set(route, Math.round(row!.height));
    // Title never glued to the top edge of the content pane.
    const h1 = await header.locator('h1').first().boundingBox();
    expect(h1!.y - row!.y, `${route}: title top inset`).toBeGreaterThanOrEqual(4);
  }
  const distinct = [...new Set(heights.values())];
  expect(distinct, `header row heights by route: ${JSON.stringify(Object.fromEntries(heights))}`).toEqual([46]);
});

test('the Agents page keeps its session TabBar instead of a PageHeader', async ({ page }) => {
  await gotoRoute(page, 'agents');
  await expect(page.locator('.tabbar')).toBeVisible();
  await expect(page.locator('[data-testid="page-header"]')).toHaveCount(0);
});

test('header actions overflow into a ⋯ menu that stays inside the viewport', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 700 });
  await gotoRoute(page, 'workflows');
  // The workflow auto-opens (single item) → the editor's action row.
  const header = page.locator('[data-testid="page-header"]');
  await expect(header.locator('h1')).toContainText('Chrome overflow workflow', { timeout: 20_000 });

  // Actions never wrap: the bar keeps its single-row height…
  const row = await header.locator('.ph-row').boundingBox();
  expect(Math.round(row!.height)).toBe(46);
  // …and the low-priority ones moved behind ⋯.
  const more = header.getByRole('button', { name: 'More actions' });
  await expect(more).toBeVisible();
  // Every visible action sits inside the header row (nothing spilled out).
  const actions = header.locator('.ph-actions > :not([data-ph-hidden])');
  for (let i = 0; i < (await actions.count()); i++) {
    const b = await actions.nth(i).boundingBox();
    if (!b || b.width === 0) continue;
    expect(b.y).toBeGreaterThanOrEqual(row!.y - 1);
    expect(b.y + b.height).toBeLessThanOrEqual(row!.y + row!.height + 1);
    expect(b.x + b.width).toBeLessThanOrEqual(1100);
  }

  await more.click();
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await page.waitForTimeout(100); // post-render clamp (rAF)
  await expectFullyInViewport(page, menu, 'header overflow menu');
  // The collapsed controls are real menu rows, labelled from their buttons.
  await expect(menu.getByRole('menuitem', { name: 'Tidy' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);

  // Wide enough → no overflow menu at all.
  await page.setViewportSize({ width: 1800, height: 900 });
  await expect(more).toHaveCount(0);
});

test('a list/detail page opens on its first item and restores the last one', async ({ page }) => {
  await page.addInitScript(() => {
    // Start without a remembered selection (one-shot: init scripts re-run on reload).
    if (!sessionStorage.getItem('chrome-reset')) {
      sessionStorage.setItem('chrome-reset', '1');
      localStorage.removeItem('otto.lastSelection.proof');
    }
  });
  await gotoRoute(page, 'proof');
  const title = page.locator('[data-testid="page-header"] h1');
  // No click: the page opened on an item, not on a "Select a proof pack" pane.
  await expect(title).toHaveText(/Chrome proof [AB]/, { timeout: 20_000 });
  await expect(page.getByText('Select a proof pack')).toHaveCount(0);

  // Pick the other pack, reload → that one is restored.
  const current = (await title.textContent())?.trim() ?? '';
  const other = current.endsWith('A') ? 'Chrome proof B' : 'Chrome proof A';
  await page.getByText(other, { exact: true }).first().click();
  await expect(title).toHaveText(other);
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await expect(title).toHaveText(other, { timeout: 20_000 });
  expect([packA, packB]).toContain(await page.evaluate(() => localStorage.getItem('otto.lastSelection.proof')));
});
