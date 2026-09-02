<script lang="ts">
  // One personal agent: Overview / Schedules / Runs / Chat / Memory tabs.
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { authedText } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import AgentEditSheet from './AgentEditSheet.svelte';
  import {
    browserTz,
    buildCadence,
    cadenceLabel,
    defaultCadence,
    loadCadence,
    WEEKDAYS,
    type CadenceForm,
  } from './cadence';
  import type { PersonalAgentRun, PersonalAgentSchedule } from '../../lib/api/types';

  interface Props {
    agentId: string;
  }
  let { agentId }: Props = $props();

  type Tab = 'overview' | 'schedules' | 'runs' | 'chat' | 'memory';
  let tab = $state<Tab>('overview');
  let editing = $state(false);
  let busy = $state(false);
  let error = $state('');

  const agent = $derived(personalAgents.agent(agentId));
  const schedules = $derived(personalAgents.schedulesByAgent[agentId] ?? []);
  const runs = $derived(personalAgents.runsByAgent[agentId] ?? []);

  // The agent may not be cached yet (deep link) — the list load fills it in.
  $effect(() => {
    if (!personalAgents.agent(agentId) && ws.currentId) void personalAgents.loadAgents(ws.currentId);
  });

  $effect(() => {
    if (tab === 'runs') void personalAgents.loadRuns(agentId);
  });

  // --- Chat: one interactive session pinned to the agent's provider/model/cwd.
  let chatSessionId = $state('');
  let chatError = $state('');
  $effect(() => {
    if (tab !== 'chat' || chatSessionId) return;
    chatError = '';
    personalAgentsApi
      .chatSession(agentId)
      .then((r) => (chatSessionId = r.session_id))
      .catch((e) => (chatError = e instanceof Error ? e.message : 'Failed to open the chat session'));
  });

  // --- Schedules form -------------------------------------------------------
  let schedFormOpen = $state(false);
  let schedEditId = $state<string | null>(null);
  let sf = $state<CadenceForm>(defaultCadence());
  let sfTimezone = $state(browserTz());
  let sfDirective = $state('');
  let sfEnabled = $state(true);

  function openSchedCreate(): void {
    schedEditId = null;
    sf = defaultCadence();
    sfTimezone = browserTz();
    sfDirective = '';
    sfEnabled = true;
    schedFormOpen = true;
    error = '';
  }

  function openSchedEdit(s: PersonalAgentSchedule): void {
    schedEditId = s.id;
    sf = loadCadence(s.schedule);
    sfTimezone = s.timezone || browserTz();
    sfDirective = s.directive;
    sfEnabled = s.enabled;
    schedFormOpen = true;
    error = '';
  }

  async function saveSchedule(): Promise<void> {
    busy = true;
    error = '';
    const body = {
      schedule: buildCadence(sf),
      timezone: sfTimezone.trim() || 'UTC',
      directive: sfDirective,
      enabled: sfEnabled,
    };
    try {
      if (schedEditId) await personalAgents.updateSchedule(agentId, schedEditId, body);
      else await personalAgents.createSchedule(agentId, body);
      schedFormOpen = false;
    } catch (e) {
      error = e instanceof Error ? e.message : 'Save failed';
    } finally {
      busy = false;
    }
  }

  async function toggleSchedule(s: PersonalAgentSchedule): Promise<void> {
    try {
      await personalAgents.updateSchedule(agentId, s.id, { enabled: !s.enabled });
    } catch (e) {
      error = e instanceof Error ? e.message : 'Toggle failed';
    }
  }

  async function deleteSchedule(s: PersonalAgentSchedule): Promise<void> {
    if (!confirm('Delete this schedule?')) return;
    try {
      await personalAgents.deleteSchedule(agentId, s.id);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Delete failed';
    }
  }

  async function runNow(scheduleId?: string): Promise<void> {
    busy = true;
    error = '';
    try {
      await personalAgents.runNow(agentId, scheduleId);
      tab = 'runs';
    } catch (e) {
      error = e instanceof Error ? e.message : 'Run failed';
    } finally {
      busy = false;
    }
  }

  // --- Runs / report viewer -------------------------------------------------
  let reportOpen = $state(false);
  let reportText = $state('');
  let reportLoading = $state(false);

  async function viewReport(run: PersonalAgentRun): Promise<void> {
    reportOpen = true;
    reportLoading = true;
    reportText = '';
    try {
      reportText = await authedText(personalAgentsApi.reportPath(run.id));
    } catch (e) {
      reportText = e instanceof Error ? `Failed to load report: ${e.message}` : 'Failed to load report';
    } finally {
      reportLoading = false;
    }
  }

  function duration(r: PersonalAgentRun): string {
    if (!r.finished_at) return '…';
    const ms = Date.parse(r.finished_at) - Date.parse(r.started_at);
    if (!Number.isFinite(ms) || ms < 0) return '';
    if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
    return `${Math.round(ms / 60_000)}m`;
  }

  function statusClass(status: string | null | undefined): string {
    switch (status) {
      case 'ok': return 'pill ok';
      case 'error': return 'pill bad';
      case 'running': return 'pill working';
      default: return 'pill';
    }
  }

  function scheduleName(r: PersonalAgentRun): string {
    if (!r.schedule_id) return 'manual';
    const s = schedules.find((x) => x.id === r.schedule_id);
    return s ? cadenceLabel(s.schedule, s.timezone) : 'schedule';
  }

  function deliveryLabel(): string {
    return ((agent?.delivery?.type as string) ?? 'none') as string;
  }

  const memoryRoot = $derived(
    agent?.cwd?.trim() ? agent.cwd : `<otto data dir>/personal/${agentId}/`,
  );
</script>

<div class="page">
  <header class="head">
    <button class="btn small" onclick={() => router.go('personal-agents')}>← Agents</button>
    {#if agent}
      <span class="avatar" aria-hidden="true">{agent.avatar || agent.name.slice(0, 1)}</span>
      <h1>{agent.name}</h1>
      <span class="chip mono">{agent.provider}{agent.model ? ` · ${agent.model}` : ''}</span>
      {#if !agent.enabled}<span class="pill">paused</span>{/if}
      {#if agent.browser}<span class="pill" title="otto-browser MCP attached to runs and chat">browser</span>{/if}
      <span class="grow"></span>
      <button class="btn small" disabled={busy} onclick={() => runNow()}>Run now</button>
      <button class="btn small" onclick={() => (editing = true)}>Edit</button>
    {:else}
      <h1>Personal agent</h1>
    {/if}
  </header>

  {#if error}<div class="err" role="alert">{error}</div>{/if}

  <div class="tabs" role="tablist" aria-label="Agent sections">
    {#each [['overview', 'Overview'], ['schedules', 'Schedules'], ['runs', 'Runs'], ['chat', 'Chat'], ['memory', 'Memory']] as [id, label] (id)}
      <button
        class="tab"
        class:active={tab === id}
        role="tab"
        aria-selected={tab === id}
        onclick={() => (tab = id as Tab)}
      >{label}</button>
    {/each}
  </div>

  {#if !agent}
    <div class="muted">{personalAgents.loadingAgents ? 'Loading…' : 'Agent not found in this workspace.'}</div>
  {:else if tab === 'overview'}
    <div class="overview">
      <section class="card">
        <h2>Persona</h2>
        {#if agent.soul_md.trim()}
          <pre class="soul">{agent.soul_md}</pre>
        {:else}
          <p class="muted">No persona yet — Edit to give this agent a soul.</p>
        {/if}
      </section>
      <section class="card">
        <h2>Configuration</h2>
        <dl class="props">
          <dt>Provider</dt><dd class="mono">{agent.provider}</dd>
          <dt>Model</dt><dd class="mono">{agent.model || 'provider default'}</dd>
          <dt>Delivery</dt><dd>{deliveryLabel()}</dd>
          <dt>Browser use</dt><dd>{agent.browser ? 'on' : 'off'}</dd>
          <dt>Enabled</dt><dd>{agent.enabled ? 'yes' : 'no — schedules are paused'}</dd>
          <dt>Workspace dir</dt><dd class="mono wrap">{agent.cwd || `(default) …/personal/${agent.id}/`}</dd>
          <dt>Schedules</dt><dd>{schedules.length} — next run {personalAgents.nextRunAt(agent.id) ?? '—'}</dd>
        </dl>
      </section>
    </div>
  {:else if tab === 'schedules'}
    <div class="section-head">
      <p class="hint">Each schedule has its own cadence, directive, and run cursor.</p>
      <button class="btn small primary" onclick={openSchedCreate}>Add schedule</button>
    </div>
    {#if schedFormOpen}
      <div class="form card">
        <div class="row">
          <label class="fld">
            <span>Cadence</span>
            <select bind:value={sf.cadence}>
              <option value="interval">Interval</option>
              <option value="daily">Daily</option>
              <option value="weekly">Weekly</option>
              <option value="cron">Cron</option>
            </select>
          </label>
          {#if sf.cadence === 'interval'}
            <label class="fld">
              <span>Every (minutes, min 5)</span>
              <input type="number" min="5" bind:value={sf.everyMin} />
            </label>
          {:else if sf.cadence === 'cron'}
            <label class="fld">
              <span>Cron expression (5 fields)</span>
              <input bind:value={sf.cronExpr} placeholder="0 9 * * 1" />
            </label>
          {:else}
            <label class="fld">
              <span>At (HH:MM)</span>
              <input bind:value={sf.at} placeholder="09:00" />
            </label>
            {#if sf.cadence === 'weekly'}
              <label class="fld">
                <span>Weekday</span>
                <select bind:value={sf.weekday}>
                  {#each WEEKDAYS as d, i (d)}<option value={i}>{d}</option>{/each}
                </select>
              </label>
            {/if}
          {/if}
          {#if sf.cadence !== 'interval'}
            <label class="fld">
              <span>Timezone</span>
              <input bind:value={sfTimezone} placeholder="e.g. Europe/London" />
            </label>
          {/if}
        </div>
        <label class="fld">
          <span>Directive (the run's task prompt)</span>
          <textarea bind:value={sfDirective} rows="4" placeholder="Produce the daily recap…"></textarea>
        </label>
        <label class="chk"><input type="checkbox" bind:checked={sfEnabled} /> Enabled</label>
        <div class="actions">
          <button class="btn primary" disabled={busy} onclick={saveSchedule}>{busy ? 'Saving…' : 'Save'}</button>
          <button class="btn" disabled={busy} onclick={() => (schedFormOpen = false)}>Cancel</button>
        </div>
      </div>
    {/if}
    {#if schedules.length === 0 && !schedFormOpen}
      <div class="muted">No schedules — this agent only runs when you say Run now, or when you chat.</div>
    {:else}
      <ul class="rows">
        {#each schedules as s (s.id)}
          <li class="rowline">
            <div class="rowmain">
              <strong>{cadenceLabel(s.schedule, s.timezone)}</strong>
              {#if !s.enabled}<span class="pill">paused</span>{/if}
              <span class="meta">next {s.next_run_at ?? '—'} · last {s.last_run_at ?? 'never'}</span>
              <p class="directive">{s.directive || '(no directive)'}</p>
            </div>
            <div class="rowactions">
              <button class="btn small" disabled={busy} onclick={() => runNow(s.id)}>Run now</button>
              <button class="btn small" onclick={() => toggleSchedule(s)}>{s.enabled ? 'Pause' : 'Enable'}</button>
              <button class="btn small" onclick={() => openSchedEdit(s)}>Edit</button>
              <button class="btn small danger" onclick={() => deleteSchedule(s)}>Delete</button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  {:else if tab === 'runs'}
    {#if runs.length === 0}
      <div class="muted">No runs yet. Run now fires the first one.</div>
    {:else}
      <ul class="rows">
        {#each runs as r (r.id)}
          <li class="run">
            <span class={statusClass(r.status)}>{r.status}</span>
            <span class="run-when">{r.started_at}</span>
            <span class="pill" title="which schedule fired">{scheduleName(r)}</span>
            {#if duration(r)}<span class="meta">{duration(r)}</span>{/if}
            <span class="run-sum">{r.summary || r.error || '(no summary)'}</span>
            {#if r.report_rel}
              <button class="btn small" onclick={() => viewReport(r)}>View report</button>
            {/if}
            {#if r.session_id}
              <button class="btn small" title="Open the agent session this run drove" onclick={() => ws.navigateToSession(r.session_id ?? '')}>Open session</button>
            {/if}
            {#if (r.attempts ?? 1) > 1}<span class="pill warn">{r.attempts} attempts</span>{/if}
            {#if r.delivered}<span class="pill ok">delivered</span>{/if}
            {#if r.skipped_delivery}<span class="pill" title="report unchanged since last run">no change</span>{/if}
            {#if r.delivery_error}<span class="pill warn" title={r.delivery_error}>delivery failed</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  {:else if tab === 'chat'}
    {#if chatError}
      <div class="err" role="alert">{chatError}</div>
      <button class="btn small" onclick={() => { chatError = ''; chatSessionId = ''; }}>Retry</button>
    {:else if chatSessionId}
      <div class="chatwrap">
        <Terminal sessionId={chatSessionId} autoFocus />
      </div>
    {:else}
      <div class="muted">Opening the agent's chat session…</div>
    {/if}
  {:else if tab === 'memory'}
    <section class="card">
      <h2>Memory</h2>
      <p class="hint">
        This agent keeps its notes in <code class="mono">memory/notes.md</code> under its workspace
        folder; every run reads and updates them. There is no HTTP viewer for these files yet —
        open the folder on this machine:
      </p>
      <pre class="soul mono">{memoryRoot}memory/notes.md</pre>
    </section>
  {/if}

  {#if editing && agent}
    <AgentEditSheet {agent} onclose={() => (editing = false)} />
  {/if}

  {#if reportOpen}
    <div
      class="modal-bg"
      onclick={(e) => { if (e.target === e.currentTarget) reportOpen = false; }}
      onkeydown={(e) => { if (e.key === 'Escape') reportOpen = false; }}
      role="presentation"
    >
      <div class="modal" role="dialog" aria-label="Report" aria-modal="true" tabindex="-1">
        <header class="modal-head">
          <strong>Report</strong>
          <button class="btn small" onclick={() => (reportOpen = false)}>Close</button>
        </header>
        {#if reportLoading}
          <div class="muted">Loading…</div>
        {:else}
          <pre class="report">{reportText}</pre>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .page { padding: 1rem 1.25rem; max-width: 980px; margin: 0 auto; }
  .head { display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap; margin-bottom: 0.5rem; }
  .head h1 { margin: 0; font-size: 1.2rem; color: var(--text); }
  .avatar {
    width: 2rem; height: 2rem; border-radius: var(--radius-m); display: inline-flex;
    align-items: center; justify-content: center; font-size: 1.1rem;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .grow { flex: 1; }
  .chip { font-size: 0.75rem; padding: 0.1rem 0.5rem; border-radius: 999px; border: 1px solid var(--border); color: var(--text-dim); }
  .mono { font-family: var(--font-mono); }
  .wrap { word-break: break-all; }
  .tabs { display: flex; gap: 0.25rem; border-bottom: 1px solid var(--border); margin-bottom: 0.75rem; overflow-x: auto; }
  .tab {
    background: none; border: none; border-bottom: 2px solid transparent; color: var(--text-dim);
    padding: 0.45rem 0.7rem; font: inherit; font-size: 0.88rem; cursor: pointer;
  }
  .tab.active { color: var(--text); border-bottom-color: var(--accent); }
  .tab:focus-visible { outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent); outline-offset: -2px; }
  .err {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    color: var(--status-exited); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); margin-bottom: 0.75rem; font-size: 0.85rem;
  }
  .muted, .hint { color: var(--text-dim); font-size: 0.88rem; }
  .hint { margin: 0; }
  .overview { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 0.75rem; }
  .card { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 0.75rem 0.9rem; color: var(--text); }
  .card h2 { margin: 0 0 0.5rem; font-size: 0.95rem; color: var(--text); }
  .soul { white-space: pre-wrap; word-break: break-word; font-size: 0.85rem; line-height: 1.5; margin: 0; color: var(--text); }
  .props { display: grid; grid-template-columns: auto 1fr; gap: 0.3rem 0.8rem; margin: 0; font-size: 0.85rem; }
  .props dt { color: var(--text-dim); }
  .props dd { margin: 0; color: var(--text); }
  .section-head { display: flex; align-items: center; justify-content: space-between; gap: 0.75rem; margin-bottom: 0.6rem; flex-wrap: wrap; }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .rowline {
    border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m);
    padding: 0.6rem 0.75rem; display: flex; justify-content: space-between; gap: 0.75rem; flex-wrap: wrap;
  }
  .rowmain { min-width: 16ch; flex: 1; color: var(--text); font-size: 0.88rem; }
  .rowmain .meta { margin-inline-start: 0.5rem; }
  .meta { color: var(--text-dim); font-size: 0.78rem; }
  .directive { margin: 0.3rem 0 0; color: var(--text-dim); font-size: 0.82rem; white-space: pre-wrap; word-break: break-word; }
  .rowactions { display: flex; gap: 0.35rem; flex-wrap: wrap; align-items: flex-start; }
  .run {
    display: flex; align-items: center; gap: 0.5rem; font-size: 0.82rem; flex-wrap: wrap; color: var(--text);
    border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 0.5rem 0.7rem;
  }
  .run-when { color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .run-sum { flex: 1; min-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pill { font-size: 0.7rem; padding: 0.05rem 0.45rem; border-radius: 999px; border: 1px solid var(--border); color: var(--text-dim); }
  .pill.ok { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent); border-color: transparent; }
  .pill.bad { background: color-mix(in srgb, var(--status-exited) 16%, transparent); color: var(--status-exited); border-color: transparent; }
  .pill.warn { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); border-color: transparent; }
  .pill.working { background: color-mix(in srgb, var(--status-working) 16%, transparent); color: var(--status-working); border-color: transparent; }
  .form { display: flex; flex-direction: column; gap: 0.75rem; margin-bottom: 0.75rem; }
  .row { display: flex; gap: 0.75rem; flex-wrap: wrap; }
  .row .fld { flex: 1; min-width: 160px; }
  .fld { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; color: var(--text); }
  .fld span { color: var(--text-dim); }
  .fld input, .fld select, .fld textarea {
    background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 0.45rem 0.55rem; font: inherit;
  }
  .fld input:focus-visible, .fld select:focus-visible, .fld textarea:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent); outline-offset: 1px;
  }
  .chk { display: flex; align-items: center; gap: 0.4rem; font-size: 0.85rem; color: var(--text); }
  .actions { display: flex; gap: 0.5rem; }
  .chatwrap { height: min(64vh, 620px); border: 1px solid var(--border); border-radius: var(--radius-m); overflow: hidden; }
  .modal-bg { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.45); display: flex; align-items: center; justify-content: center; z-index: 50; }
  .modal { background: var(--surface); border: 1px solid var(--border); color: var(--text); border-radius: var(--radius-l); width: min(760px, 92vw); max-height: 82vh; overflow: auto; padding: 0.85rem 1rem; box-shadow: var(--shadow); }
  .modal-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem; }
  .report { white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono); font-size: 0.8rem; line-height: 1.45; color: var(--text); }
</style>
