import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Generic-workflow context files E2E (design 2026-07-02..03): a workflow's
// standing `instructions` and a run's ask both land verbatim as files
// alongside the auto-generated `run-brief.md`, `final-output.md` is the last
// content-bearing step (never a trailing utility step like `log`),
// `prepare_context` is a clean no-op when there's no ticket anywhere in the
// run (purely native — no agent spawn), and `instructions` round-trip
// through the editor API (PATCH + version history). Modeled on
// `desktop-workflow-context.spec.ts` — same daemon/data-dir resolution via
// `e2e/.auth-{SLOT}/daemon.json`.

const V1 = '/api/v1';
const SLOT = process.env.OTTO_E2E_SLOT ?? '0';

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
    data: { name, description: 'e2e', graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function createWorkflowWithInstructions(
  name: string,
  instructions: string,
  nodes: Node[],
  edges: Edge[],
): Promise<string> {
  const r = await ctx.post(`${base}${V1}/workspaces/${ws}/workflows`, {
    data: { name, description: 'e2e', instructions, graph: { nodes, edges } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

/** Run a workflow with the given input and poll its run row to a terminal status. */
async function runToCompletion(wfId: string, input?: unknown, deadlineMs = 90_000): Promise<any> {
  const r = await ctx.post(`${base}${V1}/workflows/${wfId}/run`, {
    data: input === undefined ? {} : { input },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const runId = (await r.json()).id as string;
  const deadline = Date.now() + deadlineMs;
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

/** The daemon's derived per-run context dir, as surfaced on the run itself. */
function contextDirOf(run: any): string {
  return run.context_dir as string;
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

test('generic context files + final output', async () => {
  const wfId = await createWorkflowWithInstructions(
    'generic-files',
    '# RULES\nfollow me',
    [
      node('trigger', 'manual_trigger'),
      node('work', 'agent_prompt', { prompt: 'do the thing' }),
      node('notify', 'log'),
    ],
    [edge('trigger', 'work'), edge('work', 'notify')],
  );
  const run = await runToCompletion(wfId, { prompt: 'the ask from slack' });
  expect(run.status).toBe('success');
  const dir = contextDirOf(run);
  expect(existsSync(dir)).toBe(true);
  expect(readFileSync(join(dir, 'instructions.md'), 'utf8')).toContain('RULES');
  expect(readFileSync(join(dir, 'prompt.md'), 'utf8')).toBe('the ask from slack');
  expect(existsSync(join(dir, 'run-brief.md'))).toBe(true);
  // final output = the agent step's handoff (log is bookkeeping-only), not the
  // trailing `log` utility step.
  expect(readFileSync(join(dir, 'final-output.md'), 'utf8')).toContain('work');
});

test('prepare_context no-op without a ticket', async () => {
  const wfId = await createWorkflow(
    'prep-noop',
    [
      node('trigger', 'manual_trigger'),
      node('prep', 'prepare_context'), // no params.prompt → purely native, no agent spawn
    ],
    [edge('trigger', 'prep')],
  );
  const run = await runToCompletion(wfId, { prompt: 'no ticket here' });
  expect(run.status).toBe('success');
  const prep = (run.nodes ?? []).find((n: any) => n.node_id === 'prep');
  expect(prep).toBeTruthy();
  expect(prep.output.jira.found).toBe(false);
});

test('instructions persist through update', async () => {
  const wfId = await createWorkflow('instr-rt', [node('trigger', 'manual_trigger')], []);
  const p = await ctx.patch(`${base}${V1}/workflows/${wfId}`, { data: { instructions: 'be precise' } });
  expect(p.ok(), await p.text()).toBeTruthy();
  const wf = await (await ctx.get(`${base}${V1}/workflows/${wfId}`)).json();
  expect(wf.instructions).toBe('be precise');
  // Versions are returned newest-first (see routes/workflows.rs list_versions).
  const versions = await (await ctx.get(`${base}${V1}/workflows/${wfId}/versions`)).json();
  expect(versions.length).toBeGreaterThan(0);
  expect(versions[0].instructions).toBe('be precise');
});
