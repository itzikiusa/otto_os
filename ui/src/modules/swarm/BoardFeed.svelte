<script lang="ts">
  // The shared surface: a live feed of agent + user board posts, with a composer.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { sentenceCase } from '../../lib/status';
  import { toasts } from '../../lib/toast.svelte';
  import type { MessageKind } from './types';

  let kindFilter = $state<string>('');
  let draft = $state('');
  let draftKind = $state<MessageKind>('message');
  // Target a specific agent ('' = the whole team). Lets you tag someone and tell
  // them what to do, or reply to an agent's post.
  let draftTo = $state<string>('');
  let composerEl = $state<HTMLInputElement | null>(null);

  const agents = $derived(swarm.detail?.agents ?? []);

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
    // Coordinator lifecycle posts.
    'worktree',
    'shared',
    'merge',
    'verify',
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
    worktree: 'ok',
    shared: 'bad',
    merge: 'ok',
    verify: 'accent',
  };

  // Emoji glyphs for the Coordinator lifecycle kinds (distinct at a glance).
  const KIND_ICON: Record<string, string> = {
    worktree: '🌿',
    shared: '⚠️',
    merge: '✅',
    verify: '🔎',
    escalation: '🚫',
  };

  let posting = $state(false);
  async function post() {
    if (!draft.trim() || posting) return;
    posting = true;
    try {
      await swarm.postBoard({
        body: draft.trim(),
        kind: draftKind,
        project_id: swarm.selectedProjectId ?? undefined,
        to_agent_id: draftTo || undefined,
      });
      draft = '';
    } catch (e) {
      // Keep the draft so nothing typed is lost; say why it didn't post.
      toasts.error("Couldn't post to the board", e instanceof Error ? e.message : String(e));
    } finally {
      posting = false;
    }
  }

  // Reply to a message: target its author and focus the composer.
  function reply(m: { author_agent_id?: string | null }) {
    draftTo = m.author_agent_id ?? '';
    draftKind = 'message';
    composerEl?.focus();
  }
</script>

<div class="board">
  <div class="b-filters">
    <button class="chip" class:accent={kindFilter === ''} onclick={() => (kindFilter = '')}>All</button>
    {#each ['idea', 'review', 'decision', 'concern', 'status', 'worktree', 'shared', 'merge', 'verify', 'escalation'] as k (k)}
      <button class="chip" class:accent={kindFilter === k} onclick={() => (kindFilter = k)}>
        {KIND_ICON[k] ?? ''} {sentenceCase(k)}
      </button>
    {/each}
    <span class="grow"></span>
    <button class="icon-btn" onclick={() => swarm.loadBoard()} aria-label="Refresh board" title="Refresh board"><Icon name="refresh" size={14} /></button>
  </div>

  <div class="feed">
    {#if filtered.length === 0}
      {#if kindFilter}
        <EmptyState icon="comment" title="No {sentenceCase(kindFilter).toLowerCase()} posts" body="Nothing of this kind on the board yet — pick All to see every post." />
      {:else}
        <EmptyState icon="comment" title="Quiet board" body="Agents post ideas, reviews and decisions here as they work." />
      {/if}
    {/if}
    {#each filtered as m (m.id)}
      <div class="msg">
        <div class="msg-head">
          <span class="chip {KIND_CLASS[m.kind] ?? ''}">{KIND_ICON[m.kind] ?? ''} {sentenceCase(m.kind)}</span>
          <span class="who">{author(m)}</span>
          {#if m.to_agent_id}<span class="dim">→ {swarm.agentById(m.to_agent_id)?.name ?? 'agent'}</span>{/if}
          <span class="grow"></span>
          {#if m.author_agent_id}
            <button class="reply-btn" onclick={() => reply(m)} title="Reply to {author(m)}">Reply</button>
          {/if}
          <span class="dim time">{rel(m.created_at)}</span>
        </div>
        <div class="msg-body">{m.body}</div>
      </div>
    {/each}
  </div>

  <div class="composer">
    <select class="input small" bind:value={draftTo} title="Who is this for?">
      <option value="">— team —</option>
      {#each agents as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
    </select>
    <select class="input small" bind:value={draftKind} title="Kind of post">
      {#each KINDS as k (k)}<option value={k}>{sentenceCase(k)}</option>{/each}
    </select>
    <input
      bind:this={composerEl}
      class="input grow"
      placeholder={draftTo ? `Tell ${swarm.agentById(draftTo)?.name ?? 'them'} what to do…` : 'Post to the team board…'}
      bind:value={draft}
      onkeydown={(e) => e.key === 'Enter' && post()}
    />
    <button class="btn small primary" onclick={post} disabled={!draft.trim() || posting} title={draft.trim() ? undefined : 'Type a message to post'}>
      {posting ? 'Posting…' : 'Post'}
    </button>
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
    font-size: var(--fs-xs);
  }
  .reply-btn {
    border: none;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
    font-size: 11px;
    padding: 0 4px;
  }
  .reply-btn:hover {
    text-decoration: underline;
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
