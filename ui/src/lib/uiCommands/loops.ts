// Agent UI control — Goal Loops handlers (`otto.ui_loops_*`). The list and
// the open detail come from the loops store; LoopsPage binds a port so the
// agent opens a loop's detail in the user's pane. Lifecycle changes (start /
// pause / resume / stop) spawn or halt agents that change a repo worktree →
// `local_write`: the attributed, rememberable confirm — except Stop, which
// can't be undone (a new run must be started) and always asks.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { loops } from '../stores/loops.svelte';
import { ws } from '../stores/workspace.svelte';
import { confirmer } from '../confirm.svelte';
import { toasts } from '../toast.svelte';
import type { GoalLoop } from '../api/types';
import { agentLabel, asUiError, createPagePort, dismissOnAbort, highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';

export const loopsPagePort = createPagePort<{
  open(id: string): void;
  selectedId(): string | null;
}>('the Goal Loops page');

async function showList(ctx: UiCommandCtx): Promise<void> {
  const wsId = ws.currentId;
  if (!wsId) throw new UiCommandError('failed', 'No workspace is selected in Otto.');
  if (router.parts[0] !== 'loops') router.go('loops');
  await loops.loadList(wsId);
  if (ctx.signal.aborted) throw new UiCommandError('cancelled_by_user', 'Cancelled');
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

const ACTIONS = ['start', 'pause', 'resume', 'stop'] as const;
type LoopAction = (typeof ACTIONS)[number];

registerUiCommands('loops', {
  async loops_list(_args, ctx) {
    await showList(ctx);
    return { loops: loops.list.map(summary) };
  },

  async loops_open(args: { loop: string }, ctx) {
    await openLoop(args.loop, ctx);
    const d = loops.detail!;
    return {
      loop: summary(d.loop),
      definition: d.loop.definition,
      iterations: d.iterations.slice(-10).map((it) => ({
        idx: it.idx,
        status: it.status,
        plan: it.plan.slice(0, 2000),
        evaluation: it.evaluation ?? null,
      })),
    };
  },

  async loops_control(args: { loop: string; action: LoopAction }, ctx) {
    if (!ACTIONS.includes(args.action)) {
      throw new UiCommandError('invalid_args', `Unknown action “${args.action}” (one of ${ACTIONS.join(', ')}).`);
    }
    const l = await openLoop(args.loop, ctx);
    const who = agentLabel(ctx.agent);
    const verb = args.action.charAt(0).toUpperCase() + args.action.slice(1);
    let ok: boolean;
    if (args.action === 'stop') {
      ctx.progress(`Waiting for you to confirm: Stop “${l.name}”`, true);
      ok = await dismissOnAbort(
        ctx.signal,
        confirmer.ask(`${who} wants to stop the goal loop “${l.name}”. It can't be resumed — a new run must be started.`, {
          title: 'Stop goal loop',
          confirmLabel: 'Stop loop',
          danger: true,
        }),
      );
    } else {
      ok = await ctx.confirmWrite({
        what: `${verb} the goal loop “${l.name}”`,
        where: `Goal Loops · ${l.repo_path}`,
        connId: `loops:${l.workspace_id}`,
        verb,
      });
    }
    if (!ok) throw new UiCommandError('cancelled_by_user', `The user declined: ${verb}.`);
    try {
      await loops[args.action](l.id);
    } catch (e) {
      throw asUiError(e);
    }
    toasts.success(`${verb} · ${l.name}`, who);
    const now = loops.list.find((x) => x.id === l.id) ?? l;
    return { loop: summary(now) };
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
