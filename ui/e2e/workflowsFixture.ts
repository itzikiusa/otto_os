// Seeding for the Workflows specs (test-plan.md §7 "Seeding"): workflows
// built from small graphs whose agent steps carry `OTTO_FAKE:` script lines
// (the fake CLIs on the isolated daemon's PATH act them out: ask, approve,
// fail …, crates/otto-agent-protocol/src/fake_script.rs), runs started through
// the API, and polling until a run reaches the state a spec needs. Never a
// real agent CLI; never the owner's daemon.
import { expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import type { WorkflowGraph, WorkflowNode, WorkflowRun } from '../src/lib/api/types';

const V1 = '/api/v1';

export interface WfApi {
  ctx: APIRequestContext;
  base: string;
  ws: string;
  get<T = unknown>(path: string): Promise<T>;
  post<T = unknown>(path: string, data?: unknown): Promise<T>;
}

export async function wfApi(name = 'E2E Workflows'): Promise<WfApi> {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base, name);
  const call = async <T>(r: Promise<import('@playwright/test').APIResponse>): Promise<T> => {
    const res = await r;
    expect(res.ok(), await res.text()).toBeTruthy();
    return (res.status() === 204 ? null : await res.json()) as T;
  };
  return {
    ctx, base, ws,
    get: (path) => call(ctx.get(`${base}${V1}${path}`)),
    post: (path, data) => call(ctx.post(`${base}${V1}${path}`, { data: data ?? {} })),
  };
}

export const node = (id: string, kind: string, name = id, params: Record<string, unknown> | null = null): WorkflowNode => ({ id, kind, name, x: 0, y: 0, params });
/** An agent step acting out `script` (one fake verb a line), e.g. ['ask Which branch?']. */
export const agentStep = (id: string, name: string, script: string[] = [], params: Record<string, unknown> = {}): WorkflowNode =>
  node(id, 'agent_prompt', name, { provider: 'claude', prompt: [`Do the "${name}" step.`, ...script.map((s) => `OTTO_FAKE: ${s}`)].join('\n'), ...params });
/** `a → b → c …` from a Start step. */
export function chain(...steps: WorkflowNode[]): WorkflowGraph {
  const nodes = [node('start', 'manual_trigger', 'Start'), ...steps];
  return { nodes, edges: nodes.slice(1).map((n, i) => ({ id: `e${i}`, source: nodes[i].id, target: n.id })) };
}
/** Start → `fan` side by side → `join`. */
export function fork(fan: WorkflowNode[], join: WorkflowNode): WorkflowGraph {
  const start = node('start', 'manual_trigger', 'Start');
  return {
    nodes: [start, ...fan, join],
    edges: [...fan.map((n, i) => ({ id: `s${i}`, source: start.id, target: n.id })), ...fan.map((n, i) => ({ id: `j${i}`, source: n.id, target: join.id }))],
  };
}

export async function createWorkflow(api: WfApi, name: string, graph: WorkflowGraph): Promise<string> {
  return (await api.post<{ id: string }>(`/workspaces/${api.ws}/workflows`, { name, description: 'e2e', graph })).id;
}

/** Runs this worker started: `cancelStarted` stops them after each test. */
const started = new Set<string>();

export async function startRun(api: WfApi, workflowId: string, input?: Record<string, unknown>): Promise<string> {
  const id = (await api.post<{ id: string }>(`/workflows/${workflowId}/run`, input ? { input } : {})).id;
  started.add(id);
  return id;
}

/** `afterEach`: cancel every run the test started, passed or failed — a run
 *  left blocking (a sleep, an ask) holds one of the daemon's two run slots
 *  (OTTO_WF_MAX_PARALLEL_RUNS) and queues the next spec's runs. A settled run's
 *  cancel is a no-op. */
export async function cancelStarted(api: WfApi | undefined): Promise<void> {
  if (!api) return;
  for (const id of started) await api.ctx.post(`${api.base}/api/v1/workflow-runs/${id}/cancel`, { data: {} }).catch(() => undefined);
  started.clear();
}

/** Poll the run until `ready` holds (or fail after `timeoutMs`). */
export async function waitRun(api: WfApi, runId: string, ready: (run: WorkflowRun) => boolean, timeoutMs = 60_000): Promise<WorkflowRun> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const run = await api.get<WorkflowRun>(`/workflow-runs/${runId}`);
    if (ready(run)) return run;
    if (Date.now() > deadline) throw new Error(`run ${runId} never reached the expected state (now ${run.status} / ${run.overview?.state ?? '—'})`);
    await new Promise((r) => setTimeout(r, 300));
  }
}
export const settled = (run: WorkflowRun): boolean => run.status !== 'running' && run.status !== 'pending';
export const needsYou = (run: WorkflowRun): boolean => run.overview?.state === 'needs_you';

/** A review step with `n` named reviewers over claude and codex (the fan-out fixture). */
export function reviewStep(id: string, n: number): WorkflowNode {
  const lenses = Array.from({ length: n }, (_, i) => `lens-${i + 1}`);
  return node(id, 'review_run', 'Multi-agent review', { reviewers: lenses.map((lens, i) => ({ lens, provider: i % 2 ? 'codex' : 'claude' })), mode: 'fan_out', threshold: 0 });
}
