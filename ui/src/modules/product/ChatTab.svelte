<script lang="ts">
  import { rel } from '../../lib/stores/now.svelte';
  // ChatTab — the story Chat (the former Chat and Refine tabs, merged), a list
  // / detail of its own (a nested ListDetail inside the story): the list pane
  // "Chats N · ＋ · ⋯" holds this story's conversations (refinement threads,
  // newest first; Archive… in the row menu) and, when the story has any, a
  // folded "Earlier chats" section from the old Discovery Chat, read-only.
  // The content is the open conversation (RefineChat) or an earlier chat
  // (EarlierChat). A phone pushes the chat ("‹ Chats").
  import { untrack } from 'svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { toastError } from '../../lib/toastError';
  import RefineChat from './RefineChat.svelte';
  import EarlierChat from './EarlierChat.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideSection from '../../lib/components/side/SideSection.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rowMenu } from '../../lib/rowMenu';
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
  let phoneOpen = $state(false);
  function pick(next: { kind: 'thread' | 'earlier'; id: string }): void {
    open = next;
    phoneOpen = true;
  }
  function rowItems(t: RefinementThread) {
    return t.status === 'active' ? [{ label: 'Archive…', icon: 'archive' as const, action: () => void archiveThread(t.id) }] : [];
  }
  function showRowMenu(e: MouseEvent, t: RefinementThread): void {
    const items = rowItems(t);
    if (!items.length) return;
    e.preventDefault();
    ctxMenu.show(e, items);
  }
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let earlierError = $state<string | null>(null);
  let creating = $state(false);
  // The empty rule: no conversation and no earlier chat → no list pane; the
  // start state below owns the page.
  const empty = $derived(!loading && !loadError && !earlierError && threads.length === 0 && earlier.length === 0);

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
      pick({ kind: 'thread', id: t.id });
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

{#snippet row(kind: 'thread' | 'earlier', id: string, title: string, status: string | null, updated: string, t: RefinementThread | null)}
  <!-- The shared row look (app.css .side-row): title + status word · when. -->
  <div class="side-row two-line chat-item" class:selected={isOpen(kind, id)} class:active={isOpen(kind, id)} class:archived={status === 'Archived'}>
    <button
      type="button"
      class="side-row-main chat-btn"
      data-side-row
      aria-current={isOpen(kind, id) ? 'true' : undefined}
      onclick={() => pick({ kind, id })}
      oncontextmenu={t ? (e) => showRowMenu(e, t) : undefined}
      use:rowMenu
    >
      <span class="side-row-text">
        <span class="side-row-title chat-title" dir="auto">{title}</span>
        <span class="side-row-meta"><span title={new Date(updated).toLocaleString()}>{status ? `${status} · ` : ''}{rel(updated)}</span></span>
      </span>
    </button>
    {#if t && t.status === 'active'}
      <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label="{title} actions" aria-haspopup="menu" title="More" onclick={(e) => showRowMenu(e, t)}>
        <Icon name="more" size={14} />
      </button>
    {/if}
  </div>
{/snippet}

<div class="chat-tab">
  <ListDetail module="product" pane="chats" name="Chats" open={phoneOpen && !!open} onback={() => (phoneOpen = false)} {empty}>
    {#snippet list()}
      <ListPane
        count={threads.length}
        primary={{ icon: 'plus', label: 'New chat', onclick: () => void createChat(), testid: 'product-new-chat' }}
        menu={() => [{ label: 'Refresh', icon: 'refresh', action: () => void load() }]}
        loading={loading && threads.length === 0 && earlier.length === 0}
        error={threads.length === 0 && earlier.length === 0 ? loadError : null}
        onretry={() => void load()}
        testid="product-chats"
      >
        {#if loadError && (threads.length > 0 || earlier.length > 0)}
          <LoadState what="chats" variant="compact" {loading} error={loadError} onretry={() => void load()} />
        {/if}
        <div class="chat-list">
          {#each threads as t (t.id)}
            {@render row('thread', t.id, t.title, t.status === 'archived' ? 'Archived' : null, t.updated_at, t)}
          {/each}
        </div>
        {#if earlier.length > 0 || earlierError}
          <SideSection title="Earlier chats" count={earlier.length || null} bind:open={showEarlier}>
            {#if earlierError}
              <LoadState what="earlier chats" variant="compact" error={earlierError} onretry={() => void load()} />
            {/if}
            <div class="chat-list">
              {#each earlier as ch (ch.id)}
                {@render row('earlier', ch.id, ch.title, 'Read-only', ch.updated_at, null)}
              {/each}
            </div>
          </SideSection>
        {/if}
      </ListPane>
    {/snippet}

    <!-- ── The open conversation, an earlier chat, or the start state ── -->
    <div class="chat-pane">
      {#if open?.kind === 'thread'}
        <RefineChat tid={open.id} />
      {:else if open?.kind === 'earlier'}
        {@const title = earlier.find((ch) => ch.id === open?.id)?.title ?? 'Discovery'}
        <EarlierChat cid={open.id} oncontinue={() => void createChat(`Continued: ${title}`)} />
      {:else if !loading}
        <EmptyState
          icon="comment"
          title={threads.length === 0 ? 'Figure out what to build' : 'No chat open'}
          body={threads.length === 0
            ? 'Start a chat and describe the rough idea. The agent researches, asks the right questions and proposes the story as a suggestion you review in Rewrite.'
            : 'Open a chat from the list, or start a new one.'}
          actionLabel="New chat"
          actionIcon="plus"
          actionKind={empty ? 'primary' : 'secondary'}
          onaction={() => void createChat()}
        />
      {/if}
    </div>
  </ListDetail>
</div>

<style>
  .chat-tab {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .chat-item.archived .side-row-title {
    color: var(--text-dim);
  }
  .chat-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
