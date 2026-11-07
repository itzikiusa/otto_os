<script lang="ts">
  // Personal Agents module — a messenger-style home. Routes:
  //   `#/personal-agents`                — the home on the main agent
  //   `#/personal-agents/<agentId>`      — the home on that agent
  //   `#/personal-agents/<agentId>/<tab>` — the agent's settings page
  //                                        (general/routines/history/…)
  //   `#/personal-agents/rooms`          — agent channels
  // The home is the agent list (main agent first, live presence), the
  // selected agent's ONE continuous conversation, and a collapsible side
  // panel with its routines and recent runs.
  import AutomateTasksTabs from '../../lib/components/AutomateTasksTabs.svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import AutomateGuide from '../../lib/components/AutomateGuide.svelte';
  import AutomateGuideButton from '../../lib/components/AutomateGuideButton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import AgentRail from './AgentRail.svelte';
  import { railOrder } from './agentHome';
  import { confirmDeleteAgent } from './confirmDelete';
  import { MASCOT } from './characters';
  import { loadErrorOf } from './loadError';
  import type { PersonalAgent, RuntimeEvent } from '../../lib/api/types';
  import { onLive } from '../../lib/live';
  import { paListPort } from '../../lib/uiCommands/personalAgentsPort';

  const AgentEditSheetLazy = lazyComponent(() => import('./AgentEditSheet.svelte'));
  const AgentPageLazy = lazyComponent(() => import('./AgentPage.svelte'));
  // The side panel and the quick create carry the personality / template
  // catalogs: loaded when first shown, never with the page.
  const SidePanelLazy = lazyComponent(() => import('./AgentSidePanel.svelte'));
  // The conversation (header, intro, composer) loads beside the list; its
  // chat view is lazy again inside it.
  const ConversationLazy = lazyComponent(() => import('./AgentConversation.svelte'));
  const RoomsLazy = lazyComponent(() => import('./RoomsView.svelte'));
  const QuickCreateLazy = lazyComponent(() => import('./AgentQuickCreate.svelte'));

  const sub = $derived(router.parts[1] ?? '');
  const deepTab = $derived(router.parts[2] ?? '');
  /** `#/personal-agents/<id>/<tab>` = the full settings page. */
  const deepId = $derived(sub && sub !== 'rooms' && deepTab ? sub : null);

  let creating = $state(false);
  let fullCreate = $state(false);
  // Agent UI control (lib/uiCommands/personalAgents.ts) opens the full New-agent sheet.
  $effect(() => paListPort.bind({ openCreate: () => { if (deepId) router.go('personal-agents'); fullCreate = true; } }));

  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    void personalAgents.loadAgents(id);
    void personalAgents.loadThreads(id);
  });
  // Live run / thread events refresh while this page is mounted; agent
  // threads' status (rail presence, Activity) follows the runtime events.
  $effect(() => personalAgents.watch());
  $effect(() => onLive(['agent_runtime_event'], (ev) => personalAgents.applyThreadEvent(ev.event as RuntimeEvent)));

  const agents = $derived(railOrder(personalAgents.agents, (id) => !!personalAgents.autonomyById[id]?.primary));
  const autonomyLoaded = $derived(agents.length > 0 && agents.every((a) => a.id in personalAgents.autonomyById));
  const hasMain = $derived(!!personalAgents.primary);
  const selected = $derived.by<PersonalAgent | null>(() => {
    if (sub && sub !== 'rooms' && !deepTab) return personalAgents.agent(sub) ?? null;
    return agents[0] ?? null;
  });
  const loadError = $derived(loadErrorOf(personalAgents, 'agentsError'));

  // The side panel: open/closed, remembered per viewer.
  const PANEL_KEY = 'otto.pa.panel';
  function readPanel(): boolean {
    try {
      return (JSON.parse(localStorage.getItem(PANEL_KEY) ?? 'null') as { open?: boolean } | null)?.open ?? true;
    } catch {
      return true;
    }
  }
  let panelOpen = $state(readPanel() && typeof window !== 'undefined' && window.innerWidth > 1100);
  // "Add a routine" (after creating an agent) needs the panel that holds the form.
  $effect(() => {
    if (personalAgents.routineFormFor && !panelOpen) panelOpen = true;
  });
  function togglePanel(): void {
    panelOpen = !panelOpen;
    try {
      localStorage.setItem(PANEL_KEY, JSON.stringify({ open: panelOpen }));
    } catch {
      /* per-viewer convenience only */
    }
  }

  function select(id: string): void {
    router.go(`personal-agents/${id}`);
  }

  // ⌘K verbs while the module is open.
  $effect(() =>
    registry.register('personal-agents', [
      { id: 'personal-agents.new', title: 'New personal agent…', group: 'Personal Agents', keywords: 'create agent assistant schedule', run: () => { creating = true; } },
      { id: 'personal-agents.rooms', title: 'Open agent channels', group: 'Personal Agents', keywords: 'channel agent rooms talk', run: () => router.go('personal-agents/rooms') },
      ...agents.flatMap((a) => [
        { id: `personal-agents.chat.${a.id}`, title: `Chat with ${a.name}`, group: 'Personal Agents', keywords: 'agent message talk', run: () => router.go(`personal-agents/${a.id}`) },
        { id: `personal-agents.open.${a.id}`, title: `${a.name}: settings`, group: 'Personal Agents', detail: 'Personal agent', keywords: 'agent page settings runs routines', run: () => router.go(`personal-agents/${a.id}/about`) },
      ]),
    ]),
  );

  async function remove(a: PersonalAgent): Promise<void> {
    if (!(await confirmDeleteAgent(a, !!personalAgents.autonomyById[a.id]?.primary))) return;
    try {
      await personalAgents.remove(a.id);
      toasts.success(`Deleted ${a.name}`);
      router.go('personal-agents');
    } catch (e) {
      toasts.error(`Couldn’t delete ${a.name}`, loadErrorText(e));
    }
  }
</script>

{#if deepId}
  {#key deepId}
    {#if AgentPageLazy.error}
      <LoadState what="this agent" variant="page" error={AgentPageLazy.error} onretry={() => AgentPageLazy.retry()} />
    {:else if AgentPageLazy.component}
      <AgentPageLazy.component agentId={deepId} />
    {:else}
      <LoadState what="this agent" variant="page" loading />
    {/if}
  {/key}
{:else}
  <div class="pa-page">
  <PageHeader
    title="Personal Agents"
  >
    {#snippet tabs()}
      <AutomateTasksTabs active={sub === 'rooms' ? 'rooms' : 'agents'} />
    {/snippet}
    {#snippet actions()}
      <AutomateGuideButton current="scheduled-tasks" />
    {/snippet}
  </PageHeader>
  <PageBody fill padded={sub === 'rooms'}>
    {#if !ws.currentId}
      <EmptyState variant="page" icon="user" title="Add a workspace to get started"
        body="Personal agents and their channels belong to a workspace. Add your project folder to create them."
        actionLabel="Add workspace" actionIcon="plus" onaction={() => (ui.newWorkspaceOpen = true)} />
    {:else if sub === 'rooms'}
      {#if RoomsLazy.component}
        <RoomsLazy.component />
      {:else}
        <LoadState what="agent channels" variant="page" error={RoomsLazy.error} loading={!RoomsLazy.error} empty onretry={() => RoomsLazy.retry()} />
      {/if}
    {:else}
      <LoadState
        what="personal agents"
        loading={personalAgents.loadingAgents && agents.length === 0}
        error={agents.length === 0 ? loadError : null}
        empty={agents.length === 0}
        variant="page"
        rows={3}
        onretry={() => ws.currentId && void personalAgents.loadAgents(ws.currentId)}
      >
        {#snippet emptyView()}
          <img class="pa-mascot" src={MASCOT.wave} alt="" width="160" height="160" loading="lazy" decoding="async" />
          <EmptyState
            icon="user"
            title="Meet your main agent"
            body="Your main agent is your one point of contact: task it with anything, and it routes the work to the right agent and comes back with what they found."
            actionLabel="Set up your main agent"
            actionIcon="plus"
            variant="page"
            onaction={() => (creating = true)}
          >
            <AutomateGuide current="scheduled-tasks" />
          </EmptyState>
        {/snippet}
        <div class="pa-home" class:panel={panelOpen && !!selected}>
          <AgentRail {agents} selectedId={selected?.id ?? null} onselect={select} oncreate={() => (creating = true)} />
          <div class="pa-center">
            {#if autonomyLoaded && !hasMain}
              <p class="pa-hint" role="note">
                <Icon name="star" size={12} /> No main agent yet — it is the one you talk to, and it hands work to the others.
                <button type="button" class="linklike" onclick={() => (creating = true)}>Set one up</button>
              </p>
            {/if}
            {#if selected}
              {#if ConversationLazy.component}
                {#key selected.id}
                  <ConversationLazy.component agent={selected} {panelOpen} ontogglepanel={togglePanel} ondelete={(a) => void remove(a)} />
                {/key}
              {:else}
                <LoadState what="the conversation" error={ConversationLazy.error} loading={!ConversationLazy.error} empty onretry={() => ConversationLazy.retry()} />
              {/if}
            {:else}
              <EmptyState icon="user" title="Agent not found" body="It may have been deleted, or it belongs to another workspace." actionLabel="Back to your agents" onaction={() => router.go('personal-agents')} />
            {/if}
          </div>
          {#if panelOpen && selected}
            {#if SidePanelLazy.component}
              <SidePanelLazy.component agent={selected} onclose={togglePanel} />
            {:else}
              <div class="pa-panel-slot"><LoadState what="routines and runs" error={SidePanelLazy.error} loading={!SidePanelLazy.error} empty onretry={() => SidePanelLazy.retry()} /></div>
            {/if}
          {/if}
        </div>
      </LoadState>
    {/if}

    {#if creating}
      {#if QuickCreateLazy.component}
        <QuickCreateLazy.component
          needsMain={!hasMain}
          oncreated={(a: PersonalAgent) => { creating = false; select(a.id); }}
          onclose={() => (creating = false)}
        />
      {:else}
        <LoadState what="new-agent form" error={QuickCreateLazy.error} loading={!QuickCreateLazy.error} empty onretry={() => QuickCreateLazy.retry()} />
      {/if}
    {/if}
    {#if fullCreate}
      {#if AgentEditSheetLazy.component}
        <AgentEditSheetLazy.component agent={null} onclose={() => (fullCreate = false)} />
      {:else}
        <LoadState what="new-agent form" error={AgentEditSheetLazy.error} loading={!AgentEditSheetLazy.error} empty onretry={() => AgentEditSheetLazy.retry()} />
      {/if}
    {/if}
  </PageBody>
  </div>
{/if}

<style>
  .pa-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .pa-mascot { display: block; margin: 24px auto 0; }
  .pa-home {
    position: relative; display: flex; flex: 1; min-block-size: 0; block-size: 100%;
    border-block-start: 1px solid var(--border); background: var(--bg);
  }
  .pa-panel-slot { inline-size: 320px; flex: none; border-inline-start: 1px solid var(--border); padding: 12px; }
  .pa-center { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; min-block-size: 0; }
  .pa-hint { display: flex; align-items: center; gap: 6px; margin: 0; padding: 6px 12px; color: var(--text-dim); font-size: var(--fs-s); flex-wrap: wrap; border-block-end: 1px solid var(--border); }
  .linklike { border: 0; background: none; padding: 0; color: var(--accent-text); font: inherit; font-weight: 600; cursor: pointer; }
  .linklike:hover { text-decoration: underline; }
  @media (max-width: 640px) {
    .pa-home { flex-direction: column; }
  }
</style>
