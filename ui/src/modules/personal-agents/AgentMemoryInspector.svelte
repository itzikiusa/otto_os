<script lang="ts">
  // Memory inspector: what the agent has learned, item by item, with where it
  // learned it (chat / Slack / Telegram / vault / run). Edit or forget one
  // item; the raw notes file stays editable below (AgentDocuments).
  import { untrack } from 'svelte';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import type {
    PersonalAgentMemories,
    PersonalAgentMemoryItem,
    PersonalAgentMemorySource,
  } from '../../lib/api/types';

  interface Props {
    agentId: string;
    editable: boolean;
    /** Bumped by the parent after the raw notes are saved / the agent is reset. */
    reloadKey?: number;
  }
  let { agentId, editable, reloadKey = 0 }: Props = $props();

  const SOURCE_LABEL: Record<PersonalAgentMemorySource, string> = {
    chat: 'Chat',
    slack: 'Slack',
    telegram: 'Telegram',
    vault: 'Vault',
    run: 'Run',
    user: 'You',
    notes: 'Notes',
  };

  let data = $state<PersonalAgentMemories | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let filter = $state<PersonalAgentMemorySource | 'all'>('all');
  let editingLine = $state<number | null>(null);
  let draft = $state('');
  let busy = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      data = await personalAgentsApi.memories(agentId);
      error = null;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void agentId;
    void reloadKey;
    untrack(() => void load());
  });

  const sources = $derived(
    [...new Set((data?.items ?? []).map((i) => i.source))] as PersonalAgentMemorySource[],
  );
  const shown = $derived((data?.items ?? []).filter((i) => filter === 'all' || i.source === filter));

  async function submit(item: PersonalAgentMemoryItem, text: string | null): Promise<void> {
    if (!data) return;
    busy = true;
    try {
      data = await personalAgentsApi.editMemory(agentId, {
        version: data.version,
        line: item.line,
        raw: item.raw,
        text,
      });
      editingLine = null;
    } catch (e) {
      toasts.error(text === null ? 'Couldn’t forget that memory' : 'Couldn’t save that memory', loadErrorText(e));
      void load();
    } finally {
      busy = false;
    }
  }

  async function forget(item: PersonalAgentMemoryItem): Promise<void> {
    const ok = await confirmer.ask(`Forget “${item.text}”? The agent won’t see it in its next run or chat.`, {
      title: 'Forget memory',
      confirmLabel: 'Forget',
      danger: true,
    });
    if (ok) await submit(item, null);
  }
</script>

<section class="pa-panel" aria-labelledby="mem-h">
  <div class="card-head">
    <h2 id="mem-h">What it has learned</h2>
    <button class="icon-btn" aria-label="Refresh memories" title="Refresh memories" onclick={() => void load()}><Icon name="refresh" size={14} /></button>
  </div>
  <p class="hint">The same memory is used by every run and chat with this agent. Each item shows where it was learned.</p>
  <LoadState what="this agent’s memories" loading={loading && !data} error={data ? null : error} empty={(data?.items.length ?? 0) === 0} rows={3} onretry={() => void load()}>
    {#snippet emptyView()}
      <EmptyState icon="bulb" title="Nothing learned yet" body="Items appear here as the agent writes bullets to its memory during runs and chats." />
    {/snippet}
    {#if sources.length > 1}
      <div class="filters" role="group" aria-label="Filter by source">
        <button class="chip pa-filter" class:active={filter === 'all'} aria-pressed={filter === 'all'} onclick={() => (filter = 'all')}>All</button>
        {#each sources as s (s)}
          <button class="chip pa-filter" class:active={filter === s} aria-pressed={filter === s} onclick={() => (filter = s)}>{SOURCE_LABEL[s]}</button>
        {/each}
      </div>
    {/if}
    <ul class="list">
      {#each shown as item (item.line + ':' + item.raw)}
        <li class="item">
          <span class="chip src" title="Learned from">{SOURCE_LABEL[item.source]}</span>
          {#if editingLine === item.line}
            <input class="grow" bind:value={draft} aria-label="Memory text" disabled={busy}
              onkeydown={(e) => { if (e.key === 'Enter') void submit(item, draft); if (e.key === 'Escape') editingLine = null; }} />
            <button class="btn small" disabled={busy} onclick={() => (editingLine = null)}>Cancel</button>
            <button class="btn small primary" disabled={busy || !draft.trim()} onclick={() => void submit(item, draft)}>Save</button>
          {:else}
            <span class="grow text">{item.text}</span>
            {#if item.section}<span class="meta">{item.section}</span>{/if}
            {#if editable}
              <button class="icon-btn" aria-label="Edit memory" title="Edit" onclick={() => { editingLine = item.line; draft = item.text; }}><Icon name="edit" size={13} /></button>
              <button class="icon-btn" aria-label="Forget memory" title="Forget" onclick={() => void forget(item)}><Icon name="trash" size={13} /></button>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
  </LoadState>
</section>

<style>
  .pa-panel { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 12px 14px; color: var(--text); min-width: 0; margin-bottom: 12px; }
  .pa-panel h2 { margin: 0; font-size: var(--fs-l); font-weight: 600; }
  .card-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 4px; }
  .hint { color: var(--text-dim); font-size: var(--fs-s); margin: 0 0 8px; }
  .filters { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 8px; }
  .pa-filter { cursor: pointer; font: inherit; font-size: var(--fs-s); }
  .pa-filter.active { color: var(--accent-text); border-color: color-mix(in srgb, var(--accent) 45%, transparent); background: var(--accent-soft); }
  .pa-filter:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .item { display: flex; align-items: center; gap: 8px; padding: 6px 0; border-bottom: 1px solid var(--border); font-size: var(--fs-m); flex-wrap: wrap; }
  .item:last-child { border-bottom: 0; }
  .src { min-width: 7ch; justify-content: center; }
  .grow { flex: 1; min-width: 16ch; }
  .text { overflow-wrap: anywhere; }
  .item input {
    background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 4px 8px; font: inherit;
  }
  .item input:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
</style>
