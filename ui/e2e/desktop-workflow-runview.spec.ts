import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Workflow run-view UI E2E (design 2026-07-02). Focused on the run-detail
// affordances added for this work:
//   - R1: the Context-files sidebar (agents-style file viewer, files-only) shows
//     a run's context dir, opens a file, and collapses/expands.
//   - R6: the run detail is resizable (maximize toggle) and a step can be zoomed
//     into a large modal to read its full JSON output.
//   - R7: an active run can be canceled from the run view (no session needed).
//   - R8: the run timeline top row renders un-clipped.
// Desktop-browser only. Runs drive the ISOLATED OTTO_E2E daemon (agent turns are
// stubbed) so they complete offline.

const V1 = '/api/v1';
let base = '';
let ctx: APIRequestContext;
let ws = '';

interface Node {
  id: string;
  kind: string;
  name?: string;
  x?: number;
  y?: number;
  params?: unknown;
}
interface Edge {
  id: string;
  source: string;
  target: string;
}
function node(id: string, kind: string, params?: unknown): Node {
  return { id, kind, name: id, x: 0, y: 0, params: params ?? null };
}
function edge(source: string, target: string): Edge {
  return { id: `${source}-${target}`, source, target };
}

async function createWorkflow(name: string, nodes: Node[], edges: Edge[]): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: { name, description: 'e2e', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}
async function startRun(wfId: string, input?: unknown): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, {
    data: input === undefined ? {} : { input },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}
async function waitRun(runId: string, deadlineMs = 90_000): Promise<{ status: string }> {
  const deadline = Date.now() + deadlineMs;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
    expect(g.ok(), await g.text()).toBeTruthy();
    const run = await g.json();
    if (run.status !== 'running' && run.status !== 'pending') return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish: ${run.status}`);
    await new Promise((res) => setTimeout(res, 400));
  }
}

// Open a specific COMPLETED run in the run detail via the Runs dropdown.
async function openCompletedRun(page: Page, wfId: string): Promise<void> {
  await page.goto('/#/workflows');
  await page.getByTestId(`wf-row-${wfId}`).locator('.row-main').click();
  await page.getByRole('button', { name: 'Runs' }).click();
  await page.getByTestId('run-item').first().click();
  await expect(page.locator('.timeline')).toBeVisible({ timeout: 15_000 });
}

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_wf_ctx_open', '1');
  }, ws);
});

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

test('context-files sidebar shows the run dir, opens a file, and collapses (R1)', async ({
  page,
}) => {
  const { dir } = await seedGitRepo(ctx, base, ws);
  const wfId = await createWorkflow(
    'E2E RunView Ctx',
    [node('trigger', 'manual_trigger'), node('set', 'transform', { json: { note: 'ctx' } }), node('tail', 'log')],
    [edge('trigger', 'set'), edge('set', 'tail')],
  );
  const runId = await startRun(wfId, { repos: [{ repo: dir, type: 'worktree', name: dir }] });
  expect((await waitRun(runId)).status).toBe('success');

  await openCompletedRun(page, wfId);

  const sidebar = page.getByTestId('ctx-sidebar');
  await expect(sidebar).toBeVisible({ timeout: 15_000 });
  // The run's context dir is browsable — repos.json is a top-level file.
  await expect(sidebar.getByText('repos.json')).toBeVisible({ timeout: 10_000 });
  // Opening it shows the read-only viewer pane.
  await sidebar.getByText('repos.json').click();
  await expect(sidebar.locator('.viewer-pane')).toBeVisible({ timeout: 10_000 });

  // The header toggle collapses the sidebar entirely (no separate rail — that
  // used to double up against the shell's right bar) and re-expands it. The
  // toggle is always present with the run; the in-sidebar close button is a
  // distinct `ctx-collapse`.
  await page.getByTestId('ctx-sidebar-toggle').click();
  await expect(page.getByTestId('ctx-sidebar')).toHaveCount(0);
  await page.getByTestId('ctx-sidebar-toggle').click();
  await expect(page.getByTestId('ctx-sidebar')).toBeVisible();
});

test('run detail: un-clipped timeline, maximize toggle, and step zoom modal (R6/R8)', async ({
  page,
}) => {
  const wfId = await createWorkflow(
    'E2E RunView Zoom',
    [
      node('trigger', 'manual_trigger'),
      node('set', 'transform', { json: { deposit_bonus: { template: 'welcome', amount: 100 } } }),
      node('tail', 'log'),
    ],
    [edge('trigger', 'set'), edge('set', 'tail')],
  );
  expect((await waitRun(await startRun(wfId))).status).toBe('success');

  await openCompletedRun(page, wfId);

  // R8: the timeline row renders at full height (overflow-x used to zero its flex
  // min-height and let the column compress it — a clipped first line).
  const box = await page.locator('.timeline').boundingBox();
  expect(box, 'timeline has a box').not.toBeNull();
  expect(box!.height).toBeGreaterThanOrEqual(24);

  // R6: maximize toggles the run detail.
  const maxBtn = page.getByTestId('run-detail-max');
  await expect(maxBtn).toHaveAttribute('aria-pressed', 'false');
  await maxBtn.click();
  await expect(maxBtn).toHaveAttribute('aria-pressed', 'true');
  await maxBtn.click();
  await expect(maxBtn).toHaveAttribute('aria-pressed', 'false');

  // R6: zoom the transform step (2nd) → a large modal with its full JSON output.
  await page.locator('.zoom-btn').nth(1).click();
  const dialog = page.locator('.sheet[role="dialog"]');
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('h2')).toContainText('Step ·');
  await expect(dialog.getByText('deposit_bonus')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
});

test('cancel a running run from the run view (R7)', async ({ page }) => {
  const wfId = await createWorkflow(
    'E2E RunView Cancel',
    [node('trigger', 'manual_trigger'), node('wait', 'delay', { ms: 15000 }), node('done', 'log')],
    [edge('trigger', 'wait'), edge('wait', 'done')],
  );
  const runId = await startRun(wfId);

  await page.goto('/#/workflows');
  // Open from the Running list (it's an active run).
  const running = page.getByTestId('running-workflows');
  await expect(running).toBeVisible({ timeout: 15_000 });
  await running.getByText('E2E RunView Cancel').click();

  // The Cancel affordance appears for an active run…
  const cancel = page.getByTestId('run-cancel');
  await expect(cancel).toBeVisible({ timeout: 10_000 });
  await cancel.click();

  // …and the run reaches the canceled terminal state (poll before the delay ends).
  await expect
    .poll(
      async () => (await (await ctx.get(`${base}${V1}/workflow-runs/${runId}`)).json()).status,
      { timeout: 12_000 },
    )
    .toBe('canceled');
});
