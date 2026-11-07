<script lang="ts">
  // One personal agent's settings: About · Activity · Memory. Its chat and
  // routines live on the home (`#/personal-agents/<id>`); older section slugs
  // (overview, runs, routines, …) land on the section holding them.
  import Tabs from '../../lib/components/Tabs.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { tabKeys } from '../../lib/tabKeys';
  import { tick, untrack } from 'svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { confirmRunAndDeliver } from '../../lib/confirmOutward';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
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
  import AgentAutonomy from './AgentAutonomy.svelte';
  import AgentActivity from './AgentActivity.svelte';
  import { openAgentSession } from '../../lib/openAgentSession';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { formatModelLabel, modelsForProvider } from '../../lib/providerModels';
  import { harnessLabel } from '../../lib/providers';
  import { routineWhen } from './cadence';
  import type { PersonalAgentAutonomy, PersonalAgentRun } from '../../lib/api/types';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { currentModels, firstLine } from './agentHome';
  import { leadLine } from '../../lib/plainText';
  import RunReport from '../../lib/components/RunReport.svelte';
  import { paAgentPort, paTab, type PaTab } from '../../lib/uiCommands/personalAgentsPort';
  import AgentPersonality from './AgentPersonality.svelte';
  import { confirmDeleteAgent } from './confirmDelete';
  import { routineLabel, runBadge } from '../../lib/personalAgentBadges';

  const AgentMemoryInspectorLazy = lazyComponent(() => import('./AgentMemoryInspector.svelte'));
  const AgentDocumentsLazy = lazyComponent(() => import('./AgentDocuments.svelte'));

  interface Props {
    agentId: string;
    embedded?: boolean;
  }
  let { agentId, embedded = false }: Props = $props();

  type Tab = PaTab;
  const TABS: { id: Tab; label: string }[] = [
    { id: 'about', label: 'About' },
    { id: 'activity', label: 'Runs' },
    { id: 'memory', label: 'Memory' },
  ];
  // The tab is part of the URL (`#/personal-agents/<id>/<tab>`), so Run now can
  // land on History and a reload / back keeps the section.
  const tab = $derived<Tab>(paTab(router.parts[2]));
  let tabTransition = 0;
  let sheetTools = $state<HTMLDivElement>();
  const tabBase = $props.id();
  async function goTab(t: Tab, event?: MouseEvent): Promise<void> {
    const list = (event?.currentTarget as HTMLElement | undefined)?.closest('[role="tablist"]') ?? sheetTools?.querySelector('[role="tablist"]');
    const seq = ++tabTransition;
    // `#/personal-agents/<id>` alone is the agent's home (its conversation);
    // every tab of this settings page has its own path, Overview included.
    const accepted = await router.goChecked(`personal-agents/${agentId}/${t}`);
    await tick();
    await Promise.resolve();
    if (seq !== tabTransition || !list?.isConnected || list.closest('[inert]')) return;
    // hashchange may follow this microtask; focus the accepted target only.
    const target = accepted ? t : tab;
    list.querySelector<HTMLButtonElement>(`[role="tab"][data-tab="${target}"], [id="${tabBase}-tab-${target}"]`)?.focus();
  }
  /** The edit sheet's open section (null = closed). */
  let editing = $state<null | 'about' | 'setup'>(null);
  let soulOpen = $state(false);
  let advancedOpen = $state(false);
  let activityOpen = $state(false);
  let busy = $state(false);

  const agent = $derived(personalAgents.agent(agentId));
  const schedules = $derived(personalAgents.schedulesByAgent[agentId] ?? []);
  const runs = $derived(personalAgents.runsByAgent[agentId] ?? []);

  // The agent may not be cached yet (deep link) — the list load fills it in.
  // Once per (workspace, agent): the lookup is untracked, so a list that comes
  // back WITHOUT the id ("Agent not found") doesn't re-run this and refetch
  // in a loop (ui-performance-2-02).
  let lookedUp = '';
  $effect(() => {
    const wsId = ws.currentId;
    const key = `${wsId}|${agentId}`;
    if (!wsId || key === lookedUp) return;
    if (untrack(() => personalAgents.agent(agentId))) return;
    lookedUp = key;
    // A failed lookup is not "not found": forget it, so a later visit asks again.
    void retryAgents(wsId).then(() => { if (personalAgents.agentsError && lookedUp === key) lookedUp = ''; });
  });
  const retryAgents = (wsId: string | null) => (wsId ? personalAgents.loadAgents(wsId) : Promise.resolve());

  $effect(() => {
    if (tab === 'activity') void personalAgents.loadRuns(agentId);
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
      `Reset “${name}”? It forgets everything it learned (memory), its chat is closed, and its routines and run history are deleted. Its persona, rules, goals and model stay. Type the agent’s name to confirm.`,
      { title: 'Reset agent', confirmLabel: 'Reset agent', placeholder: name, danger: true },
    );
    if (typed === null) return;
    if (typed.trim() !== name.trim()) {
      toasts.error('Not reset', 'The name didn’t match.');
      return;
    }
    try {
      await personalAgentsApi.reset(agentId, typed);
      memoryReload += 1;
      void personalAgents.loadSchedules(agentId);
      void personalAgents.loadRuns(agentId);
      toasts.success(`Reset ${name}`);
    } catch (e) {
      toasts.error(`Couldn’t reset ${name}`, loadErrorText(e));
    }
  }

  /** Stop a running run — asks first, naming the schedule or the start time. */
  async function stopRun(r: PersonalAgentRun): Promise<void> {
    const what = scheduleName(r);
    const when = new Date(r.started_at).toLocaleString();
    const ok = await confirmer.ask(
      `Stop the ${what} run that started ${when}? Its agent session is stopped and nothing is delivered. Its routines keep running.`,
      { title: `Stop ${what}?`, danger: true, confirmLabel: 'Stop run' },
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

  async function runNow(scheduleId?: string): Promise<void> {
    // Outward-facing: the run delivers its report the moment it finishes.
    if (agent && !(await confirmRunAndDeliver(agent.name, agent.delivery))) return;
    busy = true;
    try {
      await personalAgents.runNow(agentId, scheduleId);
      goTab('activity');
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
    if (!(await confirmDeleteAgent(agent, !!autonomy?.primary))) return;
    try {
      await personalAgents.remove(agentId);
      toasts.success(`Deleted ${name}`);
      router.go('personal-agents');
    } catch (e) {
      toasts.error(`Couldn’t delete ${name}`, loadErrorText(e));
    }
  }

  // Agent UI control (lib/uiCommands/personalAgents.ts): re-read Autonomy
  // after a save made outside the tab, open a run's report.
  let autonomyReload = $state(0);
  $effect(() => paAgentPort.bind({ agentId, reloadAutonomy: () => {
    autonomyReload += 1;
    const id = agentId;
    personalAgentsApi.autonomy(id).then((a) => { if (id === agentId) autonomy = a; }).catch(() => {});
  }, viewReport: (r) => void viewReport(r) }));

  // --- Runs / report viewer -------------------------------------------------
  let reportRun = $state<PersonalAgentRun | null>(null);
  function viewReport(run: PersonalAgentRun): void {
    reportRun = run;
  }

  function duration(r: PersonalAgentRun): string {
    if (!r.finished_at) return '…';
    const ms = Date.parse(r.finished_at) - Date.parse(r.started_at);
    if (!Number.isFinite(ms) || ms < 0) return '';
    if (ms < 60_000) return `took ${Math.round(ms / 1000)}s`;
    return `took ${Math.round(ms / 60_000)}m`;
  }

  /** The run's routine name ('' for a run without one). */
  function routineOf(r: PersonalAgentRun): string {
    const s = r.schedule_id ? schedules.find((x) => x.id === r.schedule_id) : undefined;
    return s ? routineLabel(s.directive) : '';
  }

  function scheduleName(r: PersonalAgentRun): string {
    if (r.mode === 'proactive') {
      const g = autonomy?.goals.find((x) => x.id === r.goal_id);
      return g ? `Goal · ${g.text}` : 'Standing goal';
    }
    const s = r.schedule_id ? schedules.find((x) => x.id === r.schedule_id) : undefined;
    if (r.schedule_id && !s) return 'Scheduled · deleted routine';
    return runBadge(r, s ? `${routineLabel(s.directive)} (${routineWhen(s)})` : null).label;
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
      default: return 'Its chat only';
    }
  }
  /** One model label: the pinned model, else the provider's default. */
  const modelChoices = $derived(agent ? currentModels(modelsForProvider(agent.provider, auth.meta), agent.model) : []);
  /** "What it does", edited in place: saved on blur (no Edit → dialog). */
  let soulDraft = $state('');
  let soulSave = $state<'' | 'saving' | 'saved'>('');
  let soulTimer: ReturnType<typeof setTimeout> | undefined;
  // Follows the stored text — but never over what is being typed.
  $effect(() => {
    const stored = agent?.soul_md ?? '';
    untrack(() => { if (document.activeElement?.id !== `pa-soul-${agentId}`) soulDraft = stored; });
  });
  async function saveSoul(): Promise<void> {
    if (!agent || soulDraft === agent.soul_md) return;
    soulSave = 'saving';
    clearTimeout(soulTimer);
    try {
      await personalAgents.update(agent.id, { soul_md: soulDraft });
      soulSave = 'saved';
      soulTimer = setTimeout(() => (soulSave = ''), 2500);
    } catch (e) {
      soulSave = '';
      toasts.error('Couldn’t save what it does', loadErrorText(e));
    }
  }
  /** The Setup card's calm save cue (the rules section's pattern). */
  let setupSave = $state<'' | 'saving' | 'saved'>('');
  let setupTimer: ReturnType<typeof setTimeout> | undefined;
  async function setModel(model: string): Promise<void> {
    if (!agent || model === '__other' || model === agent.model) return;
    setupSave = 'saving';
    clearTimeout(setupTimer);
    try {
      await personalAgents.update(agent.id, { model });
      setupSave = 'saved';
      setupTimer = setTimeout(() => (setupSave = ''), 2000);
    } catch (e) {
      setupSave = '';
      toasts.error('Couldn’t change the model', loadErrorText(e));
    }
  }
  const modelLabel = $derived(agent ? (agent.model ? formatModelLabel(agent.provider, agent.model, auth.meta) : `${harnessLabel(agent.provider).replace(/ Code$/, '')} · default model`) : '');
  const runsLoaded = $derived(personalAgents.runsByAgent[agentId] !== undefined);
  /** A failed runs load is shown inline with Retry — never as "No runs yet". */
  const runsErr = $derived(personalAgents.runsError[agentId] ?? null);

  const canEditDocuments = $derived(auth.can('scheduled_tasks', 'edit') &&
    (auth.isRoot || ['editor', 'admin'].includes(ws.workspaces.find((w) => w.id === agent?.workspace_id)?.my_role ?? 'viewer')));
  /** The agent runs AS its owner, so what it runs — memory and context
   *  documents, memory items, standing-goal texts and rules — is the owner's
   *  (or a ws admin's / root's) call; the server refuses everyone else 403
   *  (PC-3, access-control-4-04). An ownerless preset is adopted on the first
   *  write. Toggles, budget, primary and "Run now" stay `canEditDocuments`. */
  const myRole = $derived(ws.workspaces.find((w) => w.id === agent?.workspace_id)?.my_role ?? 'viewer');
  const canEditOwnerFields = $derived(canEditDocuments && (auth.isRoot || myRole === 'admin' ||
    !agent?.created_by || agent.created_by === auth.me?.id));
  const ownerNote = $derived(canEditDocuments && !canEditOwnerFields
    ? 'Runs as its owner; only the owner or a workspace admin can change this.'
    : '');

  function moreMenu(e: MouseEvent): void {
    if (!agent) return;
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: agent.enabled ? 'Pause' : 'Resume', icon: agent.enabled ? 'pause' : 'refresh', action: () => void toggleAgent() },
      { label: 'Reset agent…', icon: 'undo', action: () => void resetAgent() },
      { separator: true },
      { label: 'Delete agent…', icon: 'trash', danger: true, action: () => void removeAgent() },
    ]);
  }

  async function makeMain(): Promise<void> {
    try {
      autonomy = await personalAgentsApi.saveAutonomy(agentId, { primary: true });
      await Promise.all(personalAgents.agents.map((a) => personalAgents.loadAutonomy(a.id)));
      toasts.success(`${agent?.name ?? 'It'} is your main agent`, 'It routes your work to the other agents.');
    } catch (e) {
      toasts.error('Couldn’t make it the main agent', loadErrorText(e));
    }
  }
</script>

<div class="agent-page" class:embedded>
{#if !embedded}
<!-- Settings is a page of the agent inside Personal Agents, not a separate
     place: the trail leads back to the hub and to the agent's chat. -->
<PageHeader
  title={!agent ? 'Personal agent' : viewport.isPhone ? agent.name : (TABS.find((t) => t.id === tab)?.label ?? 'Settings')}
  crumbs={viewport.isPhone ? [] : [
    { label: 'Personal Agents', onclick: () => router.go('personal-agents') },
    ...(agent ? [{ label: agent.name, onclick: () => router.go(`personal-agents/${agentId}`) }] : []),
  ]}
  tabsPlacement="below"
>
  {#snippet badge()}
    {#if agent}
      {#if !agent.enabled}<span class="chip pa-warn" title="Routines don’t run while paused. Run now and chat still work.">Paused</span>{/if}
      {#if autonomy?.primary && !viewport.isPhone}<span class="chip pa-accent" title="Your main agent — creates agents, delegates to them, and they report back to it"><Icon name="star" size={12} /> Main agent</span>{/if}
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if agent}
      {#if !agent.enabled}
        <button class="btn small primary" data-icon="refresh" disabled={busy} aria-label="Resume {agent.name}" onclick={toggleAgent} title="Let its routines run again"><Icon name="refresh" size={12} /> Resume</button>
      {/if}
      <button class="btn small" data-keep aria-label="Back to chat with {agent.name}" title="Back to chat" onclick={() => router.go(`personal-agents/${agentId}`)}><Icon name="chevronLeft" size={12} />{#if !viewport.isPhone} Back to chat{/if}</button>
      <button class="btn small" data-icon="play" disabled={busy} aria-label="Run now {agent.name}" onclick={() => runNow()}><Icon name="play" size={12} /> Run now</button>
      <button class="icon-btn" data-keep aria-label="More for {agent.name}" title="Pause, reset or delete" onclick={moreMenu}><Icon name="more" size={14} /></button>
    {/if}
  {/snippet}
  {#snippet tabs()}
    <label class="tab-picker">
      <span class="tab-picker-label">Section</span>
      <select class="input" aria-label="Agent sections" value={tab} onchange={(e) => void goTab((e.currentTarget as HTMLSelectElement).value as Tab)}>
        {#each TABS as t (t.id)}<option value={t.id}>{t.label}</option>{/each}
      </select>
    </label>
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
{:else}
  <div class="sheet-tools" bind:this={sheetTools}>
    <Tabs tabs={TABS} value={tab} activate="click" idBase={tabBase} onchange={(id) => void goTab(id as Tab)} label="Agent sections" />
    <button type="button" class="icon-btn" aria-label="More for {agent?.name ?? 'agent'}" title="Pause, reset or delete" onclick={moreMenu}><Icon name="more" size={14} /></button>
  </div>
{/if}
<PageBody width="readable" padded={!embedded}>
<div class="agent-body" id={embedded ? `${tabBase}-panel-${tab}` : undefined} role={embedded ? 'tabpanel' : undefined} aria-labelledby={embedded ? `${tabBase}-tab-${tab}` : undefined}>
  {#if !agent}
    {#if personalAgents.loadingAgents}
      <Skeleton rows={3} height={64} label="the agent" />
    {:else if personalAgents.agentsError}
      <!-- A failed list load must never read as "Agent not found" (it may exist). -->
      <LoadState what="this agent" variant="page" error={personalAgents.agentsError} empty onretry={() => void retryAgents(ws.currentId)} />
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
  {:else if tab === 'about'}
    <p class="muted about-note">Changes save as you go.</p>
    <div class="about">
      <section class="pa-panel who" data-pa-persona>
        <div class="who-head">
          <AgentAvatar avatar={agent.avatar} name={agent.name} size={40} />
          <div class="who-t">
            <h2 dir="auto">{agent.name}</h2>
            <span class="muted">{agent.enabled ? 'Active' : 'Paused — its routines don’t run'}{autonomy?.primary ? ' · your main agent' : ''}</span>
          </div>
          {#if canEditDocuments}<button class="btn small" onclick={() => (editing = 'about')}><Icon name="edit" size={12} /> Name &amp; avatar</button>{/if}
        </div>
        <div class="setup-head">
          <h3><label for="pa-soul-{agent.id}">What it does <span class="muted">(its instructions)</span></label></h3>
          <span class="muted" aria-live="polite">{soulSave === 'saving' ? 'Saving…' : soulSave === 'saved' ? 'Saved' : ''}</span>
        </div>
        {#if canEditOwnerFields}
          <!-- Edited in place, saved when you leave the field — like every
               other setting on this page. -->
          <textarea id="pa-soul-{agent.id}" class="input soul-edit" rows={Math.min(14, Math.max(5, soulDraft.split('\n').length + 1))} dir="auto"
            placeholder="Who it is and how it works — or just tell {agent.name} in chat."
            bind:value={soulDraft} onblur={() => void saveSoul()}></textarea>
        {:else if agent.soul_md.trim()}
          <pre class="soul" class:clamped={!soulOpen}>{agent.soul_md}</pre>
          {#if agent.soul_md.split('\n').length > 6}<button class="link" onclick={() => (soulOpen = !soulOpen)}>{soulOpen ? 'Show less' : 'Show all'}</button>{/if}
        {:else}
          <p class="muted">None yet. Just tell {agent.name} in chat who it is and how it works.</p>
        {/if}
      </section>
      <AgentPersonality {agentId} name={agent.name} {autonomy} editable={canEditDocuments} ownerEditable={canEditOwnerFields} onsaved={(a) => (autonomy = a)} />
      <section class="pa-panel">
        <div class="setup-head">
          <h2>Setup</h2>
          <span class="muted" aria-live="polite">{setupSave === 'saving' ? 'Saving…' : setupSave === 'saved' ? 'Saved' : ''}</span>
          {#if canEditDocuments}<button class="btn small" onclick={() => (editing = 'setup')}><Icon name="edit" size={12} /> More setup…</button>{/if}
        </div>
        <dl class="props">
          <dt>Main agent</dt>
          <dd>
            {#if autonomy?.primary}
              <span class="pa-accent-text"><Icon name="star" size={12} /> Yes — it routes your work to the others</span>
            {:else}
              No{#if canEditDocuments}<button class="link" onclick={() => void makeMain()}>Make it the main agent</button>{/if}
            {/if}
          </dd>
          <dt>Model</dt>
          <dd class="prov-dd">
            <ProviderIcon provider={agent.provider} size={12} />
            {#if canEditOwnerFields && modelChoices.length > 0}
              <select class="input model-sel" aria-label="Model for {agent.name}" value={modelChoices.some((m) => m.id === agent.model) || !agent.model ? agent.model : '__other'} onchange={(e) => void setModel((e.currentTarget as HTMLSelectElement).value)}>
                <option value="">{harnessLabel(agent.provider).replace(/ Code$/, '')} · default model</option>
                {#each modelChoices as m (m.id)}<option value={m.id}>{m.label}</option>{/each}
                {#if agent.model && !modelChoices.some((m) => m.id === agent.model)}<option value="__other">{modelLabel}</option>{/if}
              </select>
            {:else}
              <span>{modelLabel}</span>
            {/if}
          </dd>
          <dt>Results go to</dt><dd class="clip" title={deliveryLabel()}>{deliveryLabel()}</dd>
          <dt>Browser</dt><dd>{agent.browser ? 'Can use the in-app browser' : 'Off'}</dd>
          <dt>Folder</dt>
          {#if agent.cwd}
            <dd class="mono path" dir="ltr" title={agent.cwd}>{agent.cwd}</dd>
          {:else}
            <dd>Its own private folder</dd>
          {/if}
          {#if agent.created_by_agent}<dt>Created by</dt><dd>{personalAgents.agent(agent.created_by_agent)?.name ?? 'your main agent'}</dd>{/if}
        </dl>
      </section>
    </div>
    <details class="pa-more" ontoggle={(e) => (advancedOpen = (e.currentTarget as HTMLDetailsElement).open)}>
      <summary>Rules, background work and Slack</summary>
      {#if advancedOpen}
        {#key `${agentId}:${autonomyReload}`}<AgentAutonomy {agentId} editable={canEditDocuments} ownerEditable={canEditOwnerFields} {ownerNote} onsaved={(a) => (autonomy = a)} />{/key}
      {/if}
    </details>
  {:else if tab === 'activity'}
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
            ? 'Each routine run and Run now shows here with its result.'
            : 'Each routine run and Run now shows here with its result. Resume the agent to let its routines run.'}
        />
      {/snippet}
      <ul class="rows">
        {#each runs as r (r.id)}
          {@const sum = (r.summary ? leadLine(r.summary) : firstLine(r.error ?? '')) || 'No summary'}
          <li class="run" data-run-id={r.id}>
            <StatusBadge status={runStatus(r.status)} variant="text" />
            <span class="run-when"><RelTime iso={r.started_at} /></span>
            <span class="chip clip-chip" title={scheduleName(r)}>{scheduleName(r)}</span>
            {#if r.read_only}<span class="chip" title="This run could read and report, but not change anything"><Icon name="lock" size={12} /> Read-only</span>{/if}
            {#if duration(r)}<span class="meta">{duration(r)}</span>{/if}
            <span class="run-sum" dir="auto" title={sum}>{sum}</span>
            {#if (r.attempts ?? 1) > 1}<span class="chip pa-warn">{r.attempts} attempts</span>{/if}
            {#if r.delivered}<span class="chip ok">Delivered</span>{/if}
            {#if r.skipped_delivery}<span class="chip" title="The result didn’t change since the last run, so nothing was sent">No change</span>{/if}
            {#if r.delivery_error}<span class="chip bad" title={r.delivery_error}>Delivery failed</span>{/if}
            {#if r.report_rel}
              <button class="btn small" onclick={() => viewReport(r)}>Result</button>
            {/if}
            {#if r.session_id}
              <button class="btn small" title="Open the thread this run worked in" onclick={() => void openAgentSession(r.session_id ?? '')}>Thread</button>
            {/if}
            {#if r.status === 'running'}
              <button class="btn small danger" title="Stop this run (its agent session is stopped)" aria-label="Stop {scheduleName(r)}" onclick={() => void stopRun(r)}>Stop…</button>
            {/if}
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
    <details class="pa-more" ontoggle={(e) => (activityOpen = (e.currentTarget as HTMLDetailsElement).open)}>
      <summary>Tool calls and approvals — across all of its runs</summary>
      {#if activityOpen}{#key agentId}<AgentActivity {agentId} />{/key}{/if}
    </details>
  {:else if tab === 'memory'}
    <div class="pa-stack">
    {#if AgentMemoryInspectorLazy.error}
      <LoadState what="this agent’s memory" error={AgentMemoryInspectorLazy.error} onretry={() => AgentMemoryInspectorLazy.retry()} />
    {:else if AgentMemoryInspectorLazy.component}
      <AgentMemoryInspectorLazy.component {agentId} editable={canEditOwnerFields} reloadKey={memoryReload} />
    {:else}
      <LoadState what="this agent’s memory" loading />
    {/if}
    {#key `${agentId}:context:${memoryReload}`}
      {#if AgentDocumentsLazy.error}
        <LoadState what="this agent’s background notes" error={AgentDocumentsLazy.error} onretry={() => AgentDocumentsLazy.retry()} />
      {:else if AgentDocumentsLazy.component}
        <AgentDocumentsLazy.component {agentId} workspaceId={agent.workspace_id} kind="context" editable={canEditOwnerFields} sharedWorkspace={!!agent.cwd.trim()} />
      {:else}
        <LoadState what="this agent’s background notes" loading />
      {/if}
    {/key}
    </div>
  {/if}

  {#if editing && agent}
    <AgentEditSheet {agent} section={editing} onclose={() => (editing = null)} />
  {/if}

  {#if reportRun}
    <RunReport run={reportRun} routine={routineOf(reportRun)} name={scheduleName(reportRun)} onclose={() => (reportRun = null)} />
  {/if}
</div>
</PageBody>
</div>

<style>
  .agent-page.embedded { height: auto; }
  .sheet-tools { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-block-end: 16px; }
  .agent-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .pa-accent-text { display: inline-flex; align-items: center; gap: 4px; color: var(--accent-text); }
  .pa-more { margin-top: 16px; }
  .pa-more > summary { cursor: pointer; font-size: var(--fs-m); font-weight: 600; color: var(--text); padding: 6px 0; }
  .pa-more > summary:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .pa-stack { display: flex; flex-direction: column; gap: 16px; }
  .pa-accent { color: var(--accent-text); border-color: var(--accent-line); display: inline-flex; align-items: center; gap: 4px; }
  .clip-chip { display: inline-block; max-width: 100%; white-space: normal; overflow-wrap: anywhere; vertical-align: middle; }
  .mono { font-family: var(--font-mono); }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tab-picker { display: none; }
  .tabs { display: flex; gap: 4px; overflow-x: auto; padding-inline: 2px; }
  @media (max-width: 640px) {
    .tabs { display: none; }
    .tab-picker {
      display: flex; align-items: center; gap: 8px; width: 100%; min-width: 0;
    }
    .tab-picker-label { color: var(--text-dim); font-size: var(--fs-s); flex: none; }
    .tab-picker select { flex: 1; min-width: 0; }
  }
  .tab {
    background: none; border: none; border-bottom: 2px solid transparent; color: var(--text-dim);
    padding: 8px 10px; font: inherit; font-size: var(--fs-m); cursor: pointer; white-space: nowrap;
  }
  .tab:hover { color: var(--text); }
  .tab.active { color: var(--text); border-bottom-color: var(--accent); font-weight: 500; }
  .tab:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: var(--radius-s); }
  .about { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; align-items: start; }
  @media (max-width: 640px) { .about { grid-template-columns: minmax(0, 1fr); } }
  /* "Who it is" spans the row; Personality and Setup share the next one — no half-empty row. */
  .about > .who { grid-column: 1 / -1; }
  .who { grid-column: 1 / -1; }
  .who-head { display: flex; align-items: center; gap: 12px; margin-block-end: 12px; }
  .setup-head { display: flex; align-items: baseline; gap: 8px; margin-block-end: 8px; }
  .setup-head h2 { margin: 0; flex: 1; }
  .setup-head h3 { margin: 0; flex: 1; font-size: var(--fs-m); font-weight: 600; }
  .who-t { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; }
  .who-t h2 { margin: 0; }
  .who h3 { margin: 0 0 4px; font-size: var(--fs-s); font-weight: 600; color: var(--text-dim); }
  .soul.clamped { display: -webkit-box; -webkit-line-clamp: 6; line-clamp: 6; -webkit-box-orient: vertical; overflow: hidden; }
  .pa-panel { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 12px 14px; color: var(--text); min-width: 0; }
  .muted { color: var(--text-dim); font-size: var(--fs-m); }
  .pa-panel h2 { margin: 0 0 8px; font-size: var(--fs-l); font-weight: 600; color: var(--text); }
  .soul-edit { inline-size: 100%; resize: vertical; font-size: var(--fs-m); line-height: 1.5; }
  .about-note { margin: 0 0 8px; }
  .soul { white-space: pre-wrap; word-break: break-word; font-family: inherit; font-size: var(--fs-m); line-height: 1.5; margin: 0; color: var(--text); }
  .props { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 12px; margin: 0; font-size: var(--fs-m); }
  .props dt { color: var(--text-dim); }
  .props dd { margin: 0; color: var(--text); min-width: 0; }
  .model-sel { padding: 2px 6px; font-size: var(--fs-s); max-width: 100%; }
  .prov-dd { display: flex; align-items: center; gap: 6px; }
  .clip { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link { border: 0; background: none; padding: 0; margin-inline-start: 6px; font: inherit; color: var(--accent-text); cursor: pointer; }
  .who .link { margin: 6px 0 0; font-size: var(--fs-s); }
  .link:hover { text-decoration: underline; }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .run {
    display: flex; align-items: center; gap: 8px; font-size: var(--fs-s); flex-wrap: wrap; color: var(--text);
    border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 8px 12px;
  }
  .run-when { color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .run-sum { flex: 1; min-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .run-err { flex-basis: 100%; margin: 0; font-size: var(--fs-xs); color: var(--danger); overflow-wrap: anywhere; }
  .run-err.warn { color: var(--warning); }
  .pa-warn { color: var(--warning); border-color: color-mix(in srgb, var(--warning) 35%, transparent); }
</style>
