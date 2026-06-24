<script lang="ts">
  // ChatTab — Discovery Chat for the current story.
  // Left: "New chat" + a list of this story's DiscoveryChats (newest first, with
  // an archive action). Right: the active DiscoveryChat pane.
  // Layout mirrors RefineTab (220px sidebar + chat pane); the conversation works
  // from an EMPTY/Untitled draft to help with early discovery & research.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import DiscoveryChat from './DiscoveryChat.svelte';
  import type { DiscoveryChat as DiscoveryChatT } from './types';

  // ── State ──────────────────────────────────────────────────────────────────
  let chats = $state<DiscoveryChatT[]>([]);
  let activeCid = $state<string | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  let creating = $state(false);

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    void loadChats();
  });

  async function loadChats(): Promise<void> {
    // No story selected → nothing to load (avoid a "No story selected" error from
    // the store's story-scoped call).
    if (!product.selectedId) {
      chats = [];
      activeCid = null;
      return;
    }
    loading = true;
    loadError = null;
    // Drop any selection carried over from a previously-open story; it belongs to
    // a different story and would make DiscoveryChat fetch the wrong chat.
    activeCid = null;
    try {
      const list = await product.listDiscoveryChats();
      // Newest first.
      chats = [...list].sort((a, b) => b.created_at.localeCompare(a.created_at));
      // Auto-select the first active chat if any.
      if (chats.length > 0) {
        const first = chats.find((c) => c.status === 'active') ?? chats[0];
        activeCid = first.id;
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // ── Create new chat ─────────────────────────────────────────────────────────

  async function createChat(): Promise<void> {
    if (creating) return;
    creating = true;
    try {
      const newChat = await product.createDiscoveryChat({});
      chats = [newChat, ...chats];
      activeCid = newChat.id;
    } catch (e) {
      toasts.error('Could not create chat', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  // ── Archive ──────────────────────────────────────────────────────────────────

  async function archiveChat(cid: string): Promise<void> {
    try {
      const updated = await product.archiveDiscoveryChat(cid);
      chats = chats.map((c) => (c.id === cid ? updated : c));
      if (activeCid === cid) {
        // Switch away from an archived chat.
        const next = chats.find((c) => c.id !== cid && c.status === 'active');
        activeCid = next?.id ?? null;
      }
    } catch (e) {
      toasts.error('Could not archive chat', e instanceof Error ? e.message : String(e));
    }
  }

  // ── Helpers ──────────────────────────────────────────────────────────────────

  function relDate(iso: string): string {
    try {
      const diff = Date.now() - new Date(iso).getTime();
      const s = Math.floor(diff / 1000);
      if (s < 60) return 'just now';
      const m = Math.floor(s / 60);
      if (m < 60) return `${m}m ago`;
      const h = Math.floor(m / 60);
      if (h < 24) return `${h}h ago`;
      const d = Math.floor(h / 24);
      if (d < 30) return `${d}d ago`;
      return new Date(iso).toLocaleDateString();
    } catch {
      return iso;
    }
  }
</script>

<div class="chat-tab">
  <!-- ── Left: chat list ────────────────────────────────────────────────── -->
  <aside class="chat-list-pane">
    <div class="pane-head">
      <span class="pane-title">Chats</span>
      <button
        class="toolbar-btn primary"
        onclick={createChat}
        disabled={creating}
        title="Start a new discovery chat"
      >
        {creating ? 'Creating…' : '+ New chat'}
      </button>
    </div>

    {#if loading && chats.length === 0}
      <div class="muted pad">Loading…</div>
    {:else if loadError}
      <div class="error-msg pad">Could not load chats: {loadError}</div>
    {:else if chats.length === 0}
      <div class="empty-state">
        <p>No chats yet.</p>
        <p>Click <strong>+ New chat</strong> to start figuring out what to build.</p>
      </div>
    {:else}
      <div class="chat-list">
        {#each chats as c (c.id)}
          <div
            class="chat-item"
            class:active={activeCid === c.id}
            class:archived={c.status === 'archived'}
          >
            <button class="chat-btn" onclick={() => (activeCid = c.id)}>
              <span class="chat-title">{c.title}</span>
              <span class="chat-meta">
                <span class="chat-status" class:status-archived={c.status === 'archived'}>
                  {c.status}
                </span>
                <span class="chat-date">{relDate(c.updated_at)}</span>
              </span>
            </button>
            {#if c.status === 'active'}
              <button
                class="archive-btn"
                onclick={() => archiveChat(c.id)}
                title="Archive this chat"
                aria-label="Archive chat"
              >
                Archive
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </aside>

  <!-- ── Right: chat or empty state ───────────────────────────────────────── -->
  <div class="chat-pane">
    {#if activeCid}
      <DiscoveryChat cid={activeCid} />
    {:else}
      <div class="chat-empty-state">
        <p>Start a new chat to figure out what to build — describe the rough idea and the agent will research, ask the right questions, and turn it into a story.</p>
      </div>
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

  /* ── Left pane ─────────────────────────────────────────────────────────── */
  .chat-list-pane {
    width: 220px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .pane-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 8px 10px 6px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .pane-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }

  .toolbar-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms;
  }
  .toolbar-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .toolbar-btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .toolbar-btn.primary:hover:not(:disabled) {
    opacity: 0.88;
  }
  .toolbar-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Chat list */
  .chat-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 4px 0;
  }

  .chat-item {
    display: flex;
    align-items: center;
    border-radius: 0;
    transition: background 100ms;
    position: relative;
  }
  .chat-item:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .chat-item.active {
    background: color-mix(in srgb, var(--accent) 13%, transparent);
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
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .chat-item.active .chat-title {
    color: var(--accent);
  }

  .chat-meta {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .chat-status {
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .chat-status.status-archived {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .chat-date {
    font-size: 10.5px;
    color: var(--text-dim);
  }

  .archive-btn {
    flex-shrink: 0;
    margin-inline-end: 8px;
    padding: 2px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 10.5px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 100ms, background 100ms;
    white-space: nowrap;
  }
  .chat-item:hover .archive-btn {
    opacity: 1;
  }
  .archive-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }

  /* States */
  .muted {
    color: var(--text-dim);
    font-size: 13px;
    font-style: italic;
  }
  .pad {
    padding: 12px 10px;
  }
  .error-msg {
    color: #ef4444;
    font-size: 12px;
    padding: 8px 10px;
  }
  .empty-state {
    padding: 24px 12px;
    text-align: center;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.6;
  }
  .empty-state p {
    margin: 4px 0;
  }

  /* ── Right pane ─────────────────────────────────────────────────────────── */
  .chat-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .chat-empty-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 32px 24px;
    text-align: center;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.6;
  }
  .chat-empty-state p {
    max-width: 360px;
    margin: 0;
  }
</style>
