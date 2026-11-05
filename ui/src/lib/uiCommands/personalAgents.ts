// Agent UI control — Personal Agents handlers (`otto.ui_pa_*`). The page's
// route `personal-agents` belongs to the `scheduled-tasks` pane (sidePane
// `paneKey`), so the commands register under that module. The list comes from
// the personalAgents store the page renders; the page, the New-agent sheet and
// one agent's page bind ports (./personalAgentsPort.ts) so a command can open
// the sheet, fill it where the user sees it and run its own Save, re-read the
// Autonomy tab, or open a run's report.
//
// Writes are `local_write`: an attributed confirm stating exactly what changes
// (persona summarized, schedule, permission), rememberable per workspace. A
// Run now whose agent DELIVERS somewhere is outward (where / what / who,
// never remembered), and lending the user's Slack identity
// (`pa_set_slack_as_me`) is ALWAYS the outward confirm, never remembered.

import { registerUiCommands, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { personalAgents } from '../stores/personalAgents.svelte';
import { personalAgentsApi } from '../api/personalAgents';
import { api, authedText } from '../api/client';
import { ws } from '../stores/workspace.svelte';
import { confirmOutward, destWho, savedDestWhere } from '../confirmOutward';
import { toasts } from '../toast.svelte';
import type { PersonalAgent, PersonalAgentAutonomy, PersonalAgentSchedule, PersonalAgentSlackGrant } from '../api/types';
import templates from '../../../../docs/contracts/personal-agent-templates.json';
import { agentLabel, asUiError, dismissOnAbort, highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';
import { paAgentPort, paListPort, paSheetPort } from './personalAgentsPort';
import {
  agentChanges,
  checkCadence,
  checkPermission,
  createConfirmText,
  PaArgError,
  personaSummary,
  scheduleText,
  slackAsMeConfirm,
  type PaAgentPatch,
  type PaCreateFill,
  type PaScheduleArg,
} from './personalAgentsPlan';

/** The New-agent sheet's templates — the same ONE file it reads. */
type Template = { id: string; title: string; name: string; soul_md: string; directive: string; schedule: Record<string, unknown> };
const templateById = (id: string): Template | undefined => (templates.templates as Template[]).find((t) => t.id === id);

/** The user's IANA timezone (a new schedule's default, like the Schedules tab). */
function browserTz(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

const TABS = ['overview', 'activity', 'autonomy', 'schedules', 'runs', 'chat', 'memory', 'context'] as const;
type Tab = (typeof TABS)[number];

/** Plan-file argument errors → `invalid_args`. */
function args<T>(f: () => T): T {
  try {
    return f();
  } catch (e) {
    if (e instanceof PaArgError) throw new UiCommandError('invalid_args', e.message);
    throw e;
  }
}

function stopIfAborted(ctx: UiCommandCtx): void {
  if (ctx.signal.aborted) throw new UiCommandError('cancelled_by_user', 'Canceled');
}

async function loadList(ctx: UiCommandCtx): Promise<string> {
  const wsId = ws.currentId;
  if (!wsId) throw new UiCommandError('failed', 'No workspace is selected in Otto.');
  await personalAgents.loadAgents(wsId);
  stopIfAborted(ctx);
  if (personalAgents.agentsError) throw new UiCommandError('failed', personalAgents.agentsError);
  return wsId;
}

async function agentFor(key: string, ctx: UiCommandCtx): Promise<PersonalAgent> {
  await loadList(ctx);
  return resolveByIdOrName(personalAgents.agents, key, (a) => a.id, (a) => a.name, 'personal agent');
}

/** Show one agent on `tab` and wait for its page to bind. */
async function openAgent(a: PersonalAgent, tab: Tab, ctx: UiCommandCtx) {
  // `personal-agents/<id>` alone is the conversation home; every settings tab has its own path.
  const path = `personal-agents/${a.id}/${tab}`;
  if (router.parts.join('/') !== path) router.go(path);
  return waitFor(() => {
    const p = paAgentPort.peek();
    return p && p.agentId === a.id ? p : null;
  }, ctx.signal, 10_000, 'the personal agent page');
}

function scheduleArg(a: Partial<PaScheduleArg>, requireCadence: boolean): Partial<PaScheduleArg> {
  return args(() => ({
    ...(a.schedule !== undefined || requireCadence ? { schedule: checkCadence(a.schedule) } : {}),
    ...(a.timezone !== undefined ? { timezone: String(a.timezone) } : {}),
    ...(a.directive !== undefined ? { directive: String(a.directive) } : {}),
    ...(a.permission !== undefined ? { permission: checkPermission(a.permission) } : {}),
    ...(a.enabled !== undefined ? { enabled: !!a.enabled } : {}),
  }));
}

function summary(a: PersonalAgent) {
  const schedules = personalAgents.schedulesByAgent[a.id] ?? [];
  return {
    id: a.id,
    name: a.name,
    provider: a.provider,
    model: a.model || null,
    enabled: a.enabled,
    delivers_to: savedDestWhere(a.delivery) || null,
    created_by_agent: a.created_by_agent ?? null,
    schedules: schedules.length,
    next_run_at: a.enabled ? personalAgents.nextRunAt(a.id) : null,
  };
}

function scheduleOut(s: PersonalAgentSchedule) {
  return {
    id: s.id,
    schedule: s.schedule,
    timezone: s.timezone,
    directive: s.directive,
    permission: s.permission ?? 'directed',
    enabled: s.enabled,
    next_run_at: s.next_run_at ?? null,
  };
}

const ROW = (attr: string, id: string) => `[${attr}="${CSS.escape(id)}"]`;

registerUiCommands('scheduled-tasks', {
  async pa_list(_args, ctx) {
    await loadList(ctx);
    if (router.parts[0] !== 'personal-agents' || router.parts[1]) router.go('personal-agents');
    void highlightWhenReady(ctx, '[data-testid="pa-cards"]');
    return { agents: personalAgents.agents.map(summary) };
  },

  async pa_open(a: { agent: string; tab?: string }, ctx) {
    const tab = (a.tab ?? 'overview') as Tab;
    if (!TABS.includes(tab)) throw new UiCommandError('invalid_args', `tab must be one of ${TABS.join(' | ')}.`);
    const agent = await agentFor(a.agent, ctx);
    await openAgent(agent, tab, ctx);
    void highlightWhenReady(ctx, `[role="tab"][data-tab="${tab}"]`);
    if (tab === 'schedules' || tab === 'overview') await personalAgents.loadSchedules(agent.id);
    return {
      agent: summary(agent),
      tab,
      persona: personaSummary(agent.soul_md),
      schedules: (personalAgents.schedulesByAgent[agent.id] ?? []).map(scheduleOut),
    };
  },

  async pa_create(a: PaCreateFill, ctx) {
    const wsId = await loadList(ctx);
    const t = a.template ? templateById(a.template) : undefined;
    if (a.template && !t) throw new UiCommandError('not_found', `No agent template “${a.template}”.`);
    if (!t && !a.name?.trim()) throw new UiCommandError('invalid_args', 'Pass a name (or a template).');
    const fill: PaCreateFill = { ...a, ...(a.schedule ? { schedule: scheduleArg(a.schedule, true) as PaScheduleArg } : {}) };
    if (fill.schedule && !fill.schedule.timezone) fill.schedule.timezone = browserTz();
    (await waitFor(() => paListPort.peek(), ctx.signal, 10_000, 'the Personal Agents page')).openCreate();
    const sheet = await waitFor(() => paSheetPort.peek(), ctx.signal, 15_000, 'the New personal agent form');
    sheet.fill(fill);
    void highlightWhenReady(ctx, '[data-pa-sheet]');
    const ok = await ctx.confirmWrite({
      what: createConfirmText(fill, t, browserTz()),
      where: 'Personal Agents',
      connId: `pa:${wsId}`,
      verb: 'Create',
    });
    if (!ok) {
      sheet.close();
      throw new UiCommandError('cancelled_by_user', 'The user declined creating the agent.');
    }
    const r = await sheet.submit();
    if (!r.agent) throw new UiCommandError('failed', r.error);
    if (r.error) throw new UiCommandError('failed', r.error);
    await personalAgents.loadSchedules(r.agent.id);
    return {
      agent: summary(r.agent),
      schedules: (personalAgents.schedulesByAgent[r.agent.id] ?? []).map(scheduleOut),
    };
  },

  async pa_edit(a: PaAgentPatch & { agent: string }, ctx) {
    const agent = await agentFor(a.agent, ctx);
    const { body, lines } = agentChanges(agent, a);
    if (lines.length === 0) return { agent: summary(agent), changed: false };
    await openAgent(agent, 'overview', ctx);
    void highlightWhenReady(ctx, '[data-pa-persona]');
    const ok = await ctx.confirmWrite({
      what: `Change the personal agent “${agent.name}”:\n${lines.map((l) => `• ${l}`).join('\n')}`,
      where: 'Personal Agents',
      connId: `pa:${agent.workspace_id}`,
      verb: 'Save',
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the change.');
    try {
      const next = await personalAgents.update(agent.id, body);
      toasts.success(`Saved ${next.name}`, agentLabel(ctx.agent));
      return { agent: summary(next), changed: true };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async pa_set_autonomy(
    a: {
      agent: string;
      primary?: boolean;
      proactive?: Partial<PersonalAgentAutonomy['proactive']>;
      goals?: string[];
      rules?: string[];
    },
    ctx,
  ) {
    const agent = await agentFor(a.agent, ctx);
    const page = await openAgent(agent, 'autonomy', ctx);
    let current: PersonalAgentAutonomy;
    try {
      current = await personalAgentsApi.autonomy(agent.id);
    } catch (e) {
      throw asUiError(e);
    }
    const body: Parameters<typeof personalAgentsApi.saveAutonomy>[1] = {};
    const lines: string[] = [];
    if (a.primary !== undefined && a.primary !== current.primary) {
      body.primary = a.primary;
      lines.push(a.primary ? 'make it your MAIN agent (the one you talk to; it creates and directs the others)' : 'it is no longer your main agent');
    }
    if (a.proactive) {
      const p = { ...current.proactive, ...a.proactive };
      body.proactive = p;
      lines.push(`proactive work ${p.enabled ? `on — up to ${p.runs_per_day} read-only runs a day, ${p.max_minutes} min each` : 'off'}`);
    }
    if (a.goals) {
      body.goals = a.goals.map((text) => ({ text: String(text), id: current.goals.find((g) => g.text === text)?.id }));
      lines.push(`standing goals (replaced): ${a.goals.length ? a.goals.map((g) => `“${g}”`).join(', ') : 'none'}`);
    }
    if (a.rules) {
      body.rules = a.rules.map((text) => ({ text: String(text), id: current.rules.find((r) => r.text === text)?.id }));
      lines.push(`rules (replaced): ${a.rules.length ? a.rules.map((r) => `“${r}”`).join(', ') : 'none'}`);
    }
    if (lines.length === 0) return { agent: agent.name, changed: false, autonomy: current };
    const section = a.primary !== undefined ? 'primary-h' : a.proactive ? 'modes-h' : a.goals ? 'goals-h' : 'rules-h';
    void highlightWhenReady(ctx, `section[aria-labelledby="${section}"]`);
    const ok = await ctx.confirmWrite({
      what: `Change the autonomy of “${agent.name}”:\n${lines.map((l) => `• ${l}`).join('\n')}`,
      where: 'Personal Agents · Autonomy',
      connId: `pa:${agent.workspace_id}`,
      verb: 'Save',
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the change.');
    try {
      const next = await personalAgentsApi.saveAutonomy(agent.id, body);
      page.reloadAutonomy();
      toasts.success(`Saved ${agent.name}’s autonomy`, agentLabel(ctx.agent));
      return { agent: agent.name, changed: true, autonomy: next };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async pa_set_slack_as_me(a: { agent: string; teams: string[] }, ctx) {
    if (!Array.isArray(a.teams)) throw new UiCommandError('invalid_args', 'teams must be a list (empty = turn it off).');
    const agent = await agentFor(a.agent, ctx);
    const page = await openAgent(agent, 'autonomy', ctx);
    const path = `/personal-agents/${encodeURIComponent(agent.id)}/slack-as-me`;
    let grant: PersonalAgentSlackGrant;
    try {
      grant = await api.get<PersonalAgentSlackGrant>(path);
    } catch (e) {
      throw asUiError(e);
    }
    if (!grant.is_owner) throw new UiCommandError('forbidden', 'Only this agent’s owner can let it act as them in Slack.');
    const picked = a.teams.map((key) =>
      resolveByIdOrName(grant.available, String(key), (t) => t.team_id, (t) => t.team, 'connected Slack workspace'),
    );
    const ids = [...new Set(picked.map((t) => t.team_id))];
    if ([...ids].sort().join(',') === [...grant.teams].sort().join(',')) {
      return { agent: agent.name, teams: grant.teams, changed: false };
    }
    void highlightWhenReady(ctx, '[data-testid="agent-slack-as-me"]');
    ctx.progress(`Waiting for you to confirm Slack access for “${agent.name}”`, true);
    // Lends the user's own Slack identity: ALWAYS asked, never remembered.
    const ok = await dismissOnAbort(ctx.signal, confirmOutward(slackAsMeConfirm(agent.name, agentLabel(ctx.agent), picked)));
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the Slack permission.');
    try {
      const next = await api.put<PersonalAgentSlackGrant>(path, { teams: ids });
      page.reloadAutonomy();
      toasts.success(ids.length ? `${agent.name} can now act as you in Slack` : `${agent.name} can no longer act as you in Slack`, agentLabel(ctx.agent));
      return { agent: agent.name, teams: next.teams, changed: true };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async pa_add_schedule(a: PaScheduleArg & { agent: string }, ctx) {
    const agent = await agentFor(a.agent, ctx);
    const s = scheduleArg(a, true) as PaScheduleArg;
    s.timezone ||= browserTz();
    await openAgent(agent, 'schedules', ctx);
    const ok = await ctx.confirmWrite({
      what: `Add a schedule to “${agent.name}”: ${scheduleText(s)}.`,
      where: 'Personal Agents · Schedules',
      connId: `pa:${agent.workspace_id}`,
      verb: 'Add schedule',
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the schedule.');
    const before = new Set((personalAgents.schedulesByAgent[agent.id] ?? []).map((x) => x.id));
    try {
      await personalAgents.createSchedule(agent.id, { ...s, idempotency_key: crypto.randomUUID() });
    } catch (e) {
      throw asUiError(e);
    }
    const added = (personalAgents.schedulesByAgent[agent.id] ?? []).find((x) => !before.has(x.id)) ?? null;
    if (added) void highlightWhenReady(ctx, ROW('data-schedule-id', added.id));
    toasts.success(`Added a schedule to ${agent.name}`, agentLabel(ctx.agent));
    return { agent: agent.name, schedule: added ? scheduleOut(added) : null };
  },

  async pa_edit_schedule(a: Partial<PaScheduleArg> & { agent: string; schedule_id: string }, ctx) {
    const agent = await agentFor(a.agent, ctx);
    await personalAgents.loadSchedules(agent.id);
    const cur = (personalAgents.schedulesByAgent[agent.id] ?? []).find((x) => x.id === a.schedule_id);
    if (!cur) throw new UiCommandError('not_found', `“${agent.name}” has no schedule ${a.schedule_id} (otto.ui_pa_open tab schedules lists them).`);
    const patch = scheduleArg(a, false);
    if (Object.keys(patch).length === 0) return { agent: agent.name, schedule: scheduleOut(cur), changed: false };
    await openAgent(agent, 'schedules', ctx);
    void highlightWhenReady(ctx, ROW('data-schedule-id', cur.id));
    const next: PaScheduleArg = {
      schedule: patch.schedule ?? cur.schedule,
      timezone: patch.timezone ?? cur.timezone,
      directive: patch.directive ?? cur.directive,
      permission: patch.permission ?? cur.permission ?? 'directed',
      enabled: patch.enabled ?? cur.enabled,
    };
    const ok = await ctx.confirmWrite({
      what: `Change a schedule of “${agent.name}”:\nfrom: ${scheduleText({ ...cur, permission: cur.permission ?? 'directed' })}\nto: ${scheduleText(next)}`,
      where: 'Personal Agents · Schedules',
      connId: `pa:${agent.workspace_id}`,
      verb: patch.enabled === false && Object.keys(patch).length === 1 ? 'Pause' : 'Save',
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the change.');
    try {
      await personalAgents.updateSchedule(agent.id, cur.id, patch);
    } catch (e) {
      throw asUiError(e);
    }
    const saved = (personalAgents.schedulesByAgent[agent.id] ?? []).find((x) => x.id === cur.id);
    toasts.success(`Saved ${agent.name}’s schedule`, agentLabel(ctx.agent));
    return { agent: agent.name, schedule: saved ? scheduleOut(saved) : null, changed: true };
  },

  async pa_run_now(a: { agent: string; schedule_id?: string }, ctx) {
    const agent = await agentFor(a.agent, ctx);
    await openAgent(agent, 'runs', ctx);
    const who = agentLabel(ctx.agent);
    const dest = savedDestWhere(agent.delivery);
    if (dest) ctx.progress(`Waiting for you to confirm running “${agent.name}”`, true);
    const ok = dest
      ? await dismissOnAbort(ctx.signal, confirmOutward({
          verb: 'Run and deliver',
          title: `${who} wants to run “${agent.name}” now`,
          where: `Personal agent “${agent.name}” → ${dest}`,
          what: `The report of a run that starts now${a.schedule_id ? ' (that schedule’s directive)' : ''}.`,
          who: destWho(agent.delivery),
        }))
      : await ctx.confirmWrite({ what: `Run the personal agent “${agent.name}” once now`, where: 'Personal Agents', connId: `pa:${agent.workspace_id}`, verb: 'Run' });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the run.');
    try {
      await personalAgents.runNow(agent.id, a.schedule_id);
    } catch (e) {
      throw asUiError(e);
    }
    toasts.success(`${agent.name} is running`, who);
    const latest = (personalAgents.runsByAgent[agent.id] ?? [])[0] ?? null;
    if (latest) void highlightWhenReady(ctx, ROW('data-run-id', latest.id));
    return { agent: agent.name, started: true, run: latest ? { id: latest.id, status: latest.status, session_id: latest.session_id ?? null } : null };
  },

  async pa_open_report(a: { agent: string; run?: string }, ctx) {
    const agent = await agentFor(a.agent, ctx);
    const page = await openAgent(agent, 'runs', ctx);
    await personalAgents.loadRuns(agent.id);
    const err = personalAgents.runsError[agent.id];
    if (err) throw new UiCommandError('failed', err);
    const runs = (personalAgents.runsByAgent[agent.id] ?? []).filter((r) => r.report_rel);
    const run = a.run ? runs.find((r) => r.id === a.run) : runs[0];
    if (!run) throw new UiCommandError('not_found', a.run ? `No run ${a.run} with a report.` : `“${agent.name}” has no run with a report yet.`);
    page.viewReport(run);
    void highlightWhenReady(ctx, ROW('data-run-id', run.id));
    let text: string;
    try {
      text = await authedText(personalAgentsApi.reportPath(run.id));
    } catch (e) {
      throw asUiError(e);
    }
    const max = 60_000;
    return {
      agent: agent.name,
      run: { id: run.id, status: run.status, started_at: run.started_at, summary: run.summary },
      report: text.length > max ? text.slice(0, max) : text,
      truncated: text.length > max,
    };
  },
});
