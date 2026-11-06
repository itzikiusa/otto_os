<!--
  The agent's side panel (opened from the conversation header):
  · Details — who it is, its Routines (live: an agent that sets one up in chat
    shows here at once), what it can use, and settings toggles;
  · Library — the reports its runs produced, newest first, openable;
  · Activity — what it is doing now: running runs, hand-offs it led or works,
    and its threads, each with what started it, openable in Threads.
  The full per-agent page (autonomy, memory, context, run history) stays one
  click away ("All settings").
-->
<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import { plural } from '../../lib/plural';
  import { authedText } from '../../lib/api/client';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { latestOnly } from '../../lib/latest';
  import { runStatus } from '../../lib/status';
  import { openAgentSession } from '../../lib/openAgentSession';
  import { agentThreadBadge, routineLabel, runBadge } from '../../lib/personalAgentBadges';
  import { cadenceLabel } from './cadence';
  import { about } from './agentHome';
  import AgentAvatar from './AgentAvatar.svelte';
  import AgentSlackAsMe from './AgentSlackAsMe.svelte';
  import { personalityOf, personalityPresets } from './personalities';
  import { MASCOT } from './characters';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type {
    PersonalAgent,
    PersonalAgentDelegation,
    PersonalAgentInstructionsProposal,
    PersonalAgentRoutine,
    PersonalAgentRun,
    PersonalAgentSchedule,
  } from '../../lib/api/types';

  const MemoryLazy = lazyComponent(() => import('./AgentMemoryInspector.svelte'));

  export type PanelTab = 'details' | 'library' | 'activity';
  interface Props {
    agent: PersonalAgent;
    tab: PanelTab;
    ontab: (t: PanelTab) => void;
    onclose: () => void;
  }
  let { agent, tab, ontab, onclose }: Props = $props();

  const TABS: { id: PanelTab; label: string; testid: string }[] = [
    { id: 'details', label: 'Details', testid: 'pa-panel-tab-details' },
    { id: 'library', label: 'Library', testid: 'pa-panel-tab-library' },
    { id: 'activity', label: 'Activity', testid: 'pa-panel-tab-activity' },
  ];

  const schedules = $derived(personalAgents.schedulesByAgent[agent.id] ?? []);
  const runs = $derived(personalAgents.runsByAgent[agent.id] ?? []);
  const reports = $derived(runs.filter((r) => r.report_rel));
  const running = $derived(runs.filter((r) => r.status === 'running'));
  const delegations = $derived(personalAgents.delegationsByAgent[agent.id] ?? []);
  const threads = $derived(personalAgents.threadsOf(agent.id, false));
  const busyThreads = $derived(threads.filter((t) => t.status === 'working' || t.status === 'starting' || t.status === 'awaiting_permission'));
  const autonomy = $derived(personalAgents.autonomyById[agent.id] ?? null);
  const nameOf = (id: string): string | undefined => personalAgents.agent(id)?.name;

  $effect(() => {
    const id = agent.id;
    void personalAgents.loadRuns(id);
    void personalAgents.loadDelegations(id);
  });

  // Routines with their next runs: re-read whenever the agent's schedules
  // change (any path — the chat's `otto.routine_*` tools included).
  let routines = $state<PersonalAgentRoutine[]>([]);
  let routinesTz = $state('');
  $effect(() => {
    const id = agent.id;
    void personalAgents.schedulesByAgent[id];
    void agent.enabled;
    personalAgentsApi
      .routines(id)
      .then((r) => {
        if (id !== agent.id) return;
        routines = r.routines;
        routinesTz = r.user_timezone;
      })
      .catch(() => {});
  });
  const fmtRun = (iso: string): string =>
    new Date(iso).toLocaleString(undefined, { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });

  // "Describe a routine": the agent sets it up in its chat (otto.routine_create).
  let describe = $state('');
  let describing = $state(false);
  async function sendRoutine(): Promise<void> {
    const text = describe.trim();
    if (!text || describing) return;
    describing = true;
    try {
      const chat = await personalAgentsApi.chatSession(agent.id);
      if (!chat.runtime) throw new Error('This agent’s chat is a terminal session — type it there.');
      await api.post(`/runtime-sessions/${chat.session_id}/prompt`, { text: `Set up this routine for yourself: ${text}` });
      describe = '';
      toasts.success(`Sent to ${agent.name}`, 'It sets the routine up in your chat — it shows here as soon as it’s saved.');
    } catch (e) {
      toasts.error('Couldn’t send it to the chat', loadErrorText(e));
    } finally {
      describing = false;
    }
  }

  // --- Personality -----------------------------------------------------------
  const PRESETS = personalityPresets();
  const myRole = $derived(ws.workspaces.find((w) => w.id === agent.workspace_id)?.my_role ?? 'viewer');
  const isOwner = $derived(auth.isRoot || myRole === 'admin' || !agent.created_by || agent.created_by === auth.me?.id);
  const persona = $derived(personalityOf(autonomy?.personality?.preset, !!autonomy?.primary));
  let noteDraft = $state<string | null>(null);
  let slackOpen = $state(false);
  let learnedOpen = $state(false);
  async function setPersonality(body: { preset?: string; note?: string }): Promise<void> {
    try {
      await personalAgentsApi.setPersonality(agent.id, body);
      await personalAgents.loadAutonomy(agent.id);
      noteDraft = null;
    } catch (e) {
      toasts.error('Couldn’t change the personality', loadErrorText(e));
    }
  }

  // --- Instruction proposals --------------------------------------------------
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

  function routineName(s: PersonalAgentSchedule): string {
    return routineLabel(s.directive);
  }
  function runRoutine(r: PersonalAgentRun): string | null {
    const s = r.schedule_id ? schedules.find((x) => x.id === r.schedule_id) : undefined;
    return s ? routineName(s) : null;
  }

  async function toggleRoutine(s: PersonalAgentSchedule): Promise<void> {
    try {
      await personalAgents.updateSchedule(agent.id, s.id, { enabled: !s.enabled });
    } catch (e) {
      toasts.error(s.enabled ? 'Couldn’t pause the routine' : 'Couldn’t resume the routine', loadErrorText(e));
    }
  }

  async function makeMain(): Promise<void> {
    try {
      await personalAgentsApi.saveAutonomy(agent.id, { primary: true });
      await Promise.all(personalAgents.agents.map((a) => personalAgents.loadAutonomy(a.id)));
      toasts.success(`${agent.name} is your main agent`, 'It routes your work to the other agents.');
    } catch (e) {
      toasts.error('Couldn’t make it the main agent', loadErrorText(e));
    }
  }

  async function stopRun(r: PersonalAgentRun): Promise<void> {
    try {
      await personalAgentsApi.cancelRun(r.id);
      toasts.info('Stopping the run…');
    } catch (e) {
      toasts.error('Couldn’t stop the run', loadErrorText(e));
    }
  }

  // --- Report viewer ---------------------------------------------------------
  let reportRun = $state<PersonalAgentRun | null>(null);
  let reportText = $state('');
  let reportError = $state('');
  let reportLoading = $state(false);
  const reportSeq = latestOnly();
  async function viewReport(r: PersonalAgentRun): Promise<void> {
    reportRun = r;
    reportLoading = true;
    reportText = '';
    reportError = '';
    const t = reportSeq.begin();
    try {
      const text = await authedText(personalAgentsApi.reportPath(r.id));
      if (t.current) reportText = text;
    } catch (e) {
      if (t.current) reportError = loadErrorText(e);
    } finally {
      if (t.current) reportLoading = false;
    }
  }

  const DELEGATION_TONE: Record<PersonalAgentDelegation['status'], string> = {
    running: 'Working…',
    done: 'Done',
    failed: 'Failed',
    stopped: 'Stopped',
    interrupted: 'Interrupted',
  };
  function delegationLine(d: PersonalAgentDelegation): string {
    return d.lead_agent_id === agent.id ? `To ${d.member_name}` : `From ${nameOf(d.lead_agent_id) ?? 'the main agent'}`;
  }
</script>

<aside class="sp" aria-label="{agent.name} details" data-testid="pa-side-panel">
  <div class="sp-head">
    <div class="sp-tabs">
      <Tabs label="Agent panel" tabs={TABS} value={tab} onchange={ontab} idBase="pa-panel" size="s" />
    </div>
    <button type="button" class="icon-btn" aria-label="Close panel" title="Close panel" onclick={onclose}><Icon name="x" size={14} /></button>
  </div>

  <div class="sp-body" role="tabpanel" id="pa-panel-panel-{TABS.findIndex((t) => t.id === tab)}" aria-labelledby="pa-panel-tab-{TABS.findIndex((t) => t.id === tab)}">
    {#if tab === 'details'}
      <section class="sp-sec">
        <div class="sp-who">
          <AgentAvatar avatar={agent.avatar} name={agent.name} size={40} />
          <div class="sp-who-t">
            <strong dir="auto">{agent.name}</strong>
            <span class="sp-dim">{about(agent)}</span>
          </div>
        </div>
        <dl class="sp-props">
          <dt>Model</dt>
          <dd class="sp-prov"><ProviderIcon provider={agent.provider} size={12} />{agent.provider || 'claude'}{agent.model ? ` · ${agent.model}` : ' · default model'}</dd>
          <dt>Status</dt>
          <dd>{agent.enabled ? 'Enabled' : 'Paused — routines don’t fire'}</dd>
          <dt>Role</dt>
          <dd>
            {#if autonomy?.primary}
              <span class="sp-main"><Icon name="star" size={12} /> Main agent — routes your work to the others</span>
            {:else}
              Specialist
              <button type="button" class="linklike" onclick={() => void makeMain()}>Make main agent</button>
            {/if}
          </dd>
        </dl>
      </section>

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
        <p class="sp-dim sp-note">Instructions were updated. <button type="button" class="linklike" onclick={() => void undoInstructions()}>Undo</button></p>
      {/if}

      <section class="sp-sec" data-testid="pa-personality">
        <h3>Personality</h3>
        <div class="sp-chips" role="group" aria-label="Personality">
          {#each PRESETS as p (p.id)}
            <button type="button" class="sp-pchip" class:on={persona?.id === p.id} aria-pressed={persona?.id === p.id} title={p.description} onclick={() => void setPersonality({ preset: p.id })}>{p.label}</button>
          {/each}
        </div>
        {#if persona}
          <p class="sp-dim">{persona.description}</p>
          <p class="sp-sample" dir="auto"><span class="sp-dim">How {agent.name} would answer:</span> “{persona.sample}”</p>
        {/if}
        {#if isOwner}
          {#if noteDraft === null}
            <button type="button" class="linklike" onclick={() => (noteDraft = autonomy?.personality?.note ?? '')}>{autonomy?.personality?.note ? `Your words: “${autonomy.personality.note}”` : 'Add your own words'}</button>
          {:else}
            <div class="sp-row">
              <input class="input" dir="auto" maxlength="400" aria-label="How it should come across, in your words" placeholder="e.g. a bit more blunt, no emoji" bind:value={noteDraft} />
              <button type="button" class="btn small" onclick={() => void setPersonality({ note: noteDraft ?? '' })}>Save</button>
            </div>
          {/if}
        {/if}
        <p class="sp-dim sp-note">Tone only — it never changes rules or approvals. You can also say “be more blunt” in chat.</p>
      </section>

      <section class="sp-sec" data-testid="pa-routines">
        <h3>Routines</h3>
        <p class="sp-dim">Recurring tasks this agent runs on a schedule. Ask it in chat to set one up — it shows here at once, and each result is posted into the chat.</p>
        {#if personalAgents.schedulesLoading[agent.id] && schedules.length === 0}
          <Skeleton rows={2} height={32} label="routines" />
        {:else if personalAgents.schedulesError[agent.id]}
          <LoadState what="routines" error={personalAgents.schedulesError[agent.id]} empty onretry={() => void personalAgents.loadSchedules(agent.id)} />
        {:else if schedules.length === 0}
          <p class="sp-empty">No routines yet.</p>
        {:else}
          <ul class="sp-list">
            {#each schedules as s (s.id)}
              {@const r = routines.find((x) => x.id === s.id)}
              <li class="sp-routine" data-schedule-id={s.id}>
                <div class="sp-routine-main">
                  <strong class="ellipsis" dir="auto" title={s.directive}>{routineName(s)}</strong>
                  <span class="sp-dim">{cadenceLabel(s.schedule, s.timezone, { cronWords: true })}{r?.deliver.type === 'slack' ? ` · Slack${r.deliver.as === 'me' ? ' as you' : ''}` : ''}</span>
                  {#if !s.enabled}
                    <span class="sp-dim">Paused</span>
                  {:else if !agent.enabled}
                    <span class="sp-dim">Agent paused</span>
                  {:else if r?.next_runs.length}
                    <span class="sp-dim">Next: {r.next_runs.map(fmtRun).join(' · ')}</span>
                  {:else}
                    <span class="sp-dim">Next <RelTime iso={s.next_run_at} fallback="not scheduled" /></span>
                  {/if}
                </div>
                <button type="button" class="btn small" aria-label="{s.enabled ? 'Pause' : 'Resume'} {routineName(s)}" onclick={() => void toggleRoutine(s)}>{s.enabled ? 'Pause' : 'Resume'}</button>
              </li>
            {/each}
          </ul>
        {/if}
        <form class="sp-row" onsubmit={(e) => { e.preventDefault(); void sendRoutine(); }}>
          <input class="input" dir="auto" aria-label="Describe a routine" placeholder="Describe a routine — “every weekday at 9, brief me on my PRs”" bind:value={describe} />
          <button type="submit" class="btn small" disabled={describing || !describe.trim()}>Set up</button>
        </form>
        {#if routinesTz}<p class="sp-dim sp-note">Times default to your timezone ({routinesTz}).</p>{/if}
        <button type="button" class="linklike" onclick={() => router.go(`personal-agents/${agent.id}/schedules`)}>Add or edit routines</button>
        <p class="sp-dim sp-note">Workspace Scheduled Tasks are a separate kind of automation; they live under <button type="button" class="linklike" onclick={() => router.go('scheduled-tasks')}>Tasks</button>.</p>
      </section>

      <section class="sp-sec">
        <h3>What it can use</h3>
        <p class="sp-dim">
          Otto’s tools (MCP){agent.browser ? ', the in-app browser' : ''}{autonomy?.rules.length ? `, under ${plural(autonomy.rules.length, 'rule')} you set` : ''}.
          {#if autonomy?.proactive.enabled} Works its standing goals in the background, read-only.{/if}
        </p>
        <details class="sp-more" ontoggle={(e) => (slackOpen = (e.currentTarget as HTMLDetailsElement).open)}>
          <summary>Act as you in Slack</summary>
          {#if slackOpen}<AgentSlackAsMe agentId={agent.id} />{/if}
        </details>
      </section>

      <!-- Loaded only when opened: the memory inspector reads the agent's notes. -->
      <details class="sp-sec sp-more" data-testid="pa-learned" ontoggle={(e) => (learnedOpen = (e.currentTarget as HTMLDetailsElement).open)}>
        <summary>What it has learned</summary>
        <p class="sp-dim">Its memory: one fact per line, curated after chats and runs. Edit or forget anything.</p>
        {#if learnedOpen}
          {#if MemoryLazy.error}
            <LoadState what="its memory" error={MemoryLazy.error} onretry={() => MemoryLazy.retry()} />
          {:else if MemoryLazy.component}
            <MemoryLazy.component agentId={agent.id} editable={isOwner} reloadKey={0} />
          {:else}
            <Skeleton rows={2} height={20} label="memory" />
          {/if}
        {/if}
      </details>

      <button type="button" class="btn small" onclick={() => router.go(`personal-agents/${agent.id}/overview`)}><Icon name="gear" size={12} /> All settings</button>
    {:else if tab === 'library'}
      <section class="sp-sec" data-testid="pa-library">
        <p class="sp-dim">Reports this agent’s runs produced, newest first.</p>
        {#if personalAgents.runsByAgent[agent.id] === undefined}
          <Skeleton rows={3} height={40} label="reports" />
        {:else if personalAgents.runsError[agent.id] && reports.length === 0}
          <LoadState what="reports" error={personalAgents.runsError[agent.id]} empty onretry={() => void personalAgents.loadRuns(agent.id)} />
        {:else if reports.length === 0}
          <p class="sp-empty">Nothing here yet. Reports land here when a routine or a run finishes.</p>
        {:else}
          <ul class="sp-list">
            {#each reports as r (r.id)}
              {@const b = runBadge(r, runRoutine(r))}
              <li class="sp-item">
                <button type="button" class="sp-item-main" onclick={() => void viewReport(r)}>
                  <Icon name="file" size={14} />
                  <span class="sp-item-t">
                    <span class="ellipsis" dir="auto">{r.summary || 'Report'}</span>
                    <span class="sp-dim"><span class="sp-chip sp-chip-{b.kind}" title={b.title}>{b.label}</span> <RelTime iso={r.finished_at ?? r.started_at} /></span>
                  </span>
                </button>
                {#if r.session_id}
                  <button type="button" class="icon-btn" aria-label="Open the run’s thread" title="Open the run’s thread" onclick={() => void openAgentSession(r.session_id ?? '')}><Icon name="external" size={12} /></button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        <div class="sp-links">
          <button type="button" class="linklike" onclick={() => router.go(`personal-agents/${agent.id}/memory`)}>Memory</button>
          <button type="button" class="linklike" onclick={() => router.go(`personal-agents/${agent.id}/context`)}>Context</button>
          <button type="button" class="linklike" onclick={() => router.go(`personal-agents/${agent.id}/runs`)}>All runs</button>
        </div>
      </section>
    {:else}
      <section class="sp-sec" data-testid="pa-activity">
        <h3>Now</h3>
        {#if running.length === 0 && busyThreads.length === 0}
          <div class="sp-idle">
            <img src={MASCOT.sleep} alt="" width="96" height="96" loading="lazy" decoding="async" />
            <p class="sp-empty">All caught up — nothing running.</p>
          </div>
        {/if}
        <ul class="sp-list">
          {#each running as r (r.id)}
            {@const b = runBadge(r, runRoutine(r))}
            <li class="sp-item">
              <span class="sp-item-main static"><span class="spinner" aria-hidden="true"></span><span class="sp-item-t"><span>{b.label}</span><span class="sp-dim">started <RelTime iso={r.started_at} /></span></span></span>
              <button type="button" class="btn small" onclick={() => void stopRun(r)}>Stop</button>
            </li>
          {/each}
          {#each busyThreads as t (t.id)}
            {@const b = agentThreadBadge(t.meta, nameOf)}
            <li class="sp-item">
              <button type="button" class="sp-item-main" onclick={() => router.go(`threads/${t.id}`)}>
                {#if t.status === 'awaiting_permission'}<Icon name="hand" size={14} />{:else}<span class="spinner" aria-hidden="true"></span>{/if}
                <span class="sp-item-t">
                  <span>{t.status === 'awaiting_permission' ? 'Waiting for your approval' : 'Working…'}{b ? ` · ${b.label}` : ''}</span>
                  <span class="sp-dim ellipsis" dir="auto">{t.title}</span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      </section>
      {#if delegations.length > 0}
        <section class="sp-sec" data-testid="pa-delegations">
          <h3>Hand-offs</h3>
          <ul class="sp-list">
            {#each delegations.slice(0, 12) as d (d.id)}
              <li class="sp-item">
                <button type="button" class="sp-item-main" title="Open the delegated thread" onclick={() => router.go(`threads/${d.id}`)}>
                  <AgentAvatar avatar={personalAgents.agent(d.lead_agent_id === agent.id ? d.member_agent_id : d.lead_agent_id)?.avatar} name={d.lead_agent_id === agent.id ? d.member_name : (nameOf(d.lead_agent_id) ?? '?')} size={20} />
                  <span class="sp-item-t">
                    <span class="ellipsis" dir="auto">{delegationLine(d)}: {d.task}</span>
                    <span class="sp-dim"><span class="sp-state sp-state-{d.status}">{DELEGATION_TONE[d.status]}</span>{d.summary ? ` · ${d.summary}` : ''} · <RelTime iso={d.finished_at ?? d.created_at} /></span>
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
      <section class="sp-sec">
        <h3>Threads</h3>
        {#if threads.length === 0}
          <p class="sp-empty">No open threads.</p>
        {:else}
          <ul class="sp-list">
            {#each threads.slice(0, 20) as t (t.id)}
              {@const b = agentThreadBadge(t.meta, nameOf)}
              <li class="sp-item">
                <button type="button" class="sp-item-main" onclick={() => router.go(`threads/${t.id}`)}>
                  {#if t.status === 'working' || t.status === 'starting'}<span class="spinner" aria-hidden="true"></span>{:else if t.status === 'awaiting_permission'}<Icon name="hand" size={14} />{:else}<Icon name="threads" size={14} />{/if}
                  <span class="sp-item-t">
                    <span class="ellipsis" dir="auto">{t.title}</span>
                    <span class="sp-dim">{#if b}<span class="sp-chip sp-chip-{b.kind}" title={b.title}>{b.label}</span> {/if}<RelTime iso={t.last_active_at} /></span>
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        <button type="button" class="linklike" onclick={() => router.go(`personal-agents/${agent.id}/activity`)}>Tool calls and approvals</button>
      </section>
    {/if}
  </div>
</aside>

{#if reportRun}
  <Modal title={reportRun.summary || 'Report'} width={760} onclose={() => (reportRun = null)}>
    {#if reportLoading}
      <Skeleton rows={6} height={18} label="the report" />
    {:else if reportError}
      <LoadState what="the report" error={reportError} empty onretry={() => reportRun && void viewReport(reportRun)} />
    {:else}
      <StatusBadge status={runStatus(reportRun.status)} variant="text" />
      <pre class="sp-report">{reportText}</pre>
    {/if}
  </Modal>
{/if}

<style>
  .sp {
    display: flex; flex-direction: column; min-block-size: 0; inline-size: 340px; flex: none;
    border-inline-start: 1px solid var(--border); background: var(--surface);
  }
  .sp-head { display: flex; align-items: center; gap: 8px; padding: 6px 8px 0; border-block-end: 1px solid var(--border); }
  .sp-tabs { flex: 1; min-inline-size: 0; }
  .sp-body { overflow-y: auto; padding: 12px; display: flex; flex-direction: column; gap: 16px; min-block-size: 0; }
  .sp-sec { display: flex; flex-direction: column; gap: 8px; }
  .sp-sec h3 { margin: 0; font-size: var(--fs-m); font-weight: 600; color: var(--text); }
  .sp-who { display: flex; gap: 10px; align-items: center; }
  .sp-who-t { display: flex; flex-direction: column; min-inline-size: 0; }
  .sp-dim { color: var(--text-dim); font-size: var(--fs-s); }
  .sp-note { margin: 0; }
  .sp-dim:is(p) { margin: 0; }
  .sp-empty { margin: 0; color: var(--text-dim); font-size: var(--fs-s); }
  .sp-props { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px 10px; margin: 0; font-size: var(--fs-s); }
  .sp-props dt { color: var(--text-dim); }
  .sp-props dd { margin: 0; min-inline-size: 0; }
  .sp-prov { display: flex; align-items: center; gap: 4px; }
  .sp-main { display: inline-flex; align-items: center; gap: 4px; color: var(--accent-text); }
  .sp-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .sp-routine {
    display: flex; align-items: center; gap: 8px; padding: 8px; border: 1px solid var(--border);
    border-radius: var(--radius-m); background: var(--bg);
  }
  .sp-routine-main { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-s); }
  .sp-item { display: flex; align-items: center; gap: 4px; }
  .sp-item-main {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 8px; padding: 6px; border: 0;
    border-radius: var(--radius-m); background: transparent; color: var(--text); font: inherit; text-align: start; cursor: pointer;
  }
  .sp-item-main.static { cursor: default; }
  .sp-item-main:not(.static):hover { background: var(--hover); }
  .sp-item-main :global(svg) { flex: none; color: var(--text-dim); }
  .sp-item-t { display: flex; flex-direction: column; min-inline-size: 0; font-size: var(--fs-s); }
  .sp-chip { border: 1px solid var(--border); border-radius: 999px; padding: 0 6px; font-size: var(--fs-xs); }
  .sp-chip-chat { color: var(--accent-text); border-color: var(--accent-line); }
  .sp-chip-scheduled { color: var(--info); }
  .sp-chip-delegated { color: var(--success); }
  .sp-state-running { color: var(--info); }
  .sp-state-done { color: var(--success); }
  .sp-state-failed, .sp-state-interrupted { color: var(--danger); }
  .sp-state-stopped { color: var(--warning); }
  .sp-links { display: flex; gap: 12px; flex-wrap: wrap; }
  .sp-row { display: flex; gap: 6px; align-items: center; }
  .sp-row > :global(.input) { flex: 1; min-inline-size: 0; }
  .sp-card { border: 1px solid var(--accent-line); border-radius: var(--radius-m); padding: 10px; background: var(--accent-faint); }
  .sp-diff { display: grid; gap: 4px; }
  .sp-pre { margin: 0; max-block-size: 160px; overflow: auto; white-space: pre-wrap; word-break: break-word; font-family: inherit; font-size: var(--fs-s); padding: 6px 8px; border-radius: var(--radius-s); background: var(--surface); border: 1px solid var(--border); }
  .sp-pre.from { color: var(--text-dim); }
  .sp-chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .sp-pchip { border: 1px solid var(--border); border-radius: 999px; background: var(--surface); color: var(--text); padding: 2px 10px; font: inherit; font-size: var(--fs-s); cursor: pointer; }
  .sp-pchip:hover { border-color: var(--border-strong); }
  .sp-pchip.on { border-color: var(--accent-line-strong); background: var(--accent-soft); }
  .sp-pchip:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .sp-sample { margin: 0; font-size: var(--fs-s); }
  .sp-more > summary { cursor: pointer; font-size: var(--fs-m); font-weight: 600; color: var(--text); }
  .sp-more > summary:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .sp-idle { display: flex; flex-direction: column; align-items: center; gap: 4px; padding-block: 8px; }
  .linklike { border: 0; background: none; padding: 0; color: var(--accent-text); font: inherit; font-size: var(--fs-s); cursor: pointer; text-align: start; }
  .linklike:hover { text-decoration: underline; }
  .sp-report { white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono); font-size: var(--fs-s); line-height: 1.45; margin: 8px 0 0; }
  @media (max-width: 1024px) {
    .sp { position: absolute; inset-block: 0; inset-inline-end: 0; z-index: 3; }
  }
  @media (max-width: 640px) {
    .sp { inline-size: 100%; }
  }
</style>
