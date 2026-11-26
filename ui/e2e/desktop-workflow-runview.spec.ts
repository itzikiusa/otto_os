import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';
import { openRun, runsTab, stepPane, stepRow } from './workflowsUi';

// Workflow run-page UI E2E (design 2026-07-02, re-pointed to the run page of
// ux.md §4). Focused on the run affordances added for this work:
//   - R1: the run's context files are browsable from the run (the Result's
//     Files tab) and a file opens read-only in place.
//   - R6: the step pane maximizes (the run column folds behind "Run") and a
//     step's full JSON output is readable in its Output tab.
//   - R7: an active run can be stopped from its page (no session needed).
//   - R8: the step rows render un-clipped.
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

// Open a specific COMPLETED run on its page.
async function openCompletedRun(page: Page, wfId: string, runId: string): Promise<void> {
  const run = await openRun(page, wfId, runId);
  await expect(run).toHaveAttribute('data-state', 'succeeded', { timeout: 15_000 });
}

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
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

test('the run\'s context files are browsable from the run and open in place (R1)', async ({
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

  await openCompletedRun(page, wfId, runId);

  // A finished run opens on its Result; its Files tab is the run's context dir —
  // repos.json is a top-level file.
  const pane = stepPane(page);
  await expect(pane).toHaveAttribute('data-node-id', '_result', { timeout: 15_000 });
  await pane.getByTestId('wf-step-tab-files').click();
  const file = pane.getByRole('button', { name: 'repos.json' });
  await expect(file).toBeVisible({ timeout: 10_000 });
  // Opening it shows its text, read-only, in place.
  await file.click();
  await expect(pane.locator('pre.content')).toBeVisible({ timeout: 10_000 });
});

test('run page: un-clipped step rows, the pane\'s maximize, and a step\'s full output (R6/R8)', async ({
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
  const runId = await startRun(wfId);
  expect((await waitRun(runId)).status).toBe('success');

  await openCompletedRun(page, wfId, runId);

  // R8: a step row renders at full height (a compressed flex row used to clip
  // its first line).
  const box = await stepRow(page, 'set').boundingBox();
  expect(box, 'step row has a box').not.toBeNull();
  expect(box!.height).toBeGreaterThanOrEqual(24);

  // R6: open the transform step; its Output tab holds the full JSON output.
  await stepRow(page, 'set').click();
  const pane = stepPane(page);
  await expect(pane).toHaveAttribute('data-node-id', 'set');
  await pane.getByTestId('wf-step-tab-output').click();
  await expect(pane.getByText('deposit_bonus')).toBeVisible({ timeout: 10_000 });

  // R6: maximize folds the run column behind "Run"; it comes back.
  await pane.getByRole('button', { name: 'Maximize conversation' }).click();
  await expect(stepRow(page, 'set')).toHaveCount(0);
  await expect(pane.getByRole('button', { name: 'Show the run' })).toBeVisible();
  await page.getByRole('button', { name: 'Run', exact: true }).click();
  await expect(stepRow(page, 'set')).toBeVisible();
  await expect(pane.getByRole('button', { name: 'Maximize conversation' })).toBeVisible();
});

test('stop a running run from its page (R7)', async ({ page }) => {
  // The engine caps ONE delay node at 10s, so a single `ms: 15000` node kept
  // the run active for only 10s — less than a cold page load under a full
  // parallel e2e run, after which the run had already finished. Three capped
  // delays keep it active for ~30s; the stop below ends it long before.
  const wfId = await createWorkflow(
    'E2E RunView Cancel',
    [
      node('trigger', 'manual_trigger'),
      node('wait1', 'delay', { ms: 10000 }),
      node('wait2', 'delay', { ms: 10000 }),
      node('wait3', 'delay', { ms: 10000 }),
      node('done', 'log'),
    ],
    [edge('trigger', 'wait1'), edge('wait1', 'wait2'), edge('wait2', 'wait3'), edge('wait3', 'done')],
  );

  // Load the page first, THEN start the run: the Runs inbox must pick a run
  // up live (WS `workflow_run_updated`), and the run's clock no longer races
  // the page load.
  await runsTab(page);
  await expect(page.getByTestId('wf-tab-runs')).toHaveAttribute('aria-selected', 'true', { timeout: 30_000 });
  const runId = await startRun(wfId);

  // Open it from the Running fold (it's an active run).
  const running = page.getByTestId('wf-runs-running');
  const row = running.locator(`[data-testid="wf-run-row"][data-run-id="${runId}"]`);
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();

  // The Stop affordance appears for an active run — one Stop, saying what it does.
  const stop = page.getByTestId('wf-run-stop');
  await expect(stop).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole('button', { name: /Stop run/ })).toHaveCount(1);
  await expect(stop).toHaveText(/Stop run…/);
  await stop.click();
  // Stopping can't be undone, so it asks first.
  await page.getByRole('dialog').getByRole('button', { name: 'Stop run', exact: true }).click();

  // …and the run reaches the canceled terminal state (poll well before the
  // ~30s of delays could finish it on their own).
  await expect
    .poll(
      async () => (await (await ctx.get(`${base}${V1}/workflow-runs/${runId}`)).json()).status,
      { timeout: 12_000 },
    )
    .toBe('canceled');
  await expect(page.locator(`[data-testid="wf-run-page"][data-run-id="${runId}"]`)).toHaveAttribute('data-state', 'canceled', { timeout: 10_000 });
  // The Running fold lets go of it.
  await page.getByRole('button', { name: 'Back to runs' }).click();
  await expect(running.locator(`[data-testid="wf-run-row"][data-run-id="${runId}"]`)).toHaveCount(0, { timeout: 10_000 });
});

// R5.5 (workflows batch): `NodeRunState.activity` is a LIVE field — the engine
// publishes a phase + sub-agent snapshot while an agent step runs and clears it
// the moment the step settles. A finished run must therefore carry no `activity`
// key at all on any node (serde skips `None`), so the run view's sub-agent chip
// and phase line can never linger on a settled step.
test('a finished step carries no activity', async () => {
  const wfId = await createWorkflow(
    'E2E RunView Activity',
    [
      node('trigger', 'manual_trigger'),
      node('set', 'transform', { json: { note: 'settled' } }),
      node('tail', 'log'),
    ],
    [edge('trigger', 'set'), edge('set', 'tail')],
  );
  const runId = await startRun(wfId);
  expect((await waitRun(runId)).status).toBe('success');

  const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
  expect(g.ok(), await g.text()).toBeTruthy();
  const nodes = ((await g.json()).nodes ?? []) as Array<Record<string, unknown>>;

  expect(nodes.map((n) => n.node_id)).toEqual(['trigger', 'set', 'tail']);
  for (const n of nodes) {
    expect(n.status, `node ${n.node_id}`).toBe('success');
    expect(
      Object.prototype.hasOwnProperty.call(n, 'activity'),
      `node ${n.node_id} still carries activity`,
    ).toBe(false);
  }
});
