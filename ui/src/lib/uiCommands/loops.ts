// Agent UI control — older goal loops (`otto.ui_loops_*`, read-only since
// goals moved into Workflows; an agent pauses or resumes a goal run with
// `otto.control_workflow_run`). The list and the open detail come from the
// loops store; LoopsPage binds a port so the agent opens a loop in the
// user's pane.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { loops } from '../stores/loops.svelte';
import { mapLimit } from '../poll';
import { ws } from '../stores/workspace.svelte';
import type { GoalLoop } from '../api/types';
import { createPagePort, highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';

export const loopsPagePort = createPagePort<{
  open(id: string): void;
  selectedId(): string | null;
}>('the Goal Loops page');

async function showList(ctx: UiCommandCtx): Promise<void> {
  const wsId = ws.currentId;
  if (!wsId) throw new UiCommandError('failed', 'No workspace is selected in Otto.');
  if (router.parts[0] !== 'loops') router.go('loops');
  await loops.loadList(wsId);
  if (ctx.signal.aborted) throw new UiCommandError('cancelled_by_user', 'Canceled');
  if (loops.listError) throw new UiCommandError('failed', loops.listError);
}

function summary(l: GoalLoop) {
  return {
    id: l.id,
    name: l.name,
    status: l.status,
    phase: l.phase,
    progress_pct: l.progress_pct,
    current_iteration: l.current_iteration,
    repo_path: l.repo_path,
    branch: l.branch ?? null,
    cost_usd: l.cost_usd,
    error: l.error ?? null,
  };
}

async function openLoop(key: string, ctx: UiCommandCtx): Promise<GoalLoop> {
  await showList(ctx);
  const l = resolveByIdOrName(loops.list, key, (x) => x.id, (x) => x.name, 'goal loop');
  const page = await loopsPagePort.get(ctx.signal);
  if (page.selectedId() !== l.id) page.open(l.id);
  await waitFor(() => !loops.loadingDetail && loops.detail?.loop.id === l.id, ctx.signal, 15_000, 'the loop detail');
  void highlightWhenReady(ctx, '.detail-page, .detail');
  return l;
}

registerUiCommands('loops', {
  async loops_list(_args, ctx) {
    await showList(ctx);
    return { loops: loops.list.map(summary) };
  },

  async loops_open(args: { loop: string }, ctx) {
    await openLoop(args.loop, ctx);
    const d = loops.detail!;
    // The store holds the SUMMARY detail (older iterations' plans blank):
    // read the last 10 in full, 3 at a time.
    const last = d.iterations.slice(-10);
    const newest = last[last.length - 1]?.idx;
    const full = await mapLimit(last, 3, (it) =>
      it.idx === newest ? Promise.resolve(it) : loops.loadIteration(d.loop.id, it, ctx.signal).catch(() => it),
    );
    return {
      loop: summary(d.loop),
      definition: d.loop.definition,
      iterations: full.map((it) => ({
        idx: it.idx,
        status: it.status,
        plan: it.plan.slice(0, 2000),
        evaluation: it.evaluation ?? null,
      })),
    };
  },
});

// `otto.ui_state` view: the open loop (if any).
registerUiState('loops', () => {
  const d = loops.detail;
  return {
    open: d ? { id: d.loop.id, name: d.loop.name, status: d.loop.status, phase: d.loop.phase } : null,
    loops: loops.list.length,
  };
});
