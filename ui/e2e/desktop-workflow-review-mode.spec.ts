import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// review_run EXECUTION MODE E2E (workflows batch, R2).
//   - The node inspector's "Execution mode" select defaults to *Default* (the
//     `mode` key is absent from params), persists `fan_out`/`orchestrator` through
//     a save + reload, and removes the key again when set back to Default.
//   - A run-level `review_mode` (the Run dialog's row / the `/run` body) OVERRIDES
//     every review_run node in that run: the step's first log line names the
//     resolved mode AND its source, and the run input carries the override.
//   - The two 400s of the wire contract.
// Desktop-browser only. Runs drive the ISOLATED OTTO_E2E daemon; the `review_run`
// step errors (no repo is provisioned) — that is fine, the mode line is logged
// BEFORE the failure and the engine keeps the live log lines on the error path.
// `RetryPolicy::default()` is `max_attempts: 0`, so the step is not retried.

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
// `name: id` matters: the canvas card renders `n.name || <catalog label>` as its
// title, so the node id is the text a `.node` locator can match on (the raw kind
// string `review_run` is never rendered — the label "Review Run" is).
function node(id: string, kind: string, params?: unknown, x = 0, y = 0): Node {
  return { id, kind, name: id, x, y, params: params ?? null };
}
function edge(source: string, target: string): Edge {
  return { id: `${source}-${target}`, source, target };
}

async function createWorkflow(name: string, nodes: Node[], edges: Edge[]): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: { name, description: 'e2e-review-mode', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

/** POST /workflows/{id}/run with the raw body (so `review_mode` can ride along). */
async function startRun(wfId: string, body: Record<string, unknown> = {}): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: body });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function getRun(runId: string): Promise<any> {
  const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
  expect(g.ok(), await g.text()).toBeTruthy();
  return g.json();
}

async function waitRun(runId: string, timeoutMs = 90_000): Promise<any> {
  const deadline = Date.now() + timeoutMs;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const run = await getRun(runId);
    if (run.status !== 'running' && run.status !== 'pending') return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish: ${run.status}`);
    await new Promise((res) => setTimeout(res, 300));
  }
}

async function getGraph(wfId: string): Promise<any> {
  const g = await ctx.get(`${base}${V1}/workflows/${wfId}`);
  expect(g.ok(), await g.text()).toBeTruthy();
  return (await g.json()).graph;
}
/** The saved `params` of one node (`{}` when the node has none). */
async function nodeParams(wfId: string, nodeId: string): Promise<Record<string, unknown>> {
  const g = await getGraph(wfId);
  const n = (g.nodes as Node[]).find((x) => x.id === nodeId);
  expect(n, `node ${nodeId} in the saved graph`).toBeTruthy();
  return (n!.params as Record<string, unknown>) ?? {};
}
/** The logs of one node of a finished run. */
function nodeLogs(run: any, nodeId: string): string[] {
  const n = (run.nodes ?? []).find((x: any) => x.node_id === nodeId);
  expect(n, `node ${nodeId} in run ${run.id}`).toBeTruthy();
  return (n.logs ?? []) as string[];
}

/** Open the workflow's editor (canvas + inspector) from the list. */
async function openWorkflow(page: Page, wfId: string): Promise<void> {
  await page.goto('/#/workflows');
  await page.getByTestId(`wf-row-${wfId}`).locator('.row-main').click();
  await expect(page.locator('.node').first()).toBeVisible({ timeout: 15_000 });
}

/** Select a canvas node by its rendered title (= the node id, see `node()`). */
async function selectNode(page: Page, id: string): Promise<void> {
  await page.locator('.node', { hasText: id }).click();
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

test('inspector defaults to Default and persists mode', async ({ page }) => {
  const wfId = await createWorkflow(
    'E2E Review Mode Inspector',
    [
      node('trigger', 'manual_trigger', null, 40, 40),
      node('impl', 'agent_prompt', { prompt: 'summarise the diff' }, 300, 40),
      node('review', 'review_run', null, 560, 40),
    ],
    [edge('trigger', 'impl'), edge('impl', 'review')],
  );

  await openWorkflow(page, wfId);
  await selectNode(page, 'review');

  // Default = no `mode` key at all, so a run-level override and the stored
  // PR-review config still get their say.
  const sel = page.getByTestId('review-mode-select');
  await expect(sel).toBeVisible({ timeout: 10_000 });
  await expect(sel).toHaveValue('');
  await expect(sel.locator('option:checked')).toHaveText(
    'Default (fan-out unless the run overrides it)',
  );
  expect(await nodeParams(wfId, 'review')).not.toHaveProperty('mode');

  // Orchestrator → saved onto the node's params.
  await sel.selectOption('orchestrator');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect
    .poll(async () => (await nodeParams(wfId, 'review')).mode, { timeout: 10_000 })
    .toBe('orchestrator');

  // …and it survives a reload (the select reads the saved params, not local state).
  await page.reload();
  await openWorkflow(page, wfId);
  await selectNode(page, 'review');
  await expect(page.getByTestId('review-mode-select')).toHaveValue('orchestrator');

  // Back to Default REMOVES the key (it must not persist as `""`).
  await page.getByTestId('review-mode-select').selectOption('');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect
    .poll(async () => 'mode' in (await nodeParams(wfId, 'review')), { timeout: 10_000 })
    .toBe(false);
});

test('run dialog override wins over the node', async ({ page }) => {
  const graph: [Node[], Edge[]] = [
    [
      node('trigger', 'manual_trigger', null, 40, 40),
      node('impl', 'agent_prompt', { prompt: 'summarise the diff' }, 300, 40),
      node('review', 'review_run', { mode: 'orchestrator' }, 560, 40),
    ],
    [edge('trigger', 'impl'), edge('impl', 'review')],
  ];
  const wfId = await createWorkflow('E2E Review Mode Override', graph[0], graph[1]);

  // 1. A run-level override beats the node's own `orchestrator`.
  const overrideRunId = await startRun(wfId, { review_mode: 'fan_out' });
  expect((await getRun(overrideRunId)).input?.review_mode).toBe('fan_out');
  const overrideRun = await waitRun(overrideRunId);
  // The step errors (no repo provisioned) — the mode line is logged first and the
  // engine keeps the live log lines on the error path.
  expect(nodeLogs(overrideRun, 'review')).toContain('review_run: mode fan_out (run override)');

  // 2. The same workflow with no override falls back to the node's setting.
  const plainRun = await waitRun(await startRun(wfId));
  expect(nodeLogs(plainRun, 'review')).toContain('review_run: mode orchestrator (node)');

  // 3. The Run dialog's "Review mode" row posts the same override.
  const known = new Set([overrideRunId, plainRun.id as string]);
  await openWorkflow(page, wfId);
  await page.getByRole('button', { name: 'Run…' }).click();
  const dialogSel = page.getByTestId('run-review-mode');
  await expect(dialogSel).toBeVisible({ timeout: 10_000 });
  await expect(dialogSel).toHaveValue('');
  await dialogSel.selectOption('fan_out');
  await page.locator('.run-input').getByRole('button', { name: 'Run' }).click();

  let dialogRunId = '';
  await expect
    .poll(
      async () => {
        const r = await ctx.get(`${base}${V1}/workflows/${wfId}/runs`);
        const runs = (await r.json()) as Array<{ id: string }>;
        dialogRunId = runs.find((x) => !known.has(x.id))?.id ?? '';
        return dialogRunId;
      },
      { timeout: 20_000 },
    )
    .not.toBe('');
  expect((await getRun(dialogRunId)).input?.review_mode).toBe('fan_out');
  expect(nodeLogs(await waitRun(dialogRunId), 'review')).toContain(
    'review_run: mode fan_out (run override)',
  );

  // 4. The wire contract's two 400s (exact messages). The body is the app's JSON
  // problem document, so the message is read from `message` — the raw text
  // escapes the quotes around the two mode names.
  const bogus = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, {
    data: { review_mode: 'bogus' },
  });
  expect(bogus.status()).toBe(400);
  expect(((await bogus.json()) as { message: string }).message).toContain(
    'review_mode must be "fan_out" or "orchestrator"',
  );

  const badInput = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, {
    data: { input: 5, review_mode: 'fan_out' },
  });
  expect(badInput.status()).toBe(400);
  expect(((await badInput.json()) as { message: string }).message).toContain(
    'input must be a JSON object when review_mode is set',
  );
});
