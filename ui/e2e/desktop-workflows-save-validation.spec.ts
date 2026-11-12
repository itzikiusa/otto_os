import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Workflow editor save rules (Slack runs whatever is SAVED, so a save is a
// publish):
//   - Tidy only re-lays the canvas and marks it unsaved — it never saves
//     (it used to PATCH the whole graph, half-made edits included).
//   - Save validates first: a graph with errors is shown, not saved.
//   - The Add-node palette filters by what a node does, grouped by category.
// Desktop-browser only; drives the ISOLATED OTTO_E2E daemon.

const V1 = '/api/v1';
let base = '';
let ctx: APIRequestContext;
let ws = '';

async function createWorkflow(name: string): Promise<string> {
  // `fetch` has no `url` — the validator's "set url before running" error.
  const nodes = [
    { id: 'start', kind: 'manual_trigger', name: 'start', x: 40, y: 40, params: null },
    { id: 'fetch', kind: 'http_request', name: 'fetch', x: 40, y: 400, params: {} },
  ];
  const edges = [{ id: 'start-fetch', source: 'start', target: 'fetch' }];
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: { name, description: 'e2e-save-validation', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function savedNode(wfId: string, id: string): Promise<{ x: number; y: number }> {
  const g = await ctx.get(`${base}${V1}/workflows/${wfId}`);
  expect(g.ok(), await g.text()).toBeTruthy();
  return ((await g.json()).graph.nodes as { id: string; x: number; y: number }[]).find((n) => n.id === id)!;
}

async function openWorkflow(page: Page, wfId: string): Promise<void> {
  await page.goto('/#/workflows');
  await page.getByTestId(`wf-row-${wfId}`).locator('.row-main').click();
  await expect(page.locator('.node').first()).toBeVisible({ timeout: 15_000 });
}

test.beforeAll(async ({}, testInfo) => {
  if (testInfo.project.name !== 'desktop-browser') return;
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  ws = await seedWorkspace(ctx, base);
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
});

test('Tidy leaves the change unsaved, and Save refuses a graph with errors', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const wfId = await createWorkflow('E2E Save Validation');
  await openWorkflow(page, wfId);
  const header = page.locator('[data-testid="page-header"]');

  await header.getByRole('button', { name: 'More actions' }).click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Tidy' }).click();

  // Re-laid out locally, marked unsaved — the stored graph is untouched.
  const save = header.getByRole('button', { name: 'Save', exact: true });
  await expect(save).toBeEnabled();
  await page.waitForTimeout(500); // ui-guards: allow — proves an absence: Tidy must NOT have saved by now
  expect(await savedNode(wfId, 'fetch')).toMatchObject({ x: 40, y: 400 });

  // Save validates: the issue is shown and nothing is written.
  await save.click();
  await expect(page.getByText('Resolve these issues before saving or running').first()).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('.preflight').getByRole('button', { name: /url/i })).toBeVisible();
  await expect(save).toBeEnabled();
  expect(await savedNode(wfId, 'fetch')).toMatchObject({ x: 40, y: 400 });
});

test('the node palette filters by description and groups by category', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const wfId = await createWorkflow('E2E Palette Filter');
  await openWorkflow(page, wfId);

  await page.locator('[data-testid="page-header"] button', { hasText: 'Node' }).click();
  const palette = page.getByRole('dialog', { name: 'Add a node' });
  await expect(palette).toBeVisible();
  const filter = palette.getByRole('searchbox', { name: 'Filter nodes' });
  await expect(filter).toBeFocused();
  // Category headings are shown, descriptions sit under the labels.
  await expect(palette.locator('.np-cat', { hasText: 'Flow' })).toBeVisible();
  await expect(palette.locator('.pal-item', { hasText: 'Delay' }).locator('.pal-desc')).toBeVisible();

  // "milliseconds" is only in Delay's description, not in any label.
  await filter.fill('milliseconds');
  await expect(palette.locator('.pal-item')).toHaveCount(1);
  await expect(palette.locator('.pal-item .pal-name')).toHaveText('Delay');

  await filter.fill('zzzz no such node');
  await expect(palette.getByText('No node matches “zzzz no such node”.')).toBeVisible();
});
