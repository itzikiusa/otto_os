import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Sub-agent visibility E2E (workflows batch, R1/R5.5). Under `OTTO_E2E` an
// `agent_prompt` whose prompt carries the sentinel line
// `OTTO_E2E_SUBAGENTS: <n> hold_ms=<ms>` makes the engine emit the real phase
// sequence — `⏳ starting claude session`, `✉ prompt accepted`,
// `🧩 sub-agents: n running · 0 done`, `🧩 sub-agents: 0 running · n done`,
// `📄 handoff file written`, `✓ step complete (handoff + idle turn)` — with a
// matching `NodeRunState.activity` snapshot, HOLDING the step for `hold_ms`
// before the canned reply. No live CLI is involved.
//
// What this asserts: the live chip + phase line on the running step card, the
// nested sub-agent rows in the run sidebar's Agents tab (the step has NO session
// of its own yet — the group must still appear), the kept phase lines in the
// finished step's logs IN ORDER, and that `activity` is cleared on finish.
//
// The hold is 12 s so the WS event / 2.5 s fallback poll have room to land every
// DOM state; the e2e daemon is one shared process, so its env cannot be tuned
// per spec — the sentinel carries `hold_ms` instead.

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
function node(id: string, kind: string, params?: unknown, x = 0, y = 0): Node {
  return { id, kind, name: id, x, y, params: params ?? null };
}
function edge(source: string, target: string): Edge {
  return { id: `${source}-${target}`, source, target };
}

async function createWorkflow(name: string, nodes: Node[], edges: Edge[]): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: { name, description: 'e2e-subagents', graph: { nodes, edges } },
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

function nodeState(run: any, nodeId: string): any {
  const n = (run.nodes ?? []).find((x: any) => x.node_id === nodeId);
  expect(n, `node ${nodeId} in run ${run.id}`).toBeTruthy();
  return n;
}

/** Open the workflows page with the seeded workspace, collapsed rail, ctx sidebar on. */
async function gotoWorkflows(page: Page): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_wf_ctx_open', '1');
  }, ws);
  await page.goto('/#/workflows');
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

test('a running step shows its sub-agents; a finished one keeps the phase lines', async ({
  page,
}) => {
  const wfId = await createWorkflow(
    'E2E Subagents',
    [
      node('trigger', 'manual_trigger', null, 40, 40),
      node(
        'impl',
        'agent_prompt',
        { prompt: 'OTTO_E2E_SUBAGENTS: 2 hold_ms=12000\nsummarise the diff' },
        300,
        40,
      ),
    ],
    [edge('trigger', 'impl')],
  );

  // Navigate FIRST so the page is up before the (held, but finite) run starts.
  await gotoWorkflows(page);
  const runId = await startRun(wfId);

  const running = page.getByTestId('running-workflows');
  await expect(running).toBeVisible({ timeout: 15_000 });
  await running.getByText('E2E Subagents').click();

  // Wait on the API — not the DOM — for the synthetic sub-agents to exist, so the
  // DOM assertions below can never race the first WS event / fallback poll.
  await expect
    .poll(async () => nodeState(await getRun(runId), 'impl').activity?.subagents?.length ?? 0, {
      timeout: 15_000,
    })
    .toBe(2);

  // The running step card carries the sub-agent chip + the live phase line.
  const chip = page.getByTestId('subagent-chip');
  await expect(chip).toBeVisible({ timeout: 10_000 });
  await expect(chip).toHaveText(/2 sub-agents · (2 running|2 done)/);
  await expect(page.getByTestId('step-phase')).toBeVisible();

  // The run sidebar's Agents tab nests one row per sub-agent under the step —
  // even though the step has no openable session of its own yet (the stub's
  // `on_ready` only fires once the hold has elapsed).
  await page.getByRole('tab', { name: 'Agents' }).click();
  const rows = page.getByTestId('subagent-rows');
  await expect(rows).toBeVisible({ timeout: 10_000 });
  await expect(rows.locator('.sub')).toHaveCount(2);

  // …and once the step settles, the phase lines are KEPT, in order.
  const run = await waitRun(runId);
  expect(run.status).toBe('success');
  const step = nodeState(run, 'impl');
  const logs = step.logs as string[];

  const running0 = logs.indexOf('🧩 sub-agents: 2 running · 0 done');
  const done2 = logs.indexOf('🧩 sub-agents: 0 running · 2 done');
  const complete = logs.indexOf('✓ step complete (handoff + idle turn)');
  expect(running0, `"2 running · 0 done" in ${JSON.stringify(logs)}`).toBeGreaterThanOrEqual(0);
  expect(done2).toBeGreaterThan(running0);
  expect(complete).toBeGreaterThan(done2);
  // `✓ step complete …` is NOT the last line — the turn's own completion line and
  // the persist/edge lines follow it.
  expect(logs).toContain('agent turn complete');
  expect(logs.indexOf('agent turn complete')).toBeGreaterThan(complete);

  // A settled step carries no activity at all (serde skips `None`).
  expect(step.activity ?? null).toBeNull();
});
