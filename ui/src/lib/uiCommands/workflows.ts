// Agent UI control — Workflows handlers (`otto.ui_wf_*`). The Workflows page
// keeps the open workflow, its canvas and the run inspector in component
// state, so it binds a page port (pagePort.ts) and the handlers open
// workflows / runs and start runs THROUGH the editor — the user watches the
// canvas and the run's steps light up next to the agent's session.
//
// Running a workflow is OUTWARD: its steps can open PRs, post to Slack/Jira,
// push branches. It always asks with the where/what/who confirm (never
// remembered). Cancelling a run is `local_write` (rememberable confirm).

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { api } from '../api/client';
import { ws } from '../stores/workspace.svelte';
import { confirmOutward } from '../confirmOutward';
import { toasts } from '../toast.svelte';
import type { RunWorkflowReq, Workflow, WorkflowRun } from '../api/types';
import { agentLabel, asUiError, createPagePort, dismissOnAbort, highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';

export const workflowsPagePort = createPagePort<{
  list(): Workflow[];
  loading(): boolean;
  currentId(): string | null;
  /** False when the user kept unsaved edits (declined "Discard"). */
  open(id: string): Promise<boolean>;
  openRun(workflowId: string, runId: string): Promise<void>;
  /** Null when it didn't start (validation/POST failure shown on the page). */
  start(body: RunWorkflowReq): Promise<WorkflowRun | null>;
  currentRun(): WorkflowRun | null;
}>('the Workflows page');

async function page(ctx: UiCommandCtx) {
  if (router.parts[0] !== 'workflows') router.go('workflows');
  const p = await workflowsPagePort.get(ctx.signal);
  await waitFor(() => !p.loading(), ctx.signal, 15_000, 'the workflow list');
  return p;
}

async function workflowFor(key: string, ctx: UiCommandCtx) {
  const p = await page(ctx);
  const wf = resolveByIdOrName(p.list(), key, (w) => w.id, (w) => w.name, 'workflow');
  return { p, wf };
}

async function openWf(p: Awaited<ReturnType<typeof page>>, wf: Workflow, ctx: UiCommandCtx): Promise<void> {
  if (p.currentId() === wf.id) return;
  ctx.progress(`Opening “${wf.name}”`, true); // may ask the user to discard edits
  if (!(await p.open(wf.id))) {
    throw new UiCommandError('cancelled_by_user', 'The user kept their unsaved edits to the open workflow.');
  }
  void highlightWhenReady(ctx, `[data-testid="wf-row-${CSS.escape(wf.id)}"]`);
}

function runSummary(r: WorkflowRun) {
  return {
    id: r.id,
    workflow_id: r.workflow_id,
    status: r.status,
    started_at: r.started_at,
    finished_at: r.finished_at ?? null,
    error: r.error ?? null,
    nodes: (r.nodes ?? []).map((n) => ({
      node_id: n.node_id,
      status: n.status,
      error: n.error ?? null,
      last_logs: (n.logs ?? []).slice(-5),
    })),
  };
}

registerUiCommands('workflows', {
  async wf_list(_args, ctx) {
    const p = await page(ctx);
    return {
      workflows: p.list().map((w) => ({ id: w.id, name: w.name, description: w.description, nodes: w.graph?.nodes?.length ?? 0 })),
      open: p.currentId(),
      running: ws.activeWorkflowRuns.map((r) => ({
        run_id: r.run_id,
        workflow_id: r.workflow_id,
        workflow: r.workflow_name,
        status: r.status,
        steps: `${r.nodes_done}/${r.nodes_total}`,
      })),
    };
  },

  async wf_open(args: { workflow: string }, ctx) {
    const { p, wf } = await workflowFor(args.workflow, ctx);
    await openWf(p, wf, ctx);
    return {
      workflow: { id: wf.id, name: wf.name, description: wf.description, instructions: wf.instructions },
      nodes: (wf.graph?.nodes ?? []).map((n) => ({ id: n.id, kind: n.kind, name: n.name })),
      edges: (wf.graph?.edges ?? []).length,
    };
  },

  async wf_list_runs(args: { workflow: string }, ctx) {
    const { p, wf } = await workflowFor(args.workflow, ctx);
    await openWf(p, wf, ctx);
    try {
      const runs = await api.get<Pick<WorkflowRun, 'id' | 'status' | 'started_at'>[]>(`/workflows/${wf.id}/runs?summary=true`);
      return { workflow: wf.name, runs: runs.slice(0, 50).map((r) => ({ id: r.id, status: r.status, started_at: r.started_at })) };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async wf_open_run(args: { workflow: string; run_id: string }, ctx) {
    const { p, wf } = await workflowFor(args.workflow, ctx);
    await openWf(p, wf, ctx);
    await p.openRun(wf.id, args.run_id);
    const shown = await waitFor(() => (p.currentRun()?.id === args.run_id ? p.currentRun() : null), ctx.signal, 10_000, 'the run');
    void highlightWhenReady(ctx, '[data-testid="run-item"].active');
    return { workflow: wf.name, run: runSummary(shown) };
  },

  async wf_run(args: { workflow: string; input?: Record<string, unknown>; prompt?: string }, ctx) {
    const { p, wf } = await workflowFor(args.workflow, ctx);
    await openWf(p, wf, ctx);
    const input = args.prompt?.trim() ? { ...(args.input ?? {}), prompt: args.prompt.trim() } : args.input;
    const who = agentLabel(ctx.agent);
    ctx.progress(`Waiting for you to confirm running “${wf.name}”`, true);
    const ok = await dismissOnAbort(ctx.signal, confirmOutward({
      verb: 'Run workflow',
      title: `${who} wants to run “${wf.name}”`,
      where: `Workflow “${wf.name}” (${wf.graph?.nodes?.length ?? 0} steps)`,
      what: input ? JSON.stringify(input, null, 2) : 'No run input.',
      who: 'Whatever its steps publish to (PRs, Slack, Jira…) sees the result.',
    }));
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the run.');
    const r = await p.start(input !== undefined ? { input } : {});
    if (!r) throw new UiCommandError('failed', `“${wf.name}” didn't start — the page shows why (validation issues or the error).`);
    toasts.info(`Started “${wf.name}”`, who);
    return { workflow: wf.name, run: runSummary(r) };
  },

  async wf_cancel_run(args: { run_id: string }, ctx) {
    const ok = await ctx.confirmWrite({
      what: `Cancel workflow run ${args.run_id} (it finishes the current step, then halts)`,
      where: 'Workflows',
      connId: 'workflows',
      verb: 'Cancel run',
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined cancelling the run.');
    try {
      await api.post(`/workflow-runs/${encodeURIComponent(args.run_id)}/cancel`, {});
    } catch (e) {
      throw asUiError(e);
    }
    toasts.info('Cancelling run…', agentLabel(ctx.agent));
    return { run_id: args.run_id, cancelling: true };
  },
});

// `otto.ui_state` view: the open workflow and the run in the inspector.
registerUiState('workflows', () => {
  const p = workflowsPagePort.peek();
  const id = p?.currentId() ?? null;
  const wf = id ? p?.list().find((w) => w.id === id) : null;
  const run = p?.currentRun() ?? null;
  return {
    workflow: wf ? { id: wf.id, name: wf.name } : id,
    run: run ? { id: run.id, status: run.status } : null,
    running: ws.activeWorkflowRuns.length,
  };
});
