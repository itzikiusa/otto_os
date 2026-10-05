<script lang="ts">
  // One personal agent: Overview / Activity / Autonomy / Schedules / Runs /
  // Chat / Memory / Context tabs.
  import RelTime from '../../lib/components/RelTime.svelte';
  import { tabKeys } from '../../lib/tabKeys';
  import { tick } from 'svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { authedText } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { runStatus } from '../../lib/status';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import AgentAvatar from './AgentAvatar.svelte';
  import AgentEditSheet from './AgentEditSheet.svelte';
  import AgentDocuments from './AgentDocuments.svelte';
  import AgentAutonomy from './AgentAutonomy.svelte';
  import AgentActivity from './AgentActivity.svelte';
  import AgentMemoryInspector from './AgentMemoryInspector.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import {
    browserTz,
    buildCadence,
    cadenceLabel,
    defaultCadence,
    loadCadence,
    WEEKDAYS,
    type CadenceForm,
  } from './cadence';
  import type {
    PersonalAgentAutonomy,
    PersonalAgentPermission,
    PersonalAgentRun,
    PersonalAgentSchedule,
  } from '../../lib/api/types';

  interface Props {
    agentId: string;
  }
  let { agentId }: Props = $props();
  /** Prefix for the schedule form's label↔control ids. */
  const uid = $props.id();

  type Tab = 'overview' | 'activity' | 'autonomy' | 'schedules' | 'runs' | 'chat' | 'memory' | 'context';
  const TABS: { id: Tab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'activity', label: 'Activity' },
    { id: 'autonomy', label: 'Autonomy' },
    { id: 'schedules', label: 'Schedules' },
    { id: 'runs', label: 'Runs' },
    { id: 'chat', label: 'Chat' },
    { id: 'memory', label: 'Memory' },
    { id: 'context', label: 'Context' },
  ];
  // The tab is part of the URL (`#/personal-agents/<id>/<tab>`), so Run now can
  // land on Runs and a reload / back keeps the section.
  const tab = $derived<Tab>(TABS.some((t) => t.id === router.parts[2]) ? (router.parts[2] as Tab) : 'overview');
  let tabTransition = 0;
  async function goTab(t: Tab, event?: MouseEvent): Promise<void> {
    const list = (event?.currentTarget as HTMLElement | undefined)?.closest('[role="tablist"]');
    const seq = ++tabTransition;
    const accepted = await router.goChecked(t === 'overview' ? `personal-agents/${agentId}` : `personal-agents/${agentId}/${t}`);
    await tick();
    await Promise.resolve();
    if (seq !== tabTransition || !list?.isConnected || list.closest('[inert]')) return;
    // hashchange may follow this microtask; focus the accepted target only.
    list.querySelector<HTMLButtonElement>(`[role="tab"][data-tab="${accepted ? t : tab}"]`)?.focus();
  }
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

  // Autonomy summary for the header badges (primary / proactive). Best-effort:
  // the Autonomy tab shows its own load state.
  let autonomy = $state<PersonalAgentAutonomy | null>(null);
  $effect(() => {
    const id = agentId;
    autonomy = null;
    personalAgentsApi
      .autonomy(id)
      .then((a) => { if (id === agentId) autonomy = a; })
      .catch(() => {});
  });

  // Reset: memories + conversations + schedules + run history (confirmed by
  // typing the agent's name — the server checks it too).
  let memoryReload = $state(0);
  async function resetAgent(): Promise<void> {
    if (!agent) return;
    const name = agent.name;
    const typed = await confirmer.promptText(
      `Reset “${name}”? It forgets everything it learned (memory), its chat is closed, and its schedules and run history are deleted. Its persona, rules, goals and model stay. Type the agent’s name to confirm.`,
      { title: 'Reset agent', confirmLabel: 'Reset agent', placeholder: name, danger: true },
    );
    if (typed === null) return;
    if (typed.trim() !== name.trim()) {
      toasts.error('Not reset', 'The name didn’t match.');
      return;
    }
    try {
      await personalAgentsApi.reset(agentId, typed);
      chatSessionId = '';
      memoryReload += 1;
      void personalAgents.loadSchedules(agentId);
      void personalAgents.loadRuns(agentId);
      toasts.success(`Reset ${name}`);
    } catch (e) {
      toasts.error(`Couldn’t reset ${name}`, loadErrorText(e));
    }
  }

  /** Stop a running run — asks first: what it did so far is not reported. */
  async function stopRun(r: PersonalAgentRun): Promise<void> {
    const ok = await confirmer.ask(
      'Stop this run? Its agent session is stopped and nothing is delivered. The agent’s schedules keep running.',
      { title: 'Stop run', confirmLabel: 'Stop run' },
    );
    if (!ok) return;
    try {
      await personalAgentsApi.cancelRun(r.id);
      toasts.info('Stopping the run…');
    } catch (e) {
      toasts.error('Couldn’t stop the run', loadErrorText(e));
    }
    void personalAgents.loadRuns(agentId);
  }

  // --- Chat: one interactive session pinned to the agent's provider/model/cwd.
  let chatSessionId = $state('');
  let chatError = $state('');
  // Bumped by Retry: clearing the (already empty) session id is no change, so
  // the effect never re-ran and Retry left "Opening…" up forever.
  let chatAttempt = $state(0);
  $effect(() => {
    void chatAttempt;
    if (tab !== 'chat' || chatSessionId) return;
    chatError = '';
    personalAgentsApi
      .chatSession(agentId)
      .then((r) => (chatSessionId = r.session_id))
      .catch((e) => (chatError = loadErrorText(e)));
  });

  // --- Schedules form -------------------------------------------------------
  let schedFormOpen = $state(false);
  let schedEditId = $state<string | null>(null);
  let sf = $state<CadenceForm>(defaultCadence());
  let sfTimezone = $state(browserTz());
  let sfDirective = $state('');
  let sfEnabled = $state(true);
  let sfPermission = $state<PersonalAgentPermission>('directed');

  function openSchedCreate(): void {
    schedEditId = null;
    sf = defaultCadence();
    sfTimezone = browserTz();
    sfDirective = '';
    sfEnabled = true;
    sfPermission = 'directed';
    schedFormOpen = true;
    error = '';
  }

  function openSchedEdit(s: PersonalAgentSchedule): void {
    schedEditId = s.id;
    sf = loadCadence(s.schedule, s.timezone || browserTz());
    sfTimezone = s.timezone || browserTz();
    sfDirective = s.directive;
    sfEnabled = s.enabled;
    sfPermission = s.permission ?? 'directed';
    schedFormOpen = true;
    error = '';
  }

  async function saveSchedule(): Promise<void> {
    if (sf.cadence === 'once' && !sf.runAt) {
      error = 'Pick the date and time it runs.';
      return;
    }
    busy = true;
    error = '';
    const body = {
      schedule: buildCadence(sf),
      timezone: sfTimezone.trim() || 'UTC',
      directive: sfDirective,
      enabled: sfEnabled,
      permission: sfPermission,
    };
    try {
      if (schedEditId) await personalAgents.updateSchedule(agentId, schedEditId, body);
      else await personalAgents.createSchedule(agentId, body);
      schedFormOpen = false;
    } catch (e) {
      // Stays inline next to the form: it's usually a field problem (cron, tz).
      error = `Couldn’t save the schedule. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }

  async function toggleSchedule(s: PersonalAgentSchedule): Promise<void> {
    try {
      await personalAgents.updateSchedule(agentId, s.id, { enabled: !s.enabled });
    } catch (e) {
      toasts.error(s.enabled ? 'Couldn’t pause the schedule' : 'Couldn’t enable the schedule', loadErrorText(e));
    }
  }

  async function deleteSchedule(s: PersonalAgentSchedule): Promise<void> {
    const what = cadenceLabel(s.schedule, s.timezone);
    if (!(await confirmer.ask(`Delete the schedule “${what}”? Past runs stay in Runs.`, { title: 'Delete schedule', confirmLabel: 'Delete' }))) return;
    try {
      await personalAgents.deleteSchedule(agentId, s.id);
    } catch (e) {
      toasts.error('Couldn’t delete the schedule', loadErrorText(e));
    }
  }

  async function runNow(scheduleId?: string): Promise<void> {
    busy = true;
    try {
      await personalAgents.runNow(agentId, scheduleId);
      goTab('runs');
    } catch (e) {
      toasts.error(`Couldn’t run ${agent?.name ?? 'the agent'}`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function toggleAgent(): Promise<void> {
    if (!agent) return;
    const was = agent.enabled;
    busy = true;
    try {
      await personalAgents.setEnabled(agentId, !was);
    } catch (e) {
      toasts.error(was ? `Couldn’t pause ${agent.name}` : `Couldn’t enable ${agent.name}`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function removeAgent(): Promise<void> {
    if (!agent) return;
    const name = agent.name;
    if (!(await confirmer.ask(`Delete personal agent “${name}”? Its schedules, memory and run history go with it.`, { title: 'Delete personal agent', confirmLabel: 'Delete' }))) return;
    try {
      await personalAgents.remove(agentId);
      toasts.success(`Deleted ${name}`);
      router.go('personal-agents');
    } catch (e) {
      toasts.error(`Couldn’t delete ${name}`, loadErrorText(e));
    }
  }

  // --- Runs / report viewer -------------------------------------------------
  let reportRun = $state<PersonalAgentRun | null>(null);
  let reportText = $state('');
  let reportError = $state('');
  let reportLoading = $state(false);

  async function viewReport(run: PersonalAgentRun): Promise<void> {
    reportRun = run;
    reportLoading = true;
    reportText = '';
    reportError = '';
    try {
      reportText = await authedText(personalAgentsApi.reportPath(run.id));
    } catch (e) {
      reportError = loadErrorText(e);
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

  function scheduleName(r: PersonalAgentRun): string {
    if (r.mode === 'proactive') {
      const g = autonomy?.goals.find((x) => x.id === r.goal_id);
      return g ? `Goal · ${g.text}` : 'Standing goal';
    }
    if (!r.schedule_id) return 'Manual';
    const s = schedules.find((x) => x.id === r.schedule_id);
    return s ? cadenceLabel(s.schedule, s.timezone) : 'Deleted schedule';
  }

  /** Where each run's report goes, in words (outward destinations named). */
  function deliveryLabel(): string {
    const d = agent?.delivery ?? {};
    const str = (k: string): string => (typeof d[k] === 'string' ? (d[k] as string) : '');
    switch (d.type) {
      case 'slack': return str('chat_id') ? `Slack · ${str('chat_id')}` : 'Slack · the integration’s channel';
      case 'telegram': return str('chat_id') ? `Telegram · ${str('chat_id')}` : 'Telegram · the integration’s chat';
      case 'email': return `Email · ${str('to') || 'no address set'}`;
      case 'webhook': return `Webhook · ${str('url') || 'no URL set'}`;
      default: return 'None — reports stay on this page';
    }
  }
  const runsLoaded = $derived(personalAgents.runsByAgent[agentId] !== undefined);
  /** A failed runs load is shown inline with Retry — never as "No runs yet". */
  const runsErr = $derived(personalAgents.runsError[agentId] ?? null);

  const canEditDocuments = $derived(auth.can('scheduled_tasks', 'edit') &&
    (auth.isRoot || ['editor', 'admin'].includes(ws.workspaces.find((w) => w.id === agent?.workspace_id)?.my_role ?? 'viewer')));
</script>

<div class="agent-page">
<PageHeader title={agent ? agent.name : 'Personal agent'} tabsPlacement="below">
  {#snippet leading()}
    <button class="icon-btn" title="Back to Personal Agents" aria-label="Back to Personal Agents" onclick={() => router.go('personal-agents')}>
      <Icon name="chevronLeft" size={16} />
    </button>
    {#if agent}<AgentAvatar avatar={agent.avatar} name={agent.name} size={24} />{/if}
  {/snippet}
  {#snippet badge()}
    {#if agent}
      <span class="chip prov" title={agent.model ? `${agent.provider} · ${agent.model}` : agent.provider}>
        <ProviderIcon provider={agent.provider} size={12} />{agent.provider}{agent.model ? ` · ${agent.model}` : ''}
      </span>
      {#if !agent.enabled}<span class="chip" title="Schedules don’t fire while paused. Run now and chat still work.">Paused</span>{/if}
      {#if agent.browser}<span class="chip" title="Runs and chat can use the Otto browser tool">Browser</span>{/if}
      {#if autonomy?.primary}<span class="chip pa-accent" title="Your primary assistant — routes specialist work to your other agents"><Icon name="star" size={12} /> Your agent</span>{/if}
      {#if autonomy?.proactive.enabled}<span class="chip" title="Works its standing goals in the background, read-only"><Icon name="eye" size={12} /> Proactive</span>{/if}
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if agent}
      {#if agent.enabled}
        <button class="btn small primary" data-icon="play" disabled={busy} onclick={() => runNow()}><Icon name="play" size={12} /> Run now</button>
        <button class="btn small" data-icon="clock" disabled={busy} onclick={toggleAgent} title="Stop its schedules from firing">Pause</button>
      {:else}
        <button class="btn small primary" data-icon="check" disabled={busy} onclick={toggleAgent}><Icon name="check" size={12} /> Enable</button>
        <button class="btn small" data-icon="play" disabled={busy} onclick={() => runNow()} title="Run it once without enabling its schedules">Run once</button>
      {/if}
      <button class="btn small" data-icon="edit" onclick={() => (editing = true)}><Icon name="edit" size={12} /> Edit</button>
      <button class="icon-btn" data-overflow="-1" data-icon="undo" data-label="Reset agent…" onclick={resetAgent} aria-label="Reset agent" title="Reset agent — forget memory, chat, schedules and runs">
        <Icon name="undo" size={14} />
      </button>
      <button class="icon-btn" data-overflow="-2" data-icon="trash" data-label="Delete agent…" onclick={removeAgent} aria-label="Delete agent" title="Delete agent">
        <Icon name="trash" size={14} />
      </button>
    {/if}
  {/snippet}
  {#snippet tabs()}
    <div class="tabs" role="tablist" aria-label="Agent sections" tabindex="-1" onkeydown={tabKeys({ activate: 'click' })}>
      {#each TABS as t (t.id)}
        <button
          class="tab"
          class:active={tab === t.id}
          role="tab"
          data-tab={t.id}
          aria-selected={tab === t.id}
          tabindex={tab === t.id ? 0 : -1}
          onclick={(event) => void goTab(t.id, event)}
        >{t.label}</button>
      {/each}
    </div>
  {/snippet}
</PageHeader>
<PageBody width="readable">
<div class="agent-body">
  {#if !agent}
    {#if personalAgents.loadingAgents}
      <div aria-busy="true" aria-label="Loading the agent"><Skeleton rows={3} height={64} /></div>
    {:else}
      <EmptyState
        variant="page"
        icon="user"
        title="Agent not found"
        body="It may have been deleted, or it belongs to another workspace."
        actionLabel="Back to Personal Agents"
        onaction={() => router.go('personal-agents')}
      />
    {/if}
  {:else if tab === 'overview'}
    <div class="overview">
      <section class="pa-panel">
        <div class="card-head">
          <h2>Persona</h2>
          <button class="btn small ghost" onclick={() => (editing = true)}>Edit</button>
        </div>
        {#if agent.soul_md.trim()}
          <pre class="soul">{agent.soul_md}</pre>
        {:else}
          <p class="muted">No persona yet. Edit the agent to say who it is and how it works.</p>
        {/if}
      </section>
      <section class="pa-panel">
        <h2>Configuration</h2>
        <dl class="props">
          <dt>Provider</dt><dd class="prov-dd"><ProviderIcon provider={agent.provider} size={12} /><span class="mono">{agent.provider}</span></dd>
          <dt>Model</dt><dd class:mono={!!agent.model}>{agent.model || 'Provider default'}</dd>
          <dt>Delivery</dt><dd class="clip" title={deliveryLabel()}>{deliveryLabel()}</dd>
          <dt>Browser use</dt><dd>{agent.browser ? 'On' : 'Off'}</dd>
          <dt>Status</dt><dd>{agent.enabled ? 'Enabled' : 'Paused — schedules don’t fire'}</dd>
          <dt>Working folder</dt>
          {#if agent.cwd}
            <dd class="mono path" dir="ltr" title={agent.cwd}>{agent.cwd}</dd>
          {:else}
            <!-- The default is a private per-agent folder; its ULID path is in
                 the tooltip, not the primary text. -->
            <dd title={`…/personal/${agent.id}/`}>Its own private folder (default)</dd>
          {/if}
          <dt>Schedules</dt>
          <dd>
            {schedules.length === 0 ? 'None' : schedules.length}
            {#if schedules.length > 0}
              · {#if agent.enabled}next run <RelTime iso={personalAgents.nextRunAt(agent.id)} fallback="not scheduled" />{:else}paused{/if}
            {/if}
            <button class="link" onclick={() => goTab('schedules')}>Manage</button>
          </dd>
        </dl>
      </section>
    </div>
  {:else if tab === 'activity'}
    {#key agentId}<AgentActivity {agentId} />{/key}
  {:else if tab === 'autonomy'}
    {#key agentId}<AgentAutonomy {agentId} editable={canEditDocuments} onsaved={(a) => (autonomy = a)} />{/key}
  {:else if tab === 'schedules'}
    {#if schedules.length > 0 || schedFormOpen}
      <div class="section-head">
        <p class="hint">Each schedule has its own cadence, directive and run cursor.{#if !agent.enabled}{' They don’t fire while the agent is paused.'}{/if}</p>
        {#if !schedFormOpen}
          <button class="btn small" onclick={openSchedCreate}><Icon name="plus" size={12} /> Add schedule</button>
        {/if}
      </div>
    {/if}
    {#if schedFormOpen}
      <div class="form pa-panel">
        <h2>{schedEditId ? 'Edit schedule' : 'New schedule'}</h2>
        {#if error}<div class="err" role="alert">{error}</div>{/if}
        <div class="field-row">
          <div class="field">
            <label for="{uid}-cadence">Cadence</label>
            <select id="{uid}-cadence" class="input" bind:value={sf.cadence}>
              <option value="interval">Interval</option>
              <option value="daily">Daily</option>
              <option value="weekly">Weekly</option>
              <option value="cron">Cron</option>
              <option value="once">Once</option>
            </select>
          </div>
          {#if sf.cadence === 'interval'}
            <div class="field">
              <label for="{uid}-every">Every (minutes, min 5)</label>
              <input id="{uid}-every" class="input" type="number" min="5" bind:value={sf.everyMin} />
            </div>
          {:else if sf.cadence === 'once'}
            <div class="field">
              <label for="{uid}-run-at">Runs once at (in the timezone)</label>
              <input id="{uid}-run-at" class="input" type="datetime-local" bind:value={sf.runAt} />
            </div>
          {:else if sf.cadence === 'cron'}
            <div class="field">
              <label for="{uid}-cron">Cron expression (5 fields)</label>
              <input id="{uid}-cron" class="input" bind:value={sf.cronExpr} placeholder="0 9 * * 1" dir="ltr" />
            </div>
          {:else}
            <div class="field">
              <label for="{uid}-at">At (HH:MM)</label>
              <input id="{uid}-at" class="input" bind:value={sf.at} placeholder="09:00" dir="ltr" />
            </div>
            {#if sf.cadence === 'weekly'}
              <div class="field">
                <label for="{uid}-weekday">Weekday</label>
                <select id="{uid}-weekday" class="input" bind:value={sf.weekday}>
                  {#each WEEKDAYS as d, i (d)}<option value={i}>{d}</option>{/each}
                </select>
              </div>
            {/if}
          {/if}
          {#if sf.cadence !== 'interval'}
            <div class="field">
              <label for="{uid}-tz">Timezone</label>
              <input id="{uid}-tz" class="input" bind:value={sfTimezone} placeholder="e.g. Europe/London" dir="ltr" />
            </div>
          {/if}
        </div>
        <div class="field">
          <label for="{uid}-directive">Directive (the run’s task prompt)</label>
          <textarea id="{uid}-directive" class="input" bind:value={sfDirective} rows="4" placeholder="Produce the daily recap…"></textarea>
        </div>
        <div class="field">
          <label for="{uid}-permission">Permissions for its runs</label>
          <select id="{uid}-permission" class="input" bind:value={sfPermission}>
            <option value="directed">Directed — your normal approvals and auto-approve rules</option>
            <option value="read_only">Read-only — can read and report, can’t change anything</option>
          </select>
        </div>
        <label class="chk"><input type="checkbox" bind:checked={sfEnabled} /> Enabled</label>
        <div class="actions">
          <button class="btn" disabled={busy} onclick={() => { schedFormOpen = false; error = ''; }}>Cancel</button>
          <button class="btn primary" disabled={busy} onclick={saveSchedule}>{busy ? 'Saving…' : schedEditId ? 'Save schedule' : 'Add schedule'}</button>
        </div>
      </div>
    {/if}
    {#if schedules.length === 0 && !schedFormOpen}
      <EmptyState
        icon="calendar"
        title="No schedules"
        body="This agent only runs when you press Run now, or when you chat with it. Add a schedule to run it on a cadence."
        actionLabel="Add schedule"
        actionIcon="plus"
        onaction={openSchedCreate}
      />
    {:else}
      <ul class="rows">
        {#each schedules as s (s.id)}
          <li class="rowline">
            <div class="rowmain">
              <div class="rowtitle">
                <strong>{cadenceLabel(s.schedule, s.timezone)}</strong>
                {#if !s.enabled}<span class="chip">Paused</span>{/if}
                {#if s.permission === 'read_only'}<span class="chip" title="Its runs can read and report, but can’t send, post, write or change anything"><Icon name="lock" size={12} /> Read-only</span>{/if}
              </div>
              <!-- a paused schedule (or paused agent) never fires: no "next" promise -->
              <span class="meta">{#if s.enabled && agent.enabled}Next <RelTime iso={s.next_run_at} /> · {/if}Last <RelTime iso={s.last_run_at} fallback="never" /></span>
              <p class="directive">{s.directive || 'No directive — runs on the persona alone.'}</p>
            </div>
            <div class="rowactions">
              <button class="btn small" disabled={busy} onclick={() => runNow(s.id)}>Run now</button>
              <button class="btn small" onclick={() => toggleSchedule(s)}>{s.enabled ? 'Pause' : 'Enable'}</button>
              <button class="btn small" onclick={() => openSchedEdit(s)}>Edit</button>
              <button class="icon-btn" onclick={() => deleteSchedule(s)} aria-label="Delete schedule {cadenceLabel(s.schedule, s.timezone)}" title="Delete schedule">
                <Icon name="trash" size={14} />
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  {:else if tab === 'runs'}
    <LoadState
      what="this agent’s runs"
      loading={!runsLoaded}
      error={runsErr}
      empty={runs.length === 0}
      rows={4}
      onretry={() => void personalAgents.loadRuns(agentId)}
    >
      {#snippet emptyView()}
        <EmptyState
          icon="play"
          title="No runs yet"
          body={agent.enabled
            ? 'Runs appear here with their report and delivery status — from a schedule firing, or from Run once above.'
            : 'Runs appear here with their report and delivery status. Enable the agent to let its schedules fire, or use Run once above.'}
        />
      {/snippet}
      <ul class="rows">
        {#each runs as r (r.id)}
          <li class="run">
            <StatusBadge status={runStatus(r.status)} variant="text" />
            <span class="run-when"><RelTime iso={r.started_at} /></span>
            <span class="chip clip-chip" title="What started this run">{scheduleName(r)}</span>
            {#if r.read_only}<span class="chip" title="This run could read and report, but not change anything"><Icon name="lock" size={12} /> Read-only</span>{/if}
            {#if duration(r)}<span class="meta">{duration(r)}</span>{/if}
            <span class="run-sum" title={r.summary || r.error || undefined}>{r.summary || r.error || 'No summary'}</span>
            {#if (r.attempts ?? 1) > 1}<span class="chip pa-warn">{r.attempts} attempts</span>{/if}
            {#if r.delivered}<span class="chip ok">Delivered</span>{/if}
            {#if r.skipped_delivery}<span class="chip" title="The report didn’t change since the last run, so nothing was sent">No change</span>{/if}
            {#if r.delivery_error}<span class="chip bad" title={r.delivery_error}>Delivery failed</span>{/if}
            {#if r.report_rel}
              <button class="btn small" onclick={() => viewReport(r)}>View report</button>
            {/if}
            {#if r.session_id}
              <button class="btn small" title="Open the agent session this run drove" onclick={() => ws.navigateToSession(r.session_id ?? '')}>Open session</button>
            {/if}
            {#if r.status === 'running'}
              <button class="btn small danger" title="Stop this run (its agent session is stopped)" onclick={() => void stopRun(r)}>Stop…</button>
            {/if}
            <!-- Readable without hovering: the error used to hide behind a summary. -->
            {#if r.status === 'error' && r.error && r.summary}
              <p class="run-err" role="note">{r.error}</p>
            {/if}
            {#if r.delivery_error}
              <p class="run-err warn" role="note">Not delivered: {r.delivery_error}</p>
            {/if}
          </li>
        {/each}
      </ul>
    </LoadState>
  {:else if tab === 'chat'}
    {#if chatError}
      <LoadState what="the chat session" error={chatError} empty onretry={() => { chatError = ''; chatSessionId = ''; chatAttempt += 1; }} />
    {:else if chatSessionId}
      <div class="chatwrap">
        <LazyTerminal sessionId={chatSessionId} autoFocus />
      </div>
    {:else}
      <p class="muted" role="status">Opening {agent.name}’s chat session…</p>
    {/if}
  {:else if (tab === 'memory' || tab === 'context') && agent}
    {#if tab === 'memory'}
      <AgentMemoryInspector {agentId} editable={canEditDocuments} reloadKey={memoryReload} />
    {/if}
    {#key `${agentId}:${tab}:${memoryReload}`}
      <AgentDocuments {agentId} workspaceId={agent.workspace_id} kind={tab} editable={canEditDocuments} sharedWorkspace={!!agent.cwd.trim()} />
    {/key}
  {/if}

  {#if editing && agent}
    <AgentEditSheet {agent} onclose={() => (editing = false)} />
  {/if}

  {#if reportRun}
    <!-- the shared sheet: Esc works from anywhere, focus is trapped + restored -->
    <Modal title="Report" width={760} onclose={() => (reportRun = null)}>
      {#if reportLoading}
        <div aria-busy="true" aria-label="Loading the report"><Skeleton rows={6} height={18} /></div>
      {:else if reportError}
        <LoadState what="the report" error={reportError} empty onretry={() => reportRun && viewReport(reportRun)} />
      {:else}
        <pre class="report">{reportText}</pre>
      {/if}
    </Modal>
  {/if}
</div>
</PageBody>
</div>

<style>
  .agent-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .prov { max-width: 260px; }
  .pa-accent { color: var(--accent-text); border-color: var(--accent-line); display: inline-flex; align-items: center; gap: 4px; }
  .clip-chip { max-width: 28ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .prov :global(svg) { flex-shrink: 0; }
  .mono { font-family: var(--font-mono); }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tabs { display: flex; gap: 4px; overflow-x: auto; padding-inline: 2px; }
  .tab {
    background: none; border: none; border-bottom: 2px solid transparent; color: var(--text-dim);
    padding: 8px 10px; font: inherit; font-size: var(--fs-m); cursor: pointer; white-space: nowrap;
  }
  .tab:hover { color: var(--text); }
  .tab.active { color: var(--text); border-bottom-color: var(--accent); font-weight: 500; }
  .tab:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: var(--radius-s); }
  .err {
    background: var(--danger-soft); color: var(--danger); padding: 8px 12px;
    border-radius: var(--radius-s); font-size: var(--fs-s);
  }
  .muted, .hint { color: var(--text-dim); font-size: var(--fs-m); }
  .hint { margin: 0; }
  .overview { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; }
  .pa-panel { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 12px 14px; color: var(--text); min-width: 0; }
  .pa-panel h2 { margin: 0 0 8px; font-size: var(--fs-l); font-weight: 600; color: var(--text); }
  .card-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .card-head h2 { margin: 0 0 8px; }
  .soul { white-space: pre-wrap; word-break: break-word; font-family: inherit; font-size: var(--fs-m); line-height: 1.5; margin: 0; color: var(--text); }
  .props { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 12px; margin: 0; font-size: var(--fs-m); }
  .props dt { color: var(--text-dim); }
  .props dd { margin: 0; color: var(--text); min-width: 0; }
  .prov-dd { display: flex; align-items: center; gap: 6px; }
  .clip { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link { border: 0; background: none; padding: 0; margin-inline-start: 6px; font: inherit; color: var(--accent-text); cursor: pointer; }
  .link:hover { text-decoration: underline; }
  .section-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 10px; flex-wrap: wrap; }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .rowline {
    border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m);
    padding: 10px 12px; display: flex; justify-content: space-between; gap: 12px; flex-wrap: wrap;
  }
  .rowmain { min-width: 16ch; flex: 1; color: var(--text); font-size: var(--fs-m); display: flex; flex-direction: column; gap: 2px; }
  .rowtitle { display: flex; align-items: center; gap: 8px; }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .directive { margin: 4px 0 0; color: var(--text-dim); font-size: var(--fs-s); white-space: pre-wrap; word-break: break-word; }
  .rowactions { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
  .run {
    display: flex; align-items: center; gap: 8px; font-size: var(--fs-s); flex-wrap: wrap; color: var(--text);
    border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 8px 12px;
  }
  .run-when { color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .run-sum { flex: 1; min-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .run-err { flex-basis: 100%; margin: 0; font-size: var(--fs-xs); color: var(--danger); overflow-wrap: anywhere; }
  .run-err.warn { color: var(--warning); }
  .pa-warn { color: var(--warning); border-color: color-mix(in srgb, var(--warning) 35%, transparent); }
  .form { display: flex; flex-direction: column; gap: 12px; margin-bottom: 12px; }
  .form h2 { margin: 0; }
  /* Global .field/.input; the form's own gap spaces them, rows share a line. */
  .form .field { margin-bottom: 0; }
  .field-row { display: flex; gap: 12px; flex-wrap: wrap; }
  .field-row > .field { flex: 1; min-width: 160px; }
  .chk { display: flex; align-items: center; gap: 6px; font-size: var(--fs-m); color: var(--text); }
  .actions { display: flex; gap: 8px; justify-content: flex-end; }
  .chatwrap { height: min(64vh, 620px); border: 1px solid var(--border); border-radius: var(--radius-m); overflow: hidden; }
  .report { white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono); font-size: var(--fs-s); line-height: 1.45; color: var(--text); margin: 0; }
</style>
