// Agent UI control — Scheduled Tasks handlers (`otto.ui_sched_*`). The list
// and runs come from the scheduledTasks store the page renders; the page binds
// a port so the agent can expand a task's run history in place.
//
// `sched_run_now` / `sched_set_enabled` are `local_write`: an attributed
// confirm, rememberable for the workspace. A run whose task DELIVERS somewhere
// (Slack / Telegram / email / webhook) is outward, so running it always gets
// the where/what/who confirm and is never remembered.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { scheduledTasks } from '../stores/scheduledTasks.svelte';
import { ws } from '../stores/workspace.svelte';
import { confirmOutward } from '../confirmOutward';
import { toasts } from '../toast.svelte';
import type { ScheduledTask } from '../api/types';
import { agentLabel, asUiError, createPagePort, dismissOnAbort, highlightWhenReady, resolveByIdOrName } from './pagePort';

export const scheduledTasksPort = createPagePort<{ expand(id: string | null): void }>('the Scheduled Tasks page');

async function showList(ctx: UiCommandCtx): Promise<string> {
  const wsId = ws.currentId;
  if (!wsId) throw new UiCommandError('failed', 'No workspace is selected in Otto.');
  if (router.parts[0] !== 'scheduled-tasks') router.go('scheduled-tasks');
  await scheduledTasks.loadList(wsId);
  if (ctx.signal.aborted) throw new UiCommandError('cancelled_by_user', 'Cancelled');
  if (scheduledTasks.listError) throw new UiCommandError('failed', scheduledTasks.listError);
  return wsId;
}

async function taskFor(key: string, ctx: UiCommandCtx): Promise<ScheduledTask> {
  await showList(ctx);
  return resolveByIdOrName(scheduledTasks.list, key, (t) => t.id, (t) => t.name, 'scheduled task');
}

/** Where a task's report goes, or null when it stays in Otto. */
function delivery(t: ScheduledTask): string | null {
  const d = t.destination ?? {};
  switch (d.type as string) {
    case 'slack':
      return `Slack${d.channel ? ` (${String(d.channel)})` : ''}`;
    case 'telegram':
      return 'Telegram';
    case 'email':
      return d.to ? `email to ${String(d.to)}` : 'email';
    case 'webhook':
      return 'a webhook';
    default:
      return null;
  }
}

function summary(t: ScheduledTask) {
  return {
    id: t.id,
    name: t.name,
    kind: t.kind,
    provider: t.provider,
    enabled: t.enabled,
    schedule: t.schedule,
    timezone: t.timezone,
    delivers_to: delivery(t),
    last_status: t.last_status ?? null,
    next_run_at: t.next_run_at ?? null,
  };
}

registerUiCommands('scheduled-tasks', {
  async sched_list(_args, ctx) {
    await showList(ctx);
    void highlightWhenReady(ctx, 'ul.tasks');
    return { tasks: scheduledTasks.list.map(summary) };
  },

  async sched_show_runs(args: { task: string }, ctx) {
    const t = await taskFor(args.task, ctx);
    const page = await scheduledTasksPort.get(ctx.signal);
    page.expand(t.id);
    await scheduledTasks.loadRuns(t.id);
    const err = scheduledTasks.runsError[t.id];
    if (err) throw new UiCommandError('failed', err);
    void highlightWhenReady(ctx, `li[data-task-id="${CSS.escape(t.id)}"]`);
    return {
      task: summary(t),
      runs: (scheduledTasks.runsByTask[t.id] ?? []).slice(0, 50).map((r) => ({
        id: r.id,
        status: r.status,
        trigger: r.trigger,
        started_at: r.started_at,
        finished_at: r.finished_at ?? null,
        summary: r.summary,
        delivered: r.delivered,
        error: r.error ?? r.delivery_error ?? null,
        session_id: r.session_id ?? null,
      })),
    };
  },

  async sched_run_now(args: { task: string }, ctx) {
    const t = await taskFor(args.task, ctx);
    void highlightWhenReady(ctx, `li[data-task-id="${CSS.escape(t.id)}"]`);
    const who = agentLabel(ctx.agent);
    const dest = delivery(t);
    if (dest) ctx.progress(`Waiting for you to confirm running “${t.name}”`, true);
    const ok = dest
      ? await dismissOnAbort(ctx.signal, confirmOutward({
          verb: 'Run now',
          title: `${who} wants to run “${t.name}” now`,
          where: `Scheduled task “${t.name}” → ${dest}`,
          what: t.prompt || undefined,
          who: `Whoever reads ${dest} receives the report.`,
        }))
      : await ctx.confirmWrite({ what: `Run the scheduled task “${t.name}” once now`, where: 'Scheduled Tasks', connId: `sched:${t.workspace_id}`, verb: 'Run' });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the run.');
    try {
      await scheduledTasks.runNow(t.id);
    } catch (e) {
      throw asUiError(e);
    }
    scheduledTasksPort.peek()?.expand(t.id);
    toasts.success(`Started “${t.name}”`, who);
    const latest = (scheduledTasks.runsByTask[t.id] ?? [])[0] ?? null;
    return { task: t.name, started: true, run: latest ? { id: latest.id, status: latest.status, session_id: latest.session_id ?? null } : null };
  },

  async sched_set_enabled(args: { task: string; enabled: boolean }, ctx) {
    const t = await taskFor(args.task, ctx);
    if (t.enabled === args.enabled) return { task: t.name, enabled: t.enabled, changed: false };
    void highlightWhenReady(ctx, `li[data-task-id="${CSS.escape(t.id)}"]`);
    const ok = await ctx.confirmWrite({
      what: `${args.enabled ? 'Resume' : 'Pause'} the scheduled task “${t.name}”`,
      where: 'Scheduled Tasks',
      connId: `sched:${t.workspace_id}`,
      verb: args.enabled ? 'Resume' : 'Pause',
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the change.');
    try {
      await scheduledTasks.setEnabled(t.id, args.enabled);
    } catch (e) {
      throw asUiError(e);
    }
    toasts.success(`${args.enabled ? 'Resumed' : 'Paused'} “${t.name}”`, agentLabel(ctx.agent));
    return { task: t.name, enabled: args.enabled, changed: true };
  },
});

// `otto.ui_state` view: how many tasks, and which are paused.
registerUiState('scheduled-tasks', () => ({
  tasks: scheduledTasks.list.length,
  paused: scheduledTasks.list.filter((t) => !t.enabled).map((t) => t.name),
}));
