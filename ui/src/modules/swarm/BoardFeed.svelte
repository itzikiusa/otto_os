<script lang="ts">
  // The shared surface: a live feed of agent + user board posts, with a composer.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { MessageKind } from './types';

  let kindFilter = $state<string>('');
  let draft = $state('');
  let draftKind = $state<MessageKind>('message');

  const KINDS: MessageKind[] = [
    'message',
    'idea',
    'review_request',
    'review',
    'decision',
    'status',
    'concern',
    'escalation',
    'handoff',
    'system',
  ];

  const filtered = $derived(
    kindFilter ? swarm.board.filter((m) => m.kind === kindFilter) : swarm.board,
  );

  function author(m: { author_agent_id?: string | null; author_user_id?: string | null }): string {
    if (m.author_agent_id) return swarm.agentById(m.author_agent_id)?.name ?? 'agent';
    if (m.author_user_id) return 'you';
    return 'system';
  }

  const KIND_CLASS: Record<string, string> = {
    concern: 'bad',
    escalation: 'bad',
    decision: 'accent',
    review: 'accent',
    idea: 'ok',
  };

  async function post() {
    if (!draft.trim()) return;
    await swarm.postBoard({ body: draft.trim(), kind: draftKind, project_id: swarm.selectedProjectId ?? undefined });
    draft = '';
  }
</script>

<div class="board">
  <div class="b-filters">
    <button class="chip" class:accent={kindFilter === ''} onclick={() => (kindFilter = '')}>all</button>
    {#each ['idea', 'review', 'decision', 'concern', 'status'] as k (k)}
      <button class="chip" class:accent={kindFilter === k} onclick={() => (kindFilter = k)}>{k}</button>
    {/each}
    <span class="grow"></span>
    <button class="icon-btn" onclick={() => swarm.loadBoard()} aria-label="refresh"><Icon name="refresh" size={14} /></button>
  </div>

  <div class="feed">
    {#if filtered.length === 0}
      <EmptyState icon="comment" title="Quiet board" body="Agents post ideas, reviews and decisions here as they work." />
    {/if}
    {#each filtered as m (m.id)}
      <div class="msg">
        <div class="msg-head">
          <span class="chip {KIND_CLASS[m.kind] ?? ''}">{m.kind}</span>
          <span class="who">{author(m)}</span>
          {#if m.to_agent_id}<span class="dim">→ {swarm.agentById(m.to_agent_id)?.name ?? 'agent'}</span>{/if}
          <span class="grow"></span>
          <span class="dim time">{rel(m.created_at)}</span>
        </div>
        <div class="msg-body">{m.body}</div>
      </div>
    {/each}
  </div>

  <div class="composer">
    <select class="input small" bind:value={draftKind}>
      {#each KINDS as k (k)}<option value={k}>{k}</option>{/each}
    </select>
    <input
      class="input grow"
      placeholder="Post to the team board…"
      bind:value={draft}
      onkeydown={(e) => e.key === 'Enter' && post()}
    />
    <button class="btn small primary" onclick={post}>Post</button>
  </div>
</div>

<style>
  .board {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .b-filters {
    display: flex;
    gap: 4px;
    align-items: center;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .chip {
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
  }
  .feed {
    flex: 1;
    overflow-y: auto;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .msg {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    background: var(--surface);
  }
  .msg-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    margin-bottom: 4px;
  }
  .who {
    font-weight: 600;
  }
  .time {
    font-size: 10.5px;
  }
  .msg-body {
    font-size: 12.5px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .composer {
    display: flex;
    gap: 8px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }
</style>
