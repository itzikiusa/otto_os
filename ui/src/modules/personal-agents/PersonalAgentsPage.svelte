<script lang="ts">
  // Personal Agents module. Routes: `#/personal-agents` (agent cards),
  // `#/personal-agents/rooms` (agent rooms), `#/personal-agents/<agentId>`
  // (one agent's page). The first list GET seeds four disabled example agents
  // server-side — they render as normal rows.
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import AgentEditSheet from './AgentEditSheet.svelte';
  import AgentPage from './AgentPage.svelte';
  import RoomsView from './RoomsView.svelte';
  import type { PersonalAgent } from '../../lib/api/types';

  const sub = $derived(router.parts[1] ?? '');
  const agentId = $derived(sub && sub !== 'rooms' ? sub : null);

  let creating = $state(false);
  let error = $state('');

  $effect(() => {
    if (ws.currentId) void personalAgents.loadAgents(ws.currentId);
  });

  const agents = $derived(personalAgents.agents);

  async function toggle(a: PersonalAgent): Promise<void> {
    try {
      await personalAgents.setEnabled(a.id, !a.enabled);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Toggle failed';
    }
  }

  async function runNow(a: PersonalAgent): Promise<void> {
    error = '';
    try {
      await personalAgents.runNow(a.id);
      router.go(`personal-agents/${a.id}`);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Run failed';
    }
  }

  async function remove(a: PersonalAgent): Promise<void> {
    if (!confirm(`Delete personal agent "${a.name}"? Its schedules and run history go with it.`)) return;
    try {
      await personalAgents.remove(a.id);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Delete failed';
    }
  }

  function cardMenu(e: MouseEvent | KeyboardEvent, a: PersonalAgent): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'user', action: () => router.go(`personal-agents/${a.id}`) },
      { label: 'Run now', icon: 'play', action: () => void runNow(a) },
      { label: a.enabled ? 'Pause' : 'Enable', icon: a.enabled ? 'clock' : 'play', action: () => void toggle(a) },
      { separator: true },
      { label: 'Delete', icon: 'trash', danger: true, action: () => void remove(a) },
    ]);
  }

  /** A seeded example: disabled, never run, still carrying its shipped persona. */
  function isExample(a: PersonalAgent): boolean {
    return !a.enabled && !a.chat_session_id && (personalAgents.runsByAgent[a.id]?.length ?? 0) === 0;
  }
</script>

{#if agentId}
  {#key agentId}
    <AgentPage {agentId} />
  {/key}
{:else}
  <div class="pa">
    <header class="head">
      <div>
        <h1>Personal Agents</h1>
        <p class="sub">
          Named personas with a pinned provider + model, their own schedules, memory, and delivery —
          chat with them anytime, and let them talk to each other in rooms you can always read.
        </p>
      </div>
      <div class="head-actions">
        <button class="btn" class:primary={sub === 'rooms'} onclick={() => router.go(sub === 'rooms' ? 'personal-agents' : 'personal-agents/rooms')}>
          {sub === 'rooms' ? 'Agents' : 'Rooms'}
        </button>
        <button class="btn primary" onclick={() => (creating = true)}>New agent</button>
      </div>
    </header>

    {#if error}<div class="err" role="alert">{error}</div>{/if}

    {#if sub === 'rooms'}
      <RoomsView />
    {:else if agents.length === 0}
      {#if personalAgents.loadingAgents}
        <div class="muted">Loading…</div>
      {:else}
        <EmptyState
          icon="user"
          title="No personal agents"
          body="Create a named agent with its own persona, schedules and memory."
          actionLabel="New agent"
          onaction={() => (creating = true)}
        />
      {/if}
    {:else}
      <ul class="cards">
        {#each agents as a (a.id)}
          <li>
            <div
              class="card"
              class:paused={!a.enabled}
              role="button"
              tabindex="0"
              onclick={() => router.go(`personal-agents/${a.id}`)}
              onkeydown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  router.go(`personal-agents/${a.id}`);
                }
              }}
              oncontextmenu={(e) => cardMenu(e, a)}
            >
              <div class="card-top">
                <span class="avatar" aria-hidden="true">{a.avatar || a.name.slice(0, 1)}</span>
                <div class="card-id">
                  <strong class="name">{a.name}</strong>
                  <span class="chip mono">{a.provider}{a.model ? ` · ${a.model}` : ''}</span>
                </div>
              </div>
              <div class="card-meta">
                {#if !a.enabled}
                  <span class="pill">{isExample(a) ? 'example — enable to use' : 'paused'}</span>
                {:else}
                  <span class="pill ok">enabled</span>
                {/if}
                {#if a.browser}<span class="pill">browser</span>{/if}
                <span class="meta">next run {personalAgents.nextRunAt(a.id) ?? '—'}</span>
              </div>
              <div class="card-actions">
                <button class="btn small" onclick={(e) => { e.stopPropagation(); void runNow(a); }}>Run now</button>
                <button class="btn small" onclick={(e) => { e.stopPropagation(); void toggle(a); }}>
                  {a.enabled ? 'Pause' : 'Enable'}
                </button>
              </div>
            </div>
          </li>
        {/each}
      </ul>
    {/if}

    {#if creating}
      <AgentEditSheet agent={null} onclose={() => (creating = false)} />
    {/if}
  </div>
{/if}

<style>
  .pa { padding: 1rem 1.25rem; max-width: 1080px; margin: 0 auto; display: flex; flex-direction: column; min-height: 0; height: 100%; box-sizing: border-box; }
  .head { display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; margin-bottom: 0.75rem; flex-wrap: wrap; }
  .head h1 { margin: 0; font-size: 1.25rem; color: var(--text); }
  .sub { margin: 0.25rem 0 0; color: var(--text-dim); font-size: 0.85rem; max-width: 64ch; }
  .head-actions { display: flex; gap: 0.5rem; }
  .muted { color: var(--text-dim); padding: 0.75rem 0; font-size: 0.9rem; }
  .err {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    color: var(--status-exited); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); margin-bottom: 0.75rem; font-size: 0.85rem;
  }
  .cards { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 0.6rem; }
  .card {
    border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m);
    padding: 0.7rem 0.8rem; color: var(--text); cursor: pointer; display: flex; flex-direction: column; gap: 0.5rem;
    height: 100%; box-sizing: border-box;
  }
  .card:hover { border-color: color-mix(in srgb, var(--accent) 45%, var(--border)); }
  .card:focus-visible { outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent); outline-offset: 1px; }
  .card.paused { opacity: 0.75; }
  .card-top { display: flex; align-items: center; gap: 0.6rem; }
  .avatar {
    width: 2.2rem; height: 2.2rem; flex: 0 0 auto; border-radius: var(--radius-m); display: inline-flex;
    align-items: center; justify-content: center; font-size: 1.2rem;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .card-id { min-width: 0; display: flex; flex-direction: column; gap: 0.2rem; }
  .name { font-size: 0.95rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chip { font-size: 0.72rem; padding: 0.05rem 0.45rem; border-radius: 999px; border: 1px solid var(--border); color: var(--text-dim); align-self: flex-start; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mono { font-family: var(--font-mono); }
  .card-meta { display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap; }
  .meta { color: var(--text-dim); font-size: 0.75rem; }
  .pill { font-size: 0.7rem; padding: 0.05rem 0.45rem; border-radius: 999px; border: 1px solid var(--border); color: var(--text-dim); }
  .pill.ok { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent); border-color: transparent; }
  .card-actions { display: flex; gap: 0.35rem; margin-top: auto; }
</style>
