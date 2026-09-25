<script lang="ts">
  // The shared surface: a live feed of agent + user board posts, with a composer.
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
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

  // Colour carries meaning only: a concern / shared-file clash wants a look
  // (warning), an escalation is blocked (danger), a merge landed (success).
  // Every other kind is neutral — the icon + word tell them apart.
  const KIND_TONE: Record<string, 'warn' | 'bad' | 'ok'> = {
    concern: 'warn',
    shared: 'warn',
    escalation: 'bad',
    merge: 'ok',
  };

  const KIND_ICON: Record<string, IconName> = {
    message: 'comment',
    idea: 'bulb',
    review_request: 'eye',
    review: 'eye',
    decision: 'check',
    status: 'info',
    concern: 'warning',
    escalation: 'warning',
    handoff: 'send',
    system: 'gear',
    worktree: 'worktree',
    shared: 'warning',
    merge: 'merge',
    verify: 'search',
  };
  const FILTER_KINDS = ['idea', 'review', 'decision', 'concern', 'status', 'worktree', 'shared', 'merge', 'verify', 'escalation'];

  let posting = $state(false);
  async function post() {
    if (!draft.trim() || posting) return;
    const submitted = draft;
    posting = true;
    try {
      await swarm.postBoard({
        body: submitted.trim(),
        kind: draftKind,
        project_id: swarm.selectedProjectId ?? undefined,
        to_agent_id: draftTo || undefined,
      });
      if (draft === submitted) draft = '';
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
  <div class="b-filters" role="group" aria-label="Filter posts by kind">
    <button class="chip" class:accent={kindFilter === ''} aria-pressed={kindFilter === ''} onclick={() => (kindFilter = '')}>All</button>
    {#each FILTER_KINDS as k (k)}
      <button class="chip" class:accent={kindFilter === k} aria-pressed={kindFilter === k} onclick={() => (kindFilter = k)}>
        {#if KIND_ICON[k]}<Icon name={KIND_ICON[k]} size={12} />{/if} {sentenceCase(k)}
      </button>
    {/each}
    <span class="grow"></span>
    <button class="icon-btn" onclick={() => swarm.loadBoard()} aria-label="Refresh board" title="Refresh board"><Icon name="refresh" size={14} /></button>
  </div>

  <div class="feed">
    <LoadState what="board posts" loading={swarm.boardLoading} error={swarm.boardError}
      empty={swarm.board.length === 0} onretry={() => void swarm.loadBoard()}>
      {#snippet emptyView()}
        <EmptyState icon="comment" title="Quiet board" body="Agents post ideas, reviews and decisions here as they work." />
      {/snippet}
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
          <span class="chip kind-chip tone-{KIND_TONE[m.kind] ?? 'neutral'}">{#if KIND_ICON[m.kind]}<Icon name={KIND_ICON[m.kind]} size={12} />{/if} {sentenceCase(m.kind)}</span>
          <span class="who">{author(m)}</span>
          {#if m.to_agent_id}<span class="dim to">to {swarm.agentById(m.to_agent_id)?.name ?? 'an agent'}</span>{/if}
          <span class="grow"></span>
          {#if m.author_agent_id}
            <button class="btn small ghost" onclick={() => reply(m)} title="Reply to {author(m)}">Reply</button>
          {/if}
          <span class="dim time" title={new Date(m.created_at).toLocaleString()}>{rel(m.created_at)}</span>
        </div>
        <div class="msg-body">{m.body}</div>
      </div>
    {/each}
    </LoadState>
  </div>

  <div class="composer">
    <select class="input small" bind:value={draftTo} aria-label="Post to" title="Who is this for?">
      <option value="">Whole team</option>
      {#each agents as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
    </select>
    <select class="input small" bind:value={draftKind} aria-label="Kind of post" title="Kind of post">
      {#each KINDS as k (k)}<option value={k}>{sentenceCase(k)}</option>{/each}
    </select>
    <input
      bind:this={composerEl}
      class="input grow"
      aria-label="Message"
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
    container-type: inline-size;
  }
  .b-filters {
    display: flex;
    gap: 4px;
    align-items: center;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    flex-wrap: wrap;
  }
  button.chip {
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
  }
  /* Kind chips: neutral by default; a tone only where the kind means one. */
  .kind-chip {
    gap: 4px;
  }
  .tone-warn {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: transparent;
  }
  .tone-bad {
    color: var(--danger);
    background: var(--danger-soft);
    border-color: transparent;
  }
  .tone-ok {
    color: var(--success);
    background: var(--success-soft);
    border-color: transparent;
  }
  .feed {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .msg {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 12px;
    background: var(--surface);
  }
  .msg-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    margin-block-end: 4px;
    min-height: 22px;
  }
  .who {
    font-weight: 600;
  }
  .time {
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .msg-body {
    font-size: var(--fs-m);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .composer {
    flex: none;
    display: flex;
    gap: 8px;
    padding: 8px 12px;
    border-block-start: 1px solid var(--border);
  }
  @container (max-width: 640px) {
    .composer { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto; }
    .composer select { min-width: 0; width: 100%; }
    .composer input { grid-column: 1 / -1; grid-row: 1; width: 100%; min-width: 0; }
    .composer button { min-width: 48px; }
  }
</style>
