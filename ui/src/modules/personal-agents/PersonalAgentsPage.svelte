<script lang="ts">
  // Personal Agents module. Routes: `#/personal-agents` (agent cards),
  // `#/personal-agents/rooms` (agent rooms), `#/personal-agents/<agentId>[/<tab>]`
  // (one agent's page). The first list GET seeds four disabled example agents
  // server-side — they render as normal cards, marked "Example".
  import RelTime from '../../lib/components/RelTime.svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import AgentAvatar from './AgentAvatar.svelte';
  import AgentEditSheet from './AgentEditSheet.svelte';
  import AgentPage from './AgentPage.svelte';
  import RoomsView from './RoomsView.svelte';
  import { loadErrorOf } from './loadError';
  import type { PersonalAgent } from '../../lib/api/types';

  const sub = $derived(router.parts[1] ?? '');
  const agentId = $derived(sub && sub !== 'rooms' ? sub : null);

  let creating = $state(false);
  /** Agent id whose Run now / Enable / Pause is in flight (no double fire). */
  let busyId = $state<string | null>(null);

  $effect(() => {
    if (ws.currentId) void personalAgents.loadAgents(ws.currentId);
  });

  const agents = $derived(personalAgents.agents);
  const loadError = $derived(loadErrorOf(personalAgents, 'agentsError'));

  async function toggle(a: PersonalAgent): Promise<void> {
    busyId = a.id;
    try {
      await personalAgents.setEnabled(a.id, !a.enabled);
    } catch (e) {
      toasts.error(a.enabled ? `Couldn’t pause ${a.name}` : `Couldn’t enable ${a.name}`, loadErrorText(e));
    } finally {
      busyId = null;
    }
  }

  async function runNow(a: PersonalAgent): Promise<void> {
    busyId = a.id;
    try {
      await personalAgents.runNow(a.id);
      toasts.success(`${a.name} is running`, 'Its report lands in Runs when it finishes.');
      router.go(`personal-agents/${a.id}/runs`);
    } catch (e) {
      toasts.error(`Couldn’t run ${a.name}`, loadErrorText(e));
    } finally {
      busyId = null;
    }
  }

  async function remove(a: PersonalAgent): Promise<void> {
    if (!(await confirmer.ask(`Delete personal agent “${a.name}”? Its schedules, memory and run history go with it.`, { title: 'Delete personal agent', confirmLabel: 'Delete' }))) return;
    try {
      await personalAgents.remove(a.id);
      toasts.success(`Deleted ${a.name}`);
    } catch (e) {
      toasts.error(`Couldn’t delete ${a.name}`, loadErrorText(e));
    }
  }

  function cardMenu(e: MouseEvent | KeyboardEvent, a: PersonalAgent): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'user', action: () => router.go(`personal-agents/${a.id}`) },
      { label: 'Run now', icon: 'play', action: () => void runNow(a) },
      { label: a.enabled ? 'Pause' : 'Enable', icon: a.enabled ? 'clock' : 'check', action: () => void toggle(a) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(a) },
    ]);
  }

  /** A seeded example: disabled, never run, still carrying its shipped persona. */
  function isExample(a: PersonalAgent): boolean {
    return !a.enabled && !a.chat_session_id && (personalAgents.runsByAgent[a.id]?.length ?? 0) === 0;
  }

  function providerText(a: PersonalAgent): string {
    return a.model ? `${a.provider} · ${a.model}` : a.provider;
  }

  // Tab list keyboard: ←/→ switch between Agents and Rooms.
  function onTabKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft' && e.key !== 'Home' && e.key !== 'End') return;
    e.preventDefault();
    const toRooms = e.key === 'End' || ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && sub !== 'rooms');
    router.go(toRooms ? 'personal-agents/rooms' : 'personal-agents');
    queueMicrotask(() => (e.currentTarget as HTMLElement | null)?.querySelector<HTMLButtonElement>('[aria-selected="true"]')?.focus());
  }
</script>

{#if agentId}
  {#key agentId}
    <AgentPage {agentId} />
  {/key}
{:else}
  <div class="pa-page">
  <PageHeader
    title="Personal Agents"
    subtitle="Named agents with their own persona, schedules and memory — they talk to each other only in rooms you can read."
  >
    {#snippet tabs()}
      <div class="segmented" role="tablist" aria-label="Personal agents view" tabindex="-1" onkeydown={onTabKey}>
        <button role="tab" aria-selected={sub !== 'rooms'} tabindex={sub !== 'rooms' ? 0 : -1} class:active={sub !== 'rooms'} onclick={() => router.go('personal-agents')}>Agents</button>
        <button role="tab" aria-selected={sub === 'rooms'} tabindex={sub === 'rooms' ? 0 : -1} class:active={sub === 'rooms'} onclick={() => router.go('personal-agents/rooms')}>Rooms</button>
      </div>
    {/snippet}
    {#snippet actions()}
      {#if sub === 'rooms' || agents.length > 0}
        <button class="btn primary" data-icon="plus" onclick={() => (creating = true)}><Icon name="plus" size={12} /> New agent</button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody fill={sub === 'rooms'}>
  <div class="pa">
    {#if sub === 'rooms'}
      <RoomsView />
    {:else}
      <LoadState
        what="personal agents"
        loading={personalAgents.loadingAgents}
        error={loadError}
        empty={agents.length === 0}
        variant="page"
        rows={3}
        onretry={() => ws.currentId && void personalAgents.loadAgents(ws.currentId)}
      >
        {#snippet emptyView()}
          <EmptyState
            icon="user"
            title="No personal agents yet"
            body="A personal agent is a named persona on a pinned provider and model, with its own schedules, memory and delivery. Start blank or from a template."
            actionLabel="New agent"
            actionIcon="plus"
            variant="page"
            onaction={() => (creating = true)}
          />
        {/snippet}
        <ul class="cards" data-testid="pa-cards">
          {#each agents as a (a.id)}
            {@const example = isExample(a)}
            <li class="pa-card" class:paused={!a.enabled} oncontextmenu={(e) => cardMenu(e, a)}>
              <div class="card-top">
                <AgentAvatar avatar={a.avatar} name={a.name} size={36} />
                <div class="card-id">
                  <!-- The name is the card's one link; ::after stretches its hit
                       area over the whole card, so the buttons stay real buttons
                       (no button-in-button) and still sit on top. -->
                  <button class="name" title={a.name} onclick={() => router.go(`personal-agents/${a.id}`)}>{a.name}</button>
                  <span class="prov" title={providerText(a)}>
                    <ProviderIcon provider={a.provider} size={12} /><span class="prov-t">{providerText(a)}</span>
                  </span>
                </div>
                <button
                  class="icon-btn card-more"
                  aria-label="More actions for {a.name}"
                  title="More actions"
                  onclick={(e) => cardMenu(e, a)}><Icon name="more" size={14} /></button>
              </div>
              <div class="card-meta">
                {#if example}
                  <span class="chip" title="A shipped example — it stays off until you enable it. Run now still runs it once.">Example</span>
                {:else if !a.enabled}
                  <span class="chip" title="Schedules don’t fire while paused. Run now and chat still work.">Paused</span>
                {:else}
                  <span class="chip ok">Enabled</span>
                {/if}
                {#if a.browser}<span class="chip" title="Runs and chat can drive the in-app browser">Browser</span>{/if}
                <!-- A paused agent's schedules never fire — don't promise a next run. -->
                {#if a.enabled}
                  <span class="meta">Next run <RelTime iso={personalAgents.nextRunAt(a.id)} fallback="not scheduled" /></span>
                {/if}
              </div>
              <div class="card-actions">
                {#if a.enabled}
                  <button class="btn small" disabled={busyId === a.id} onclick={() => void runNow(a)}><Icon name="play" size={12} /> Run now</button>
                  <button class="btn small" disabled={busyId === a.id} onclick={() => void toggle(a)}>Pause</button>
                {:else}
                  <button class="btn small" disabled={busyId === a.id} onclick={() => void toggle(a)}><Icon name="check" size={12} /> Enable</button>
                  <button class="btn small" disabled={busyId === a.id} onclick={() => void runNow(a)} title="Run it once without enabling its schedules">Run once</button>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      </LoadState>
    {/if}

    {#if creating}
      <AgentEditSheet agent={null} onclose={() => (creating = false)} />
    {/if}
  </div>
  </PageBody>
  </div>
{/if}

<style>
  .pa-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .pa { display: flex; flex-direction: column; min-height: 0; flex: 1; }
  .cards {
    list-style: none; margin: 0; padding: 0; display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 12px;
  }
  .pa-card {
    position: relative;
    border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m);
    padding: 12px; color: var(--text); display: flex; flex-direction: column; gap: 10px;
    box-sizing: border-box; min-width: 0;
  }
  .pa-card:hover { border-color: var(--border-strong); }
  .pa-card:focus-within { border-color: var(--border-strong); }
  .card-top { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .card-id { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .name {
    all: unset; cursor: pointer; font-size: var(--fs-l); font-weight: 600; color: var(--text);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; border-radius: var(--radius-s);
  }
  /* Stretch the name's hit area over the whole card (the "card link" pattern). */
  .name::after { content: ''; position: absolute; inset: 0; border-radius: var(--radius-m); }
  .name:focus-visible { outline: none; }
  .name:focus-visible::after { outline: 2px solid var(--accent); outline-offset: -1px; }
  .paused .name { color: var(--text-dim); }
  .prov { display: flex; align-items: center; gap: 5px; min-width: 0; font-size: var(--fs-s); color: var(--text-dim); }
  .prov :global(svg) { flex-shrink: 0; }
  .prov-t { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Positioned + later in the DOM than .name::after, so they paint above it
     (and chips with a tooltip still get their hover). */
  .card-more, .card-actions, .card-meta [title] { position: relative; }
  .card-meta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; min-height: 20px; }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .card-actions { display: flex; gap: 6px; margin-top: auto; }
</style>
