<script lang="ts">
  import { rel } from '../../lib/stores/now.svelte';
  // ChatTab — the story Chat (the former Chat and Refine tabs, merged).
  // Left: "New chat" + this story's conversations (refinement threads, newest
  // first, with an archive action) and, when the story has any, its "Earlier
  // chats" from the old Discovery Chat, read-only. Right: the open
  // conversation (RefineChat) or an earlier chat (EarlierChat).
  import { untrack } from 'svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { toastError } from '../../lib/toastError';
  import RefineChat from './RefineChat.svelte';
  import EarlierChat from './EarlierChat.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ListPane from './ui/ListPane.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { DiscoveryChat, RefinementThread } from './types';

  // ── State ──────────────────────────────────────────────────────────────────
  let threads = $state<RefinementThread[]>([]);
  let earlier = $state<DiscoveryChat[]>([]);
  /** The open item: a conversation, or an earlier (read-only) chat. */
  let open = $state<{ kind: 'thread' | 'earlier'; id: string } | null>(null);
  let showEarlier = $state(false);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let earlierError = $state<string | null>(null);
  let creating = $state(false);

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes; a conversation belongs to
    // ONE story, so drop the previous story's lists and selection.
    product.selectedId;
    untrack(() => {
      threads = [];
      earlier = [];
      open = null;
      showEarlier = false;
      creating = false;
    });
    void load();
  });

  async function load(): Promise<void> {
    const current = product.captureSelection();
    if (!product.selectedId) return;
    loading = true;
    loadError = null;
    earlierError = null;
    const [t, e] = await Promise.allSettled([product.listRefinementThreads(), product.listDiscoveryChats()]);
    if (!current()) return;
    loading = false;
    if (t.status === 'fulfilled') threads = t.value;
    else loadError = loadErrorText(t.reason);
    if (e.status === 'fulfilled') earlier = [...e.value].sort((a, b) => b.created_at.localeCompare(a.created_at));
    else earlierError = loadErrorText(e.reason);
    // Open on an item: the newest active conversation, else the newest one.
    if (!open && threads.length > 0) {
      const first = threads.find((x) => x.status === 'active') ?? threads[0];
      open = { kind: 'thread', id: first.id };
    }
  }

  // ── Create / archive ───────────────────────────────────────────────────────

  async function createChat(title?: string): Promise<void> {
    const current = product.captureSelection();
    if (creating) return;
    creating = true;
    try {
      const t = await product.createRefinementThread(title ? { title } : {});
      if (!current()) return;
      threads = [t, ...threads];
      open = { kind: 'thread', id: t.id };
    } catch (e) {
      if (!current()) return;
      toastError('Couldn’t start a chat', e);
    } finally {
      if (current()) creating = false;
    }
  }

  async function archiveThread(tid: string): Promise<void> {
    const current = product.captureSelection();
    const label = threads.find((t) => t.id === tid)?.title || 'Untitled';
    const ok = await confirmer.ask(
      `Archive chat “${label}”? It stays in this story’s list, marked archived.`,
      { title: 'Archive chat', confirmLabel: 'Archive', danger: false },
    );
    if (!ok || !current()) return;
    try {
      const updated = await product.archiveRefinementThread(tid);
      if (!current()) return;
      threads = threads.map((t) => (t.id === tid ? updated : t));
      if (open?.kind === 'thread' && open.id === tid) {
        const next = threads.find((t) => t.id !== tid && t.status === 'active');
        open = next ? { kind: 'thread', id: next.id } : null;
      }
    } catch (e) {
      if (!current()) return;
      toastError('Couldn’t archive chat', e);
    }
  }

  const isOpen = (kind: 'thread' | 'earlier', id: string) => open?.kind === kind && open.id === id;
</script>

<div class="chat-tab">
  <!-- ── Left: conversations + earlier chats ────────────────────────────── -->
  <ListPane title="Chats">
    {#snippet actions()}
      <button class="btn small primary" onclick={() => void createChat()} disabled={creating} title="Start a new conversation on this story">
        <Icon name="plus" size={12} /> {creating ? 'Starting…' : 'New chat'}
      </button>
    {/snippet}
    {#snippet children()}
      {#if (loading || loadError) && threads.length === 0}
        <LoadState what="chats" variant="compact" {loading} error={loadError} empty onretry={() => void load()} />
      {:else}
        {#if loadError}
          <LoadState what="chats" variant="compact" {loading} error={loadError} onretry={() => void load()} />
        {/if}
        {#if threads.length === 0}
          <p class="empty-rail-hint">No chats yet — use New chat above to start one.</p>
        {/if}
        <div class="chat-list">
          {#each threads as t (t.id)}
            <div class="chat-item" class:active={isOpen('thread', t.id)} class:archived={t.status === 'archived'}>
              <button class="chat-btn" aria-current={isOpen('thread', t.id) ? 'true' : undefined} onclick={() => (open = { kind: 'thread', id: t.id })}>
                <span class="chat-title">{t.title}</span>
                <span class="chat-meta">
                  {#if t.status === 'archived'}<span class="chat-status">Archived</span>{/if}
                  <span class="chat-date" title={new Date(t.updated_at).toLocaleString()}>{rel(t.updated_at)}</span>
                </span>
              </button>
              {#if t.status === 'active'}
                <button class="archive-btn reveal-on-hover" onclick={() => void archiveThread(t.id)} title="Archive this chat">Archive</button>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if earlier.length > 0 || earlierError}
        <div class="earlier">
          <button class="earlier-toggle" type="button" aria-expanded={showEarlier} onclick={() => (showEarlier = !showEarlier)}>
            <Icon name={showEarlier ? 'fold' : 'unfold'} size={12} />
            Earlier chats{earlier.length ? ` (${earlier.length})` : ''}
          </button>
          {#if showEarlier}
            {#if earlierError}
              <LoadState what="earlier chats" variant="compact" error={earlierError} onretry={() => void load()} />
            {/if}
            <div class="chat-list">
              {#each earlier as c (c.id)}
                <div class="chat-item" class:active={isOpen('earlier', c.id)}>
                  <button class="chat-btn" aria-current={isOpen('earlier', c.id) ? 'true' : undefined} onclick={() => (open = { kind: 'earlier', id: c.id })}>
                    <span class="chat-title">{c.title}</span>
                    <span class="chat-meta">
                      <span class="chat-status">Read-only</span>
                      <span class="chat-date" title={new Date(c.updated_at).toLocaleString()}>{rel(c.updated_at)}</span>
                    </span>
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {/snippet}
  </ListPane>

  <!-- ── Right: the open conversation, an earlier chat, or the start state ── -->
  <div class="chat-pane">
    {#if open?.kind === 'thread'}
      <RefineChat tid={open.id} />
    {:else if open?.kind === 'earlier'}
      {@const title = earlier.find((c) => c.id === open?.id)?.title ?? 'Discovery'}
      <EarlierChat cid={open.id} oncontinue={() => void createChat(`Continued: ${title}`)} />
    {:else if !loading}
      <EmptyState
        icon="comment"
        title={threads.length === 0 ? 'Figure out what to build' : 'No chat open'}
        body={threads.length === 0
          ? 'Start a chat and describe the rough idea. The agent researches, asks the right questions and proposes the story as a suggestion you review in Rewrite.'
          : 'Open a chat on the left, or start a new one.'}
        actionLabel="New chat"
        actionIcon="plus"
        actionKind="secondary"
        onaction={() => void createChat()}
      />
    {/if}
  </div>
</div>

<style>
  .chat-tab {
    display: flex;
    height: 100%;
    min-height: 0;
    gap: 0;
  }

  .empty-rail-hint {
    margin: 0;
    padding: 12px 10px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  /* ── Left pane — chrome (width, border, header, scroll) is ListPane's job;
     only the row markup below is ChatTab-specific. ───────────────────────── */
  .chat-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 4px 0;
  }

  .chat-item {
    display: flex;
    align-items: center;
    transition: background var(--dur-fast);
    position: relative;
  }
  .chat-item:hover {
    background: var(--hover);
  }
  .chat-item.active {
    background: var(--accent-soft);
  }
  .chat-item.archived {
    opacity: 0.65;
  }

  .chat-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .chat-title {
    font-size: var(--fs-s);
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .chat-item.active .chat-title {
    color: var(--accent-text);
  }

  .chat-meta {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .chat-status {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    padding: 1px 4px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .chat-date {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  .archive-btn {
    flex-shrink: 0;
    margin-inline-end: 8px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--dur-fast), background var(--dur-fast);
    white-space: nowrap;
  }
  /* Revealed on row hover / keyboard focus, and always shown on a touch
     screen (no hover there) — the shared .reveal-on-hover recipe. */
  .chat-item:hover .archive-btn,
  .chat-item:focus-within .archive-btn,
  .chat-item.active .archive-btn,
  .archive-btn:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .archive-btn {
      opacity: 1;
    }
  }
  .archive-btn:hover {
    background: var(--hover);
    color: var(--text);
  }

  /* ── Earlier chats (read-only history) ──────────────────────────────────── */
  .earlier {
    border-top: 1px solid var(--border);
    margin-top: 4px;
  }
  .earlier-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 8px 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    cursor: pointer;
    text-align: start;
  }
  .earlier-toggle:hover {
    color: var(--text);
  }

  /* ── Right pane ─────────────────────────────────────────────────────────── */
  .chat-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  @media (max-width: 640px) {
    .chat-tab {
      flex-direction: column;
    }
  }
  @media (pointer: coarse), (max-width: 640px) {
    .archive-btn,
    .earlier-toggle {
      min-height: var(--hit-min);
    }
    .archive-btn {
      padding: 6px 10px;
      opacity: 1;
    }
  }
</style>
