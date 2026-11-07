<!--
  The agent's side panel beside its chat — one calm scroll:
  · a pending "update instructions?" proposal (only when there is one);
  · Routines — what it runs on a schedule, in plain words and local time,
    with the next run on the same line; ⋯ per routine (Run now, Pause, Edit,
    Delete); "Add routine" opens the form (or ask the agent in chat);
  · Now — only while something runs (runs, threads, hand-offs);
  · Recent runs — the last few, each opens its report or thread.
  Who it is, personality, model, memory, Slack and rules live in Settings.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { confirmRunAndDeliver } from '../../lib/confirmOutward';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { runStatus } from '../../lib/status';
  import { now } from '../../lib/stores/now.svelte';
  import { routineLabel, runBadge } from '../../lib/personalAgentBadges';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { browserTz, routineWhen, scheduleForDays, workdays } from './cadence';
  import { firstLine, nextRunWords } from './agentHome';
  import { leadLine } from '../../lib/plainText';
  import RunReport from '../../lib/components/RunReport.svelte';
  import RoutineForm from './RoutineForm.svelte';
  import type {
    PersonalAgent,
    PersonalAgentInstructionsProposal,
    PersonalAgentRoutine,
    PersonalAgentRun,
    PersonalAgentSchedule,
  } from '../../lib/api/types';

  interface Props {
    agent: PersonalAgent;
    onclose: () => void;
  }
  let { agent, onclose }: Props = $props();

  const schedules = $derived(personalAgents.schedulesByAgent[agent.id] ?? []);
  const runs = $derived(personalAgents.runsByAgent[agent.id] ?? []);
  const running = $derived(runs.filter((r) => r.status === 'running'));
  /** A run event can land before the runs list refetches: say so anyway. */
  const runStarting = $derived(!!personalAgents.runningByAgent[agent.id] && running.length === 0);
  const recent = $derived(runs.filter((r) => r.status !== 'running').slice(0, 5));
  /** Busy threads that are not a running run's own thread (listed above). */
  const runThreadIds = $derived(new Set(running.map((r) => r.session_id).filter(Boolean)));
  const busyThreads = $derived(
    personalAgents
      .threadsOf(agent.id, false)
      .filter((t) => (t.status === 'working' || t.status === 'starting' || t.status === 'awaiting_permission') && !runThreadIds.has(t.id) && !t.meta?.run_id),
  );

  $effect(() => {
    const id = agent.id;
    // The loads read their own loading flags: untracked, or this re-runs forever.
    untrack(() => {
      void personalAgents.loadRuns(id);
      void personalAgents.loadSchedules(id);
    });
  });

  // Routines with their next runs: re-read whenever the agent's schedules
  // change (any path — the chat's `otto.routine_*` tools included).
  let routines = $state<PersonalAgentRoutine[]>([]);
  $effect(() => {
    const id = agent.id;
    void personalAgents.schedulesByAgent[id];
    void agent.enabled;
    personalAgentsApi
      .routines(id)
      .then((r) => { if (id === agent.id) routines = r.routines; })
      .catch(() => {});
  });

  /** The routine form: null = closed, 'new', or the routine being edited. */
  let form = $state<'new' | PersonalAgentSchedule | null>(null);
  $effect(() => {
    if (personalAgents.routineFormFor !== agent.id) return;
    personalAgents.routineFormFor = null;
    form = 'new';
  });

  // --- Instruction proposals --------------------------------------------------
  const myRole = $derived(ws.workspaces.find((w) => w.id === agent.workspace_id)?.my_role ?? 'viewer');
  const isOwner = $derived(auth.isRoot || myRole === 'admin' || !agent.created_by || agent.created_by === auth.me?.id);
  let prop = $state<PersonalAgentInstructionsProposal | null>(null);
  let editText = $state<string | null>(null);
  $effect(() => {
    const id = agent.id;
    void agent.soul_md;
    void personalAgents.autonomyById[id];
    personalAgentsApi
      .proposal(id)
      .then((p) => { if (id === agent.id) prop = p; })
      .catch(() => {});
  });
  async function applyProposal(): Promise<void> {
    try {
      await personalAgentsApi.applyProposal(agent.id, editText ?? undefined);
      editText = null;
      toasts.success('Instructions updated', 'Applies from its next session. You can undo it here.');
    } catch (e) {
      toasts.error('Couldn’t update the instructions', loadErrorText(e));
    }
  }
  async function dismissProposal(): Promise<void> {
    try {
      await personalAgentsApi.dismissProposal(agent.id);
      prop = prop ? { ...prop, proposal: null } : prop;
    } catch (e) {
      toasts.error('Couldn’t dismiss it', loadErrorText(e));
    }
  }
  async function undoInstructions(): Promise<void> {
    try {
      await personalAgentsApi.undoInstructions(agent.id);
      toasts.success('Restored the previous instructions');
    } catch (e) {
      toasts.error('Couldn’t undo', loadErrorText(e));
    }
  }

  // --- Routine actions ---------------------------------------------------------
  const when = (s: PersonalAgentSchedule): string => routineWhen(s);

  async function toggleRoutine(s: PersonalAgentSchedule): Promise<void> {
    try {
      await personalAgents.updateSchedule(agent.id, s.id, { enabled: !s.enabled });
    } catch (e) {
      toasts.error(s.enabled ? 'Couldn’t pause the routine' : 'Couldn’t resume the routine', loadErrorText(e));
    }
  }
  async function runRoutine(s: PersonalAgentSchedule): Promise<void> {
    if (!(await confirmRunAndDeliver(agent.name, agent.delivery))) return;
    try {
      await personalAgents.runNow(agent.id, s.id);
      toasts.success(`${routineLabel(s.directive)} is running`, 'Its result is posted into the chat when it finishes.');
    } catch (e) {
      toasts.error('Couldn’t run the routine', loadErrorText(e));
    }
  }
  async function deleteRoutine(s: PersonalAgentSchedule): Promise<void> {
    if (!(await confirmer.ask(`Delete the routine “${routineLabel(s.directive)}” (${when(s)})? Past runs stay.`, { title: 'Delete routine', danger: true, confirmLabel: 'Delete' }))) return;
    try {
      await personalAgents.deleteSchedule(agent.id, s.id);
    } catch (e) {
      toasts.error('Couldn’t delete the routine', loadErrorText(e));
    }
  }
  function routineMenu(e: MouseEvent, s: PersonalAgentSchedule): void {
    ctxMenu.show(e, [
      { label: 'Run now', icon: 'play', action: () => void runRoutine(s) },
      // While the whole agent is paused, a per-routine pause is a second,
      // invisible level — offered only on an active agent.
      ...(agent.enabled ? [{ label: s.enabled ? 'Pause' : 'Resume', icon: s.enabled ? 'pause' : 'refresh', action: () => void toggleRoutine(s) } as const] : []),
      { label: 'Edit…', icon: 'edit', action: () => (form = s) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteRoutine(s) },
    ]);
  }
  const nextOf = (s: PersonalAgentSchedule): string | null => routines.find((x) => x.id === s.id)?.next_runs[0] ?? s.next_run_at ?? null;
  /** A state worth its own line (paused, won't run, starting), else null. */
  function warnLine(s: PersonalAgentSchedule): string | null {
    if (!s.enabled) return 'Paused';
    if (!agent.enabled) return 'Won’t run — the agent is paused';
    const next = nextOf(s);
    if (!next) return 'Not scheduled';
    // Due, and the scheduler's minute tick hasn't picked it up yet.
    return Date.parse(next) <= now() ? 'Starting…' : null;
  }
  function dismissDraft(): void {
    const { [agent.id]: _gone, ...rest } = personalAgents.routineDrafts;
    personalAgents.routineDrafts = rest;
  }
  /** "Sun–Thu at 09:00 · next Tomorrow, 09:00" — one line when all is well. */
  function whenLine(s: PersonalAgentSchedule): string {
    const next = warnLine(s) ? null : nextOf(s);
    return next ? `${when(s)} · next ${nextRunWords(next)}` : when(s);
  }
  /** At most this many routines before "Show all". */
  const ROUTINE_CAP = 4;
  let showAll = $state(false);
  $effect(() => {
    void agent.id;
    showAll = false;
  });
  /** A routine running now or finished in the last few minutes first (so the
   *  one you are waiting for never drops behind "Show all"), then soonest
   *  next run; paused / won't-run last (in creation order). */
  const RECENT_MS = 10 * 60_000;
  const sortedSchedules = $derived.by(() => {
    const active = new Set(
      runs
        .filter((r) => r.schedule_id && (r.status === 'running' || Date.now() - Date.parse(r.finished_at ?? r.started_at) < RECENT_MS))
        .map((r) => r.schedule_id as string),
    );
    const key = (s: PersonalAgentSchedule): number => {
      if (active.has(s.id)) return Number.NEGATIVE_INFINITY;
      const n = s.enabled && agent.enabled ? nextOf(s) : null;
      return n ? Date.parse(n) : Number.POSITIVE_INFINITY;
    };
    return [...schedules].sort((a, b) => {
      const x = key(a), y = key(b);
      return x === y ? 0 : x < y ? -1 : 1;
    });
  });

  async function stopRun(r: PersonalAgentRun): Promise<void> {
    try {
      await personalAgentsApi.cancelRun(r.id);
      toasts.info('Stopping the run…');
    } catch (e) {
      toasts.error('Couldn’t stop the run', loadErrorText(e));
    }
  }

  /** The run's routine name ('' for a run without one). */
  function routineOf(r: PersonalAgentRun): string {
    const s = r.schedule_id ? schedules.find((x) => x.id === r.schedule_id) : undefined;
    return s ? routineLabel(s.directive) : '';
  }

  function runName(r: PersonalAgentRun): string {
    const s = r.schedule_id ? schedules.find((x) => x.id === r.schedule_id) : undefined;
    return runBadge(r, s ? routineLabel(s.directive) : null).label;
  }

  // --- Report viewer ---------------------------------------------------------
  let reportRun = $state<PersonalAgentRun | null>(null);
  function openRun(r: PersonalAgentRun): void {
    reportRun = r;
  }
  // A finished run reads as its result; anything else as its name, with the
  // reason (stopped / failed) in the subline.
  const ok = (r: PersonalAgentRun) => r.status === 'ok';
  const summaryOf = (r: PersonalAgentRun): string => (ok(r) && leadLine(r.summary)) || runName(r);
  const sublineOf = (r: PersonalAgentRun): string => (ok(r) ? runName(r) : firstLine(r.error ?? '') || runStatus(r.status).label);
</script>

<svelte:window onkeydown={(e) => {
  // Where the panel overlays the chat (narrow windows), Escape closes it —
  // unless a dialog of its own is open (that dialog takes the Escape).
  if (e.key === 'Escape' && !e.defaultPrevented && !reportRun && !form && window.matchMedia('(max-width: 1024px)').matches) onclose();
}} />

<aside class="sp" aria-label="{agent.name}: routines and runs" data-testid="pa-side-panel">
  <div class="sp-head">
    <h2><span class="sp-who" dir="auto">{agent.name}</span> · Routines &amp; runs</h2>
    <button type="button" class="icon-btn" aria-label="Settings for {agent.name}" title="Settings" onclick={() => router.go(`personal-agents/${agent.id}/about`)}><Icon name="gear" size={14} /></button>
    <button type="button" class="icon-btn" aria-label="Close panel" title="Close panel" onclick={onclose}><Icon name="x" size={14} /></button>
  </div>

  <div class="sp-body">
    {#if prop?.proposal}
      <section class="sp-sec sp-card" data-testid="pa-proposal">
        <h3>Update instructions?</h3>
        <p class="sp-dim">{agent.name} suggests a change{prop.proposal.reason ? `: ${prop.proposal.reason}` : '.'}</p>
        {#if editText !== null}
          <textarea class="input" rows="8" dir="auto" aria-label="Edit the new instructions" bind:value={editText}></textarea>
        {:else}
          <div class="sp-diff">
            <span class="sp-dim">From</span>
            <pre class="sp-pre from">{prop.proposal.from || '(empty)'}</pre>
            <span class="sp-dim">To</span>
            <pre class="sp-pre to">{prop.proposal.to}</pre>
          </div>
        {/if}
        {#if isOwner}
          <div class="sp-row">
            <button type="button" class="btn small primary" onclick={() => void applyProposal()}>{editText !== null ? 'Apply edited' : 'Apply'}</button>
            {#if editText === null}<button type="button" class="btn small" onclick={() => (editText = prop?.proposal?.to ?? '')}>Edit first</button>{/if}
            <button type="button" class="btn small ghost" onclick={() => void dismissProposal()}>Dismiss</button>
          </div>
        {:else}
          <p class="sp-dim">Only its owner can change its instructions.</p>
        {/if}
      </section>
    {:else if prop?.can_undo && isOwner}
      <p class="sp-dim">Instructions were updated. <button type="button" class="linklike" onclick={() => void undoInstructions()}>Undo</button></p>
    {/if}

    <section class="sp-sec" data-testid="pa-routines">
      <div class="sp-sec-head">
        <h3>Routines</h3>
        <button type="button" class="btn small" data-testid="pa-add-routine" onclick={() => (form = 'new')}><Icon name="plus" size={12} />Add routine</button>
      </div>
      {#if personalAgents.schedulesLoading[agent.id] && schedules.length === 0}
        <Skeleton rows={2} height={40} label="routines" />
      {:else}
        {#if personalAgents.schedulesError[agent.id]}
          <!-- A failed refresh keeps the routines it had (with Retry). -->
          <LoadState what="this agent’s routines" variant="compact" error={personalAgents.schedulesError[agent.id]} empty={schedules.length === 0} onretry={() => void personalAgents.loadSchedules(agent.id)} />
        {/if}
        {#if schedules.length === 0 && !personalAgents.schedulesError[agent.id]}
          {@const draft = personalAgents.routineDrafts[agent.id]}
          {#if draft}
            <div class="sp-suggest" data-testid="pa-routine-suggestion">
              <span class="sp-item-t"><span class="sp-dim">Suggested</span><strong class="ellipsis" dir="auto">{draft.name}</strong><span class="sp-dim ellipsis">{routineWhen({ schedule: scheduleForDays(workdays(), '09:00'), timezone: browserTz() })}</span></span>
              <button type="button" class="btn small primary" onclick={() => (form = 'new')}>Add</button>
              <button type="button" class="icon-btn" aria-label="Dismiss the suggested routine" title="Dismiss" onclick={dismissDraft}><Icon name="x" size={12} /></button>
            </div>
          {:else}
            <p class="sp-dim">Nothing scheduled yet — add one, or ask {agent.name} in chat.</p>
          {/if}
        {/if}
        {#if schedules.length > 0}
        <ul class="sp-list">
          {#each showAll ? sortedSchedules : sortedSchedules.slice(0, ROUTINE_CAP) as s (s.id)}
            {@const r = routines.find((x) => x.id === s.id)}
            {@const warn = warnLine(s)}
            <li class="sp-routine" class:off={!s.enabled || !agent.enabled} data-schedule-id={s.id}>
              <button type="button" class="sp-routine-main" title="Edit this routine" onclick={() => (form = s)}>
                <span class="sp-routine-t"><strong class="ellipsis" dir="auto" title={s.directive}>{routineLabel(s.directive)}</strong>{#if s.permission === 'read_only'}<span class="sp-tag" title="This routine only reads — it can’t send, post, write or change anything">read-only</span>{/if}</span>
                <span class="sp-dim ellipsis">{whenLine(s)}{r?.deliver.type === 'slack' ? ` · to Slack${r.deliver.as === 'me' ? ' as you' : ''}` : ''}</span>
                {#if warn}<span class="sp-next" class:warn={s.enabled && !agent.enabled}>{warn}</span>{/if}
              </button>
              <button type="button" class="icon-btn" aria-label="Actions for {routineLabel(s.directive)}" title="Run now, pause, edit or delete" onclick={(e) => routineMenu(e, s)}><Icon name="more" size={14} /></button>
            </li>
          {/each}
        </ul>
        {#if sortedSchedules.length > ROUTINE_CAP}
          <button type="button" class="linklike" onclick={() => (showAll = !showAll)}>{showAll ? 'Show fewer' : `Show all (${sortedSchedules.length})`}</button>
        {/if}
        {/if}
      {/if}
    </section>

    {#if running.length > 0 || busyThreads.length > 0 || runStarting}
      <section class="sp-sec" data-testid="pa-activity">
        <h3>Now</h3>
        <ul class="sp-list">
          {#if runStarting}
            <li class="sp-item"><span class="sp-item-main static"><span class="spinner" aria-hidden="true"></span><span class="sp-item-t"><span>Starting a run…</span></span></span></li>
          {/if}
          {#each running as r (r.id)}
            <li class="sp-item">
              <span class="sp-item-main static"><span class="spinner" aria-hidden="true"></span><span class="sp-item-t"><span class="ellipsis">{runName(r)}</span><span class="sp-dim">started <RelTime iso={r.started_at} /></span></span></span>
              <button type="button" class="btn small" onclick={() => void stopRun(r)}>Stop</button>
            </li>
          {/each}
          {#each busyThreads as t (t.id)}
            <li class="sp-item">
              <button type="button" class="sp-item-main" onclick={() => router.go(`threads/${t.id}`)}>
                {#if t.status === 'awaiting_permission'}<Icon name="hand" size={14} />{:else}<span class="spinner" aria-hidden="true"></span>{/if}
                <span class="sp-item-t">
                  <span>{t.status === 'awaiting_permission' ? 'Waiting for your approval' : 'Working…'}</span>
                  <span class="sp-dim ellipsis" dir="auto">{t.title}</span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="sp-sec" data-testid="pa-recent">
      <h3>Recent runs</h3>
      {#if personalAgents.runsByAgent[agent.id] === undefined}
        <Skeleton rows={3} height={36} label="recent runs" />
      {:else if personalAgents.runsError[agent.id] && runs.length === 0}
        <LoadState what="recent runs" error={personalAgents.runsError[agent.id]} empty onretry={() => void personalAgents.loadRuns(agent.id)} />
      {:else if recent.length === 0}
        <p class="sp-dim">{running.length > 0 || runStarting ? 'Its result lands here when it’s done.' : 'No runs yet. When a routine runs, its result lands in the chat and here.'}</p>
      {:else}
        <ul class="sp-list">
          {#each recent as r (r.id)}
            <li class="sp-item">
              <button type="button" class="sp-item-main" title={r.report_rel ? 'Open the report' : 'Open the run'} onclick={() => openRun(r)}>
                <span class="sp-st sp-st-{runStatus(r.status).tone}" aria-hidden="true"></span>
                <span class="sp-item-t">
                  <span class="ellipsis" dir="auto">{summaryOf(r)}</span>
                  <span class="sp-dim ellipsis" dir="auto">{sublineOf(r)} · <RelTime iso={r.finished_at ?? r.started_at} /></span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
        <button type="button" class="linklike" onclick={() => router.go(`personal-agents/${agent.id}/runs`)}>All runs</button>
      {/if}
    </section>
  </div>

</aside>

{#if form}
  <RoutineForm {agent} routine={form === 'new' ? null : form} onclose={() => (form = null)} />
{/if}

{#if reportRun}
  <RunReport run={reportRun} routine={routineOf(reportRun)} name={runName(reportRun)} onclose={() => (reportRun = null)} />
{/if}

<style>
  .sp {
    display: flex; flex-direction: column; min-block-size: 0; inline-size: 320px; flex: none;
    border-inline-start: 1px solid var(--border); background: var(--surface);
  }
  .sp-head { display: flex; align-items: center; gap: 8px; padding: 10px 12px; border-block-end: 1px solid var(--border); }
  .sp-head h2 { flex: 1; min-inline-size: 0; margin: 0; font-size: var(--fs-m); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sp-body { flex: 1; overflow-y: auto; padding: 12px; display: flex; flex-direction: column; gap: 20px; min-block-size: 0; }
  .sp-sec { display: flex; flex-direction: column; gap: 8px; }
  .sp-sec h3 { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.06em; }
  .sp-dim { margin: 0; color: var(--text-dim); font-size: var(--fs-s); }
  .sp-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .sp-routine {
    display: flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 10px 6px; border: 1px solid var(--border);
    border-radius: var(--radius-m); background: var(--bg);
  }
  .sp-routine.off strong { color: var(--text-dim); }
  .sp-routine-main {
    flex: 1; min-inline-size: 0; display: flex; flex-direction: column; gap: 1px; font-size: var(--fs-s);
    padding: 2px 4px; margin: -2px -4px; border: 0; border-radius: var(--radius-m); background: transparent;
    color: inherit; font-family: inherit; text-align: start; cursor: pointer;
  }
  .sp-routine-main:hover { background: var(--hover); }
  .sp-routine-main:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; }
  .sp-next { font-size: var(--fs-s); color: var(--text); }
  .sp-next.warn { color: var(--warning); }
  .sp-item { display: flex; align-items: center; gap: 4px; }
  .sp-item-main {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 8px; padding: 6px; border: 0;
    border-radius: var(--radius-m); background: transparent; color: var(--text); font: inherit; text-align: start; cursor: pointer;
  }
  .sp-item-main.static { cursor: default; }
  .sp-item-main:not(.static):hover { background: var(--hover); }
  .sp-item-main:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; }
  .sp-item-main :global(svg) { flex: none; color: var(--text-dim); }
  .sp-item-t { display: flex; flex-direction: column; min-inline-size: 0; font-size: var(--fs-s); }
  .sp-st { flex: none; inline-size: 8px; block-size: 8px; border-radius: 50%; background: var(--text-dim); }
  .sp-st-success { background: var(--success); }
  .sp-st-danger { background: var(--danger); }
  .sp-st-warning { background: var(--warning); }
  .sp-st-info { background: var(--info); }
  .sp-row { display: flex; gap: 6px; align-items: center; }
  .sp-row > :global(.input) { flex: 1; min-inline-size: 0; }
  .sp-sec-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .sp-routine-t { display: flex; align-items: center; gap: 6px; min-inline-size: 0; }
  .sp-who { font-weight: 600; }
  .sp-card { border: 1px solid var(--accent-line); border-radius: var(--radius-m); padding: 10px; background: var(--accent-faint); }
  .sp-card h3 { color: var(--text); text-transform: none; letter-spacing: 0; font-size: var(--fs-m); }
  .sp-diff { display: grid; gap: 4px; }
  .sp-pre { margin: 0; max-block-size: 160px; overflow: auto; white-space: pre-wrap; word-break: break-word; font-family: inherit; font-size: var(--fs-s); padding: 6px 8px; border-radius: var(--radius-s); background: var(--surface); border: 1px solid var(--border); }
  .sp-pre.from { color: var(--text-dim); }
  .linklike { align-self: flex-start; border: 0; background: none; padding: 0; color: var(--accent-text); font: inherit; font-size: var(--fs-s); cursor: pointer; text-align: start; }
  .linklike:hover { text-decoration: underline; }
  @media (max-width: 1024px) {
    .sp { position: absolute; inset-block: 0; inset-inline-end: 0; z-index: 3; box-shadow: var(--glass-shadow); }
  }
  @media (max-width: 640px) {
    .sp { inline-size: 100%; }
  }
  .sp-tag { display: inline-block; padding: 0 6px; border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-xs); color: var(--text-dim); }
  .sp-suggest {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-m);
  }
  .sp-suggest .sp-item-t { flex: 1; min-width: 0; }
</style>
