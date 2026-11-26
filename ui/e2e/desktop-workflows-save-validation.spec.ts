import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { canvasNode, openEditor, saveEditor } from './workflowsUi';

// Workflow editor save rules (Slack runs whatever is SAVED, so a save is a
// publish):
//   - An edit (here "+ Step") only marks the editor unsaved — it never saves
//     (Tidy used to PATCH the whole graph, half-made edits included).
//   - Save validates first: a draft's missing piece (a `run` issue) is shown
//     and the draft saves (DR-028); a `save` issue is shown and nothing saves.
//   - "+ Step" is a filterable menu, each step with its category.
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

async function savedNodes(wfId: string): Promise<{ id: string; kind: string }[]> {
  const g = await ctx.get(`${base}${V1}/workflows/${wfId}`);
  expect(g.ok(), await g.text()).toBeTruthy();
  return (await g.json()).graph.nodes as { id: string; kind: string }[];
}

async function openWorkflow(page: Page, wfId: string): Promise<void> {
  await openEditor(page, wfId);
  await expect(canvasNode(page, 'fetch')).toBeVisible({ timeout: 15_000 });
}

/** The editor header's "+ Step" menu. */
async function openStepMenu(page: Page): Promise<void> {
  await page.locator('[data-testid="page-header"]').getByRole('button', { name: 'Step', exact: true }).click();
  await expect(page.locator('.ctx-menu')).toBeVisible();
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

test('an edit leaves the change unsaved; Save keeps a draft with run issues and refuses a save issue', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const wfId = await createWorkflow('E2E Save Validation');
  await openWorkflow(page, wfId);
  const header = page.locator('[data-testid="page-header"]');

  // Add a step: it lands on the canvas, the editor says Unsaved — the stored
  // graph is untouched.
  await openStepMenu(page);
  await page.locator('.ctx-menu').getByRole('textbox', { name: 'Find a step…' }).fill('delay');
  await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Delay\b/ }).click();
  await expect(page.locator('[data-testid="wf-node"][data-kind="delay"]')).toHaveCount(1);
  await expect(header.getByText('Unsaved')).toBeVisible();
  const save = header.getByRole('button', { name: 'Save', exact: true });
  await expect(save).toBeEnabled();
  await page.waitForTimeout(500); // ui-guards: allow — proves an absence: the edit must NOT have saved by now
  expect((await savedNodes(wfId)).map((n) => n.id)).toEqual(['start', 'fetch']);

  // A save-class issue (stubbed: the editor cannot draw a cycle here) is shown
  // above the canvas and nothing is written.
  await page.route('**/api/v1/workflows/*/validate', (r) => r.fulfill({ json: { valid: false, issues: [{ node_id: 'fetch', edge_id: null, field: 'edges', message: 'This step belongs to or follows a cycle.', class: 'save' }] } }));
  await saveEditor(page);
  await expect(page.getByText('Not saved — resolve the problems above the canvas').first()).toBeVisible({ timeout: 10_000 });
  const problems = page.getByTestId('wf-editor-problems');
  await expect(problems).toContainText(/1 problem ·/);
  await expect(header.getByText('Unsaved')).toBeVisible();
  expect((await savedNodes(wfId)).map((n) => n.id)).toEqual(['start', 'fetch']);

  // The real validator: `fetch` has no url — a run issue. The draft saves, the
  // issue stays listed.
  await page.unroute('**/api/v1/workflows/*/validate');
  await saveEditor(page);
  await expect(header.getByText('Unsaved')).toBeHidden({ timeout: 10_000 });
  await expect.poll(async () => (await savedNodes(wfId)).map((n) => n.kind)).toContain('delay');
  await expect(problems).toContainText(/\d+ problems? ·/);
  await expect(problems.getByRole('listitem').filter({ hasText: /url/i })).toBeVisible();
});

test('"+ Step" filters the steps and names each one\'s category', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const wfId = await createWorkflow('E2E Palette Filter');
  await openWorkflow(page, wfId);

  await openStepMenu(page);
  const menu = page.locator('.ctx-menu');
  const filter = menu.getByRole('textbox', { name: 'Find a step…' });
  await expect(filter).toBeFocused();
  await filter.fill('delay');
  const only = menu.getByRole('menuitem');
  await expect(only).toHaveCount(1);
  await expect(only.locator('.ctx-label')).toHaveText('Delay');
  // Each step names its category beside the label.
  await expect(only.locator('.ctx-hint')).toHaveText(/\S/);

  await filter.fill('zzzz no such step');
  await expect(menu.getByRole('menuitem')).toHaveCount(0);
});
