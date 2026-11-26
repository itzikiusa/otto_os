import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openEditor, openStartForm as startForm, runPage, runRow, stepPane, stepRow } from './workflowsUi';

// Live-run view stability E2E. A workflow run must stream step changes into its
// run page WITHOUT resetting what the user is looking at: an open step stays
// open (same DOM element — updates are merged, not remounted), switching to
// view another run is never stomped by an in-flight run's updates, the run
// `rev` is monotonic (the UI's stale-snapshot guard), and a human-approval pause
// is announced promptly (pause/approve emit events; nothing waits on a poll).

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
  condition?: string;
}

function node(id: string, kind: string, params?: unknown): Node {
  return { id, kind, name: id, x: 0, y: 0, params: params ?? null };
}
function edge(source: string, target: string, condition?: string): Edge {
  return { id: `${source}-${target}`, source, target, condition };
}

async function createWorkflow(name: string, nodes: Node[], edges: Edge[]): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: { name, description: 'e2e-live', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function startRun(wfId: string): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: {} });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function getRun(runId: string): Promise<any> {
  const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
  expect(g.ok(), await g.text()).toBeTruthy();
  return g.json();
}

async function waitRun(runId: string, timeoutMs = 60_000): Promise<any> {
  const deadline = Date.now() + timeoutMs;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const run = await getRun(runId);
    if (run.status !== 'running' && run.status !== 'pending') return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish: ${run.status}`);
    await new Promise((res) => setTimeout(res, 300));
  }
}

/** Open Workflows (the Runs inbox) with the seeded workspace + collapsed rail. */
async function gotoWorkflows(page: Page): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  await page.goto('/#/workflows');
  await expect(page.getByTestId('wf-tab-runs')).toHaveAttribute('aria-selected', 'true', { timeout: 15_000 });
}

/** The editor's Run…: the start form. */
async function openStartForm(page: Page, wfId: string) {
  await openEditor(page, wfId);
  return startForm(page);
}

test.beforeEach(async ({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
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

test('an open step survives live step transitions — same DOM node, no reset', async ({
  page,
}) => {
  const wfId = await createWorkflow(
    'E2E LiveStable',
    [
      node('trigger', 'manual_trigger'),
      node('w1', 'delay', { ms: 3000 }),
      node('l1', 'log'),
      node('w2', 'delay', { ms: 3000 }),
      node('l2', 'log'),
    ],
    [edge('trigger', 'w1'), edge('w1', 'l1'), edge('l1', 'w2'), edge('w2', 'l2')],
  );
  // Navigate FIRST so the page is up before the (short) run starts — under
  // parallel workers a pre-started run can finish before the page loads.
  await gotoWorkflows(page);
  const runId = await startRun(wfId);
  // The daemon caps concurrent runs: under parallel workers the run can sit in
  // the queue (`pending`) a while. Its steps exist only once it has started.
  await expect.poll(async () => (await getRun(runId)).status, { timeout: 30_000 }).not.toBe('pending');
  const row = page.getByTestId('wf-runs-running').locator(`[data-testid="wf-run-row"][data-run-id="${runId}"]`);
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();
  const run = runPage(page, runId);
  await expect(run).toBeVisible({ timeout: 10_000 });

  // Open a LATER step (not the default pick) while the run is still going:
  // the pick must survive live updates.
  const l1 = stepRow(page, 'l1');
  await l1.click();
  await expect(l1).toHaveAttribute('aria-current', 'true');
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'l1');

  // Tag the open pane and the finished trigger's row so we can prove they are
  // UPDATED IN PLACE, not recreated, across subsequent step transitions.
  await page.evaluate(() => {
    document.querySelector('[data-testid="wf-step-pane"]')?.setAttribute('data-e2e-tag', 'kept');
    document.querySelector('[data-testid="wf-step"][data-node-id="trigger"]')?.setAttribute('data-e2e-tag', 'kept');
  });

  // Let the run finish (several node start/finish events flow through).
  await expect(run).toHaveAttribute('data-state', 'succeeded', { timeout: 30_000 });

  // The user's view was never reset: same step open, same DOM elements.
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'l1');
  await expect(stepPane(page)).toHaveAttribute('data-e2e-tag', 'kept');
  await expect(stepRow(page, 'trigger')).toHaveAttribute('data-e2e-tag', 'kept');
  await expect(l1).toHaveAttribute('aria-current', 'true');
});

test('viewing another run is not stomped by an in-flight run started from the page', async ({
  page,
}) => {
  // First run (via API, no input) SUCCEEDS; the second (started from the page,
  // with fail=true) takes the failing branch — so a stomped view is unmistakable
  // (the viewed 'success' run would flip to running/error).
  const wfId = await createWorkflow(
    'E2E Stomp',
    [
      node('trigger', 'manual_trigger'),
      node('w', 'delay', { ms: 4000 }),
      node('bad', 'http_request', { method: 'GET', url: 'http://127.0.0.1:9/refused' }),
      node('done', 'log'),
    ],
    [
      edge('trigger', 'w'),
      edge('w', 'bad', 'run.input.fail == true'),
      edge('w', 'done', 'run.input.fail != true'),
    ],
  );
  // A finished, SUCCESSFUL run to inspect later.
  const oldRunId = await startRun(wfId);
  const oldRun = await waitRun(oldRunId);
  expect(oldRun.status).toBe('success');

  await gotoWorkflows(page);
  // Start a NEW run from the page with the failing input.
  const form = await openStartForm(page, wfId);
  // W2: the run input opens EMPTY, a placeholder value is refused inline, and
  // the form says where the results go before anything is sent.
  await expect(form.getByTestId('wf-start-field-msg')).toHaveValue('');
  await form.getByText('Input as JSON').click();
  const json = form.getByTestId('wf-start-json');
  await expect(json).toHaveValue('{}');
  await expect(form.getByTestId('wf-start-send-to')).toHaveValue('page');
  await json.fill('{ "result_channel": "slack", "result_chat": "<channel id — optional>" }');
  await page.getByTestId('wf-start-run').click();
  await expect(form).toContainText('result_chat');
  await expect(form).toBeVisible();
  await json.fill('{ "fail": true }');
  await page.getByTestId('wf-start-run').click();
  // Run opens the new run's page, live.
  await expect(form).toHaveCount(0, { timeout: 10_000 });
  const fresh = page.locator('[data-testid="wf-run-page"]');
  await expect(fresh).toHaveAttribute('data-state', /running|queued/, { timeout: 10_000 });
  await expect(fresh).not.toHaveAttribute('data-run-id', oldRunId);

  // Now open the OLD, completed run from the Runs inbox — the user wants to
  // inspect it while the new one keeps running in the background.
  await page.getByRole('button', { name: 'Back to runs' }).click();
  await runRow(page, oldRunId).click();
  const old = runPage(page, oldRunId);
  await expect(old).toHaveAttribute('data-state', 'succeeded', { timeout: 10_000 });

  // The in-flight run's updates must NOT replace the viewed (completed) run —
  // across its mid-run ticks AND its (failing) completion.
  await page.waitForTimeout(3000); // ui-guards: allow — proves the viewed run is NOT replaced mid-run
  await expect(old).toHaveAttribute('data-state', 'succeeded');
  await page.waitForTimeout(3500); // ui-guards: allow — proves it is NOT replaced when the other run fails
  await expect(old).toHaveAttribute('data-state', 'succeeded');
  await expect(page.locator('[data-testid="wf-step"][data-status="error"]')).toHaveCount(0);
});

test('a run whose start response lands after its first live events still goes live', async ({
  page,
}) => {
  // The engine starts the run before POST /run answers, so on a busy daemon
  // its first workflow_run_updated events go out before the page has the run.
  // The run page must not then sit on a stale "Queued" until the run's NEXT
  // event (CI: Queued for the whole 4 s step, then Failed).
  const wfId = await createWorkflow(
    'E2E Late start',
    [node('trigger', 'manual_trigger'), node('w', 'delay', { ms: 6000 }), node('done', 'log')],
    [edge('trigger', 'w'), edge('w', 'done')],
  );
  await gotoWorkflows(page);
  await openStartForm(page, wfId);
  // Hold the start response until the run is demonstrably running (its first
  // events have been broadcast), then hand the page the creation snapshot.
  let heldUntil = 0;
  await page.route(`**/workflows/${wfId}/run`, async (route) => {
    const response = await route.fetch();
    const created = (await response.json()) as { id: string };
    // Generous: on a busy daemon the run may first wait in the run queue.
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      const cur = await getRun(created.id);
      if (cur.status === 'running' && (cur.rev ?? 0) > 1) break;
      await new Promise((res) => setTimeout(res, 100));
    }
    heldUntil = Date.now();
    await route.fulfill({ response, json: created });
  });
  await page.getByTestId('wf-start-run').click();
  const run = page.locator('[data-testid="wf-run-page"]');
  await expect(run).toBeVisible({ timeout: 35_000 });
  // Well inside the 6 s step: no further event arrives before it ends.
  await expect(run).toHaveAttribute('data-state', 'running', { timeout: 4_000 });
  expect(heldUntil, 'the start response was held').toBeGreaterThan(0);
});

test('run rev is monotonic while running (stale-snapshot guard contract)', async () => {
  const wfId = await createWorkflow(
    'E2E Rev',
    [
      node('trigger', 'manual_trigger'),
      node('w1', 'delay', { ms: 1200 }),
      node('l1', 'log'),
      node('w2', 'delay', { ms: 1200 }),
    ],
    [edge('trigger', 'w1'), edge('w1', 'l1'), edge('l1', 'w2')],
  );
  const runId = await startRun(wfId);
  // The daemon caps concurrent runs: under parallel workers this run can sit
  // in the FIFO queue (`pending`, rev 0) for a while. The contract is about a
  // RUNNING run, so sample only once it has started.
  await expect.poll(async () => (await getRun(runId)).status, { timeout: 30_000 }).not.toBe('pending');
  const revs: number[] = [];
  for (let i = 0; i < 5; i++) {
    const run = await getRun(runId);
    expect(typeof run.rev, 'WorkflowRun carries a rev').toBe('number');
    revs.push(run.rev);
    await new Promise((res) => setTimeout(res, 700));
  }
  for (let i = 1; i < revs.length; i++) {
    expect(revs[i], `rev never goes backward (${revs.join(',')})`).toBeGreaterThanOrEqual(
      revs[i - 1],
    );
  }
  expect(revs[revs.length - 1], 'rev advances as steps transition').toBeGreaterThan(revs[0]);
  await waitRun(runId);
});

test('human-approval pause is announced promptly; approve resumes to success', async ({
  page,
}) => {
  const wfId = await createWorkflow(
    'E2E Approve',
    [
      node('trigger', 'manual_trigger'),
      node('gate', 'human_approval', { prompt: 'ok to continue?' }),
      node('done', 'log'),
    ],
    [edge('trigger', 'gate'), edge('gate', 'done')],
  );

  await gotoWorkflows(page);
  const runId = await startRun(wfId);
  // A queued run (concurrency cap, parallel workers) has not paused yet.
  await expect.poll(async () => (await getRun(runId)).status, { timeout: 30_000 }).not.toBe('pending');

  // The pause must surface in the inbox's "Needs you" fold promptly — driven
  // by the pause EVENT, not by waiting for a poll to happen by.
  const row = page.getByTestId('wf-runs-needs').locator(`[data-testid="wf-run-row"][data-run-id="${runId}"]`);
  await expect(row).toBeVisible({ timeout: 5_000 });
  await expect(row).toHaveAttribute('data-state', 'needs_you');

  // Open the run: the approval card is up; approve it.
  await row.click();
  const run = runPage(page, runId);
  const card = run.locator('[data-testid="wf-run-card"][data-kind="approval"]');
  await expect(card).toBeVisible({ timeout: 10_000 });
  await card.getByRole('button', { name: 'Approve' }).click();

  // The run resumes and completes IN PLACE (no re-navigation), the card drops.
  await expect(run).toHaveAttribute('data-state', 'succeeded', { timeout: 20_000 });
  await expect(card).toHaveCount(0);
  const done = await getRun(runId);
  expect(done.status).toBe('success');
  expect((done.nodes ?? []).find((n: any) => n.node_id === 'gate').status).toBe('success');
});

test('the failed step opens by itself; a person\'s pick is never fought by updates', async ({
  page,
}) => {
  // The failing node sits on its OWN branch so the delay branch keeps producing
  // transitions after the error lands (that's what used to fight the person).
  const wfId = await createWorkflow(
    'E2E ErrCollapse',
    [
      node('trigger', 'manual_trigger'),
      node('bad', 'http_request', { method: 'GET', url: 'http://127.0.0.1:9/refused' }),
      node('w1', 'delay', { ms: 4000 }),
      node('l1', 'log'),
    ],
    [edge('trigger', 'bad'), edge('trigger', 'w1'), edge('w1', 'l1')],
  );
  // Navigate first (see the LiveStable test) — the run is short.
  await gotoWorkflows(page);
  const runId = await startRun(wfId);
  const row = runRow(page, runId);
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();
  const run = runPage(page, runId);

  // The bad step fails while the delay branch is still going…
  await expect(stepRow(page, 'bad')).toHaveAttribute('data-status', 'error', { timeout: 15_000 });
  // …the person opens another step — and later transitions must not move them.
  await stepRow(page, 'w1').click();
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'w1');
  await expect(run).toHaveAttribute('data-state', 'failed', { timeout: 25_000 }); // run terminal (bad branch failed)
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'w1');
  // The failing step's own error is on the run either way.
  await expect(run.locator('[data-testid="wf-run-card"][data-kind="error"]')).toBeVisible();

  // Opened fresh (no pick), the run opens on the failed step.
  await page.goto(`/#/workflows/${wfId}/runs/${runId}`);
  await expect(runPage(page, runId)).toHaveAttribute('data-state', 'failed', { timeout: 15_000 });
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'bad');
});
