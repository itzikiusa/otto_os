import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Workflow orchestrator E2E. Drives the API against the isolated OTTO_E2E daemon
// (agent turns return a deterministic canned reply + a session id, so agent nodes
// complete offline), then renders the Workflows page in the desktop browser.
//
// The API portion proves the NEW orchestrator behavior end-to-end through the
// real route → engine → repo stack: conditional branching (branch-skip ≠ error),
// the bounded fix→review-style loop, per-step agent SESSIONS surfaced on the run,
// the run→Proof-Pack link, workflow versioning + restore, and converting a
// scheduled task into a workflow. The UI portion is a desktop smoke test.

const V1 = '/api/v1';

let base = '';
let ctx: APIRequestContext;
let ws = '';
let branchWfId = '';
let loopWfId = '';
let sessionWfId = '';

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
    data: { name, description: 'e2e', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

/** Run a workflow and poll its run row to a terminal status. */
async function runToCompletion(wfId: string): Promise<any> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, { data: {} });
  expect(r.ok(), await r.text()).toBeTruthy();
  const runId = (await r.json()).id as string;
  const deadline = Date.now() + 60_000;
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const g = await ctx.get(`${base}${V1}/workflow-runs/${runId}`);
    expect(g.ok(), await g.text()).toBeTruthy();
    const run = await g.json();
    if (run.status !== 'running' && run.status !== 'pending') return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not finish: ${run.status}`);
    await new Promise((res) => setTimeout(res, 500));
  }
}

function nodeState(run: any, id: string): any {
  return (run.nodes ?? []).find((n: any) => n.node_id === id);
}

// Run on the desktop-browser project only (the API assertions don't need a
// browser, and the UI smoke is desktop-only) — mobile projects skip.
test.beforeEach(async ({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
});

test.beforeAll(async ({}, testInfo) => {
  if (testInfo.project.name !== 'desktop-browser') return;
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  ws = await seedWorkspace(ctx, base);

  // Branching workflow: trigger → agent (session) → set score → condition →
  // {passed | failed} via edge conditions on the condition's `result`.
  branchWfId = await createWorkflow(
    'E2E Branching',
    [
      node('trigger', 'manual_trigger'),
      node('agent', 'agent_prompt', { prompt: 'say hi' }),
      node('setscore', 'transform', { json: { score: 90 } }),
      node('cond', 'condition', { expr: 'score >= 80' }),
      node('passed', 'log'),
      node('failed', 'log'),
    ],
    [
      edge('trigger', 'agent'),
      edge('agent', 'setscore'),
      edge('setscore', 'cond'),
      edge('cond', 'passed', 'output.result == true'),
      edge('cond', 'failed', 'output.result == false'),
    ],
  );

  // Loop workflow: a transform step sets score=90; the loop's `until` is met on
  // the first iteration.
  loopWfId = await createWorkflow(
    'E2E Loop',
    [
      node('trigger', 'manual_trigger'),
      node('loopn', 'loop', {
        max_iterations: 3,
        until: 'last.score >= 80',
        steps: [{ kind: 'transform', name: 'set', params: { json: { score: 90 } } }],
      }),
    ],
    [edge('trigger', 'loopn')],
  );

  // Dedicated single-run workflow for the session assertion (its own agent prompt
  // → guaranteed cache miss → the agent actually runs and spawns a session).
  sessionWfId = await createWorkflow(
    'E2E Session',
    [node('trigger', 'manual_trigger'), node('agent', 'agent_prompt', { prompt: 'unique session probe' })],
    [edge('trigger', 'agent')],
  );
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test('conditional branching: true branch runs, false branch is branch-skipped (not error)', async () => {
  const run = await runToCompletion(branchWfId);
  expect(run.status).toBe('success');
  expect(nodeState(run, 'passed').status).toBe('success');
  expect(nodeState(run, 'failed').status).toBe('skipped');
  // The skip reason is "branch not taken", not an upstream failure.
  const failedLogs = (nodeState(run, 'failed').logs ?? []).join(' ');
  expect(failedLogs.toLowerCase()).toContain('branch not taken');
});

test('agent node surfaces an openable session on its step', async () => {
  const run = await runToCompletion(sessionWfId);
  const agent = nodeState(run, 'agent');
  expect(agent.status).toBe('success');
  expect(Array.isArray(agent.sessions) && agent.sessions.length >= 1).toBeTruthy();
});

test('a finished run links a Proof Pack with evidence', async () => {
  const run = await runToCompletion(branchWfId);
  expect(run.proof_pack_id, 'run links a proof pack').toBeTruthy();
  const p = await ctx.get(`${base}${V1}/proof-packs/${run.proof_pack_id}`);
  expect(p.ok(), await p.text()).toBeTruthy();
  const detail = await p.json();
  expect((detail.artifacts ?? []).length).toBeGreaterThan(0);
});

test('loop stops when the until-expression is satisfied', async () => {
  const run = await runToCompletion(loopWfId);
  expect(run.status).toBe('success');
  const out = nodeState(run, 'loopn').output;
  expect(out.satisfied).toBe(true);
  expect(out.iterations).toBe(1);
});

test('workflow versioning: graph edits snapshot, restore appends a version', async () => {
  // v1 exists from create.
  let v = await ctx.get(`${base}${V1}/workflows/${loopWfId}/versions`);
  expect(v.ok()).toBeTruthy();
  let versions = await v.json();
  expect(versions.length).toBe(1);
  expect(versions[0].version).toBe(1);

  // A graph-changing PATCH bumps to v2.
  const patch = await ctx.patch(`${base}${V1}/workflows/${loopWfId}`, {
    data: { graph: { nodes: [node('trigger', 'manual_trigger')], edges: [] } },
  });
  expect(patch.ok(), await patch.text()).toBeTruthy();
  expect((await patch.json()).version).toBe(2);

  v = await ctx.get(`${base}${V1}/workflows/${loopWfId}/versions`);
  versions = await v.json();
  expect(versions.length).toBe(2);
  expect(versions[0].version).toBe(2);

  // Restore v1 → a NEW version (3) whose graph equals v1's.
  const restore = await ctx.post(`${base}${V1}/workflows/${loopWfId}/versions/1/restore`, {
    data: { note: 'back to original' },
  });
  expect(restore.ok(), await restore.text()).toBeTruthy();
  expect((await restore.json()).version).toBe(3);
  const wf = await (await ctx.get(`${base}${V1}/workflows/${loopWfId}`)).json();
  expect(wf.graph.nodes.find((n: any) => n.id === 'loopn'), 'v1 loop node restored').toBeTruthy();
});

test('convert a scheduled task into a workflow', async () => {
  const t = await ctx.post(`${base}${V1}/workspaces/${ws}/scheduled-tasks`, {
    data: {
      name: 'E2E Convertible',
      prompt: 'Summarize the repo.',
      schedule: { cadence: 'interval', every_min: 60 },
      destination: { type: 'none' },
      enabled: true,
    },
  });
  expect(t.ok(), await t.text()).toBeTruthy();
  const taskId = (await t.json()).id;

  const c = await ctx.post(`${base}${V1}/scheduled-tasks/${taskId}/convert-to-workflow`, {
    data: { disable_task: true },
  });
  expect(c.ok(), await c.text()).toBeTruthy();
  const { workflow_id } = await c.json();
  expect(workflow_id).toBeTruthy();

  const wf = await (await ctx.get(`${base}${V1}/workflows/${workflow_id}`)).json();
  const kinds = wf.graph.nodes.map((n: any) => n.kind);
  expect(kinds).toContain('manual_trigger');
  expect(kinds).toContain('agent_prompt');
  // A schedule trigger mirroring the cadence was created.
  const trigs = await (await ctx.get(`${base}${V1}/workflows/${workflow_id}/triggers`)).json();
  expect(trigs.some((tr: any) => tr.kind === 'schedule')).toBeTruthy();
});

test('orchestrator example templates are offered', async () => {
  const r = await ctx.get(`${base}${V1}/workflows/templates`);
  expect(r.ok()).toBeTruthy();
  const ids = (await r.json()).map((t: any) => t.id);
  expect(ids).toContain('write-tests');
  expect(ids).toContain('implement-feature');
  expect(ids).toContain('po-lifecycle');
});

// --- Desktop UI smoke -------------------------------------------------------

test.describe('workflows page (desktop)', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
    await page.addInitScript((wsId) => {
      localStorage.setItem('otto_workspace', wsId as string);
      localStorage.setItem('otto_rail_expanded', '0');
    }, ws);
  });

  test('renders the workflows page and lists a seeded workflow', async ({ page }) => {
    await page.goto('/#/workflows');
    await expect(page.getByText('E2E Branching').first()).toBeVisible({ timeout: 30_000 });
  });
});
