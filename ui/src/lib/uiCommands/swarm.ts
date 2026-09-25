// Agent UI control — Agent Swarm handlers (`otto.ui_swarm_*`). The swarm list,
// open swarm, tasks and runs live in the swarm store; the view switcher (org
// tree / graph / kanban / runs / board) is SwarmPage state, bound via a page
// port. Running a task spawns an agent that works in a repo → `local_write`
// with the attributed, rememberable confirm; stopping a run too.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { swarm } from '../stores/swarm.svelte';
import { ws } from '../stores/workspace.svelte';
import { toasts } from '../toast.svelte';
import type { RunFilters, SwarmDetail, SwarmTask } from '../../modules/swarm/types';
import { agentLabel, asUiError, capList, createPagePort, highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';

type SwarmView = 'tree' | 'graph' | 'kanban' | 'runs' | 'board';
const VIEWS: SwarmView[] = ['tree', 'graph', 'kanban', 'runs', 'board'];

export const swarmPagePort = createPagePort<{
  openSwarm(id: string): Promise<void>;
  setView(v: SwarmView): void;
}>('the Swarm page');

async function showPage(ctx: UiCommandCtx) {
  const wsId = ws.currentId;
  if (!wsId) throw new UiCommandError('failed', 'No workspace is selected in Otto.');
  if (router.parts[0] !== 'swarm') router.go('swarm');
  const page = await swarmPagePort.get(ctx.signal);
  await swarm.loadSwarms(wsId);
  if (swarm.swarmsError) throw new UiCommandError('failed', swarm.swarmsError);
  return page;
}

async function openSwarm(key: string | undefined, ctx: UiCommandCtx): Promise<SwarmDetail> {
  const page = await showPage(ctx);
  if (!key) {
    if (swarm.detail) return swarm.detail;
    throw new UiCommandError('invalid_args', 'Pass `swarm` (id or name — otto.ui_swarm_list).');
  }
  const s = resolveByIdOrName(swarm.swarms, key, (x) => x.id, (x) => x.name, 'swarm');
  if (swarm.detail?.id !== s.id) await page.openSwarm(s.id);
  const d = await waitFor(() => (!swarm.loading && swarm.detail?.id === s.id ? swarm.detail : null), ctx.signal, 20_000, 'the swarm');
  if (swarm.detailError) throw new UiCommandError('failed', swarm.detailError);
  return d;
}

function taskSummary(t: SwarmTask) {
  return {
    id: t.id,
    project_id: t.project_id,
    title: t.title,
    status: t.status,
    priority: t.priority,
    assignee_agent_id: t.assignee_agent_id ?? null,
    labels: t.labels,
    depends_on: t.depends_on,
  };
}

function findTask(taskId: string): SwarmTask {
  for (const list of Object.values(swarm.tasksByProject)) {
    const t = list.find((x) => x.id === taskId);
    if (t) return t;
  }
  throw new UiCommandError('not_found', `No task ${taskId} in the open swarm (otto.ui_swarm_list_tasks).`);
}

registerUiCommands('swarm', {
  async swarm_list(_args, ctx) {
    await showPage(ctx);
    return {
      swarms: swarm.swarms.map((s) => ({ id: s.id, name: s.name, status: s.status, description: s.description })),
      open: swarm.detail?.id ?? null,
    };
  },

  async swarm_open(args: { swarm: string; view?: SwarmView; project?: string }, ctx) {
    const d = await openSwarm(args.swarm, ctx);
    if (args.project) {
      const p = resolveByIdOrName(d.projects, args.project, (x) => x.id, (x) => x.name, 'project');
      swarm.selectedProjectId = p.id;
      if (!swarm.tasksByProject[p.id]) await swarm.loadTasks(p.id);
    }
    if (args.view) {
      if (!VIEWS.includes(args.view)) throw new UiCommandError('invalid_args', `Unknown view “${args.view}” (one of ${VIEWS.join(', ')}).`);
      (await swarmPagePort.get(ctx.signal)).setView(args.view);
    }
    void highlightWhenReady(ctx, '.swarm-page');
    return {
      swarm: { id: d.id, name: d.name, status: d.status },
      agents: d.agents.map((a) => ({ id: a.id, name: a.name, title: a.title, status: a.status })),
      projects: d.projects.map((p) => ({ id: p.id, name: p.name, repo_path: p.repo_path ?? null })),
      selected_project: swarm.selectedProjectId,
      counts: d.counts,
    };
  },

  async swarm_list_tasks(args: { swarm?: string; project?: string; status?: string }, ctx) {
    const d = await openSwarm(args.swarm, ctx);
    let tasks: SwarmTask[];
    if (args.project) {
      const p = resolveByIdOrName(d.projects, args.project, (x) => x.id, (x) => x.name, 'project');
      swarm.selectedProjectId = p.id;
      await swarm.loadTasks(p.id);
      tasks = swarm.tasksByProject[p.id] ?? [];
    } else {
      tasks = Object.values(swarm.tasksByProject).flat();
    }
    if (args.status) tasks = tasks.filter((t) => t.status === args.status);
    (await swarmPagePort.get(ctx.signal)).setView('kanban');
    const { items, total, truncated } = capList(tasks);
    return { swarm: d.name, total, truncated, tasks: items.map(taskSummary) };
  },

  async swarm_list_runs(args: { swarm?: string; status?: string }, ctx) {
    const d = await openSwarm(args.swarm, ctx);
    await swarm.loadRuns({ swarm_id: d.id, status: args.status as RunFilters['status'] });
    (await swarmPagePort.get(ctx.signal)).setView('runs');
    const { items, total, truncated } = capList(swarm.runs);
    return {
      swarm: d.name,
      total,
      truncated,
      runs: items.map((r) => ({
        id: r.id,
        task_id: r.task_id ?? null,
        agent_id: r.agent_id,
        session_id: r.session_id ?? null,
        kind: r.kind,
        status: r.status,
        attempt: r.attempt,
        summary: r.summary ?? null,
        error: r.error ?? null,
      })),
    };
  },

  async swarm_run_task(args: { swarm?: string; task_id: string }, ctx) {
    const d = await openSwarm(args.swarm, ctx);
    const t = findTask(args.task_id);
    (await swarmPagePort.get(ctx.signal)).setView('kanban');
    const ok = await ctx.confirmWrite({ what: `Run the swarm task “${t.title}” (spawns an agent)`, where: `Swarm “${d.name}”`, connId: `swarm:${d.id}`, verb: 'Run' });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined running the task.');
    try {
      await swarm.runTask(t);
    } catch (e) {
      throw asUiError(e);
    }
    toasts.success(`Started “${t.title}”`, agentLabel(ctx.agent));
    const run = swarm.runs.find((r) => r.task_id === t.id) ?? null;
    return { task: taskSummary(findTask(t.id)), run: run ? { id: run.id, status: run.status, session_id: run.session_id ?? null } : null };
  },

  async swarm_stop_run(args: { swarm?: string; run_id: string }, ctx) {
    const d = await openSwarm(args.swarm, ctx);
    (await swarmPagePort.get(ctx.signal)).setView('runs');
    const ok = await ctx.confirmWrite({ what: `Stop swarm run ${args.run_id}`, where: `Swarm “${d.name}”`, connId: `swarm:${d.id}`, verb: 'Stop' });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined stopping the run.');
    try {
      await swarm.stopRun(args.run_id);
    } catch (e) {
      throw asUiError(e);
    }
    toasts.info('Stopping run…', agentLabel(ctx.agent));
    return { run_id: args.run_id, stopped: true };
  },
});

// `otto.ui_state` view: the open swarm and project.
registerUiState('swarm', () => ({
  swarm: swarm.detail ? { id: swarm.detail.id, name: swarm.detail.name, status: swarm.detail.status } : null,
  project: swarm.selectedProjectId,
  swarms: swarm.swarms.length,
}));
