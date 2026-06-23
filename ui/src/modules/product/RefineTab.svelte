<script lang="ts">
  // RefineTab — left: list of refinement threads with "New thread" / "New from
  // discovery run…" controls; right: RefineChat for the selected thread.
  // Mirrors DiscoveryTab for the list+detail layout and PlanTab for the
  // <select> idiom.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import RefineChat from './RefineChat.svelte';
  import type { RefinementThread, DiscoveryRunSummary } from './types';

  // ── State ──────────────────────────────────────────────────────────────────
  let threads = $state<RefinementThread[]>([]);
  let activeTid = $state<string | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  let creating = $state(false);

  // Discovery runs for the "New from discovery run…" picker
  let discoveryRuns = $state<DiscoveryRunSummary[]>([]);
  let selectedRunId = $state('');

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    void loadThreads();
    void loadDiscoveryRuns();
  });

  async function loadThreads(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      threads = await product.listRefinementThreads();
      // Auto-select the first active thread if nothing is selected yet.
      if (!activeTid && threads.length > 0) {
        const first = threads.find((t) => t.status === 'active') ?? threads[0];
        activeTid = first.id;
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function loadDiscoveryRuns(): Promise<void> {
    try {
      discoveryRuns = await product.listDiscoveryRuns();
    } catch {
      // Discovery runs are optional context — ignore errors here.
      discoveryRuns = [];
    }
  }

  // ── Create new thread ──────────────────────────────────────────────────────

  async function createThread(): Promise<void> {
    if (creating) return;
    creating = true;
    try {
      const newThread = await product.createRefinementThread({});
      threads = [newThread, ...threads];
      activeTid = newThread.id;
    } catch (e) {
      toasts.error('Could not create thread', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  async function createFromRun(): Promise<void> {
    if (!selectedRunId || creating) return;
    creating = true;
    try {
      const newThread = await product.createRefinementThread({ discovery_run_id: selectedRunId });
      threads = [newThread, ...threads];
      activeTid = newThread.id;
      selectedRunId = '';
    } catch (e) {
      toasts.error('Could not create thread', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  // ── Archive ────────────────────────────────────────────────────────────────

  async function archiveThread(tid: string): Promise<void> {
    try {
      const updated = await product.archiveRefinementThread(tid);
      threads = threads.map((t) => (t.id === tid ? updated : t));
      if (activeTid === tid) {
        // Switch away from an archived thread.
        const next = threads.find((t) => t.id !== tid && t.status === 'active');
        activeTid = next?.id ?? null;
      }
    } catch (e) {
      toasts.error('Could not archive thread', e instanceof Error ? e.message : String(e));
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

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

<div class="refine-tab">
  <!-- ── Left: thread list ──────────────────────────────────────────────── -->
  <aside class="thread-list-pane">
    <div class="pane-head">
      <span class="pane-title">Threads</span>
      <button
        class="toolbar-btn primary"
        onclick={createThread}
        disabled={creating}
        title="Start a new refinement conversation"
      >
        {creating ? 'Creating…' : '+ New thread'}
      </button>
    </div>

    <!-- "New from discovery run…" picker (only when runs exist) -->
    {#if discoveryRuns.length > 0}
      <div class="run-picker-row">
        <select
          class="picker"
          bind:value={selectedRunId}
          title="Seed the thread with a discovery run's findings"
        >
          <option value="">New from discovery run…</option>
          {#each discoveryRuns as dr (dr.run.id)}
            <option value={dr.run.id}>
              {relDate(dr.run.created_at)} — {dr.derived_status}
            </option>
          {/each}
        </select>
        <button
          class="toolbar-btn"
          onclick={createFromRun}
          disabled={creating || !selectedRunId}
          title="Create a thread seeded from the selected discovery run"
        >
          Create
        </button>
      </div>
    {/if}

    <!-- Thread list -->
    {#if loading && threads.length === 0}
      <div class="muted pad">Loading…</div>
    {:else if loadError}
      <div class="error-msg pad">Could not load threads: {loadError}</div>
    {:else if threads.length === 0}
      <div class="empty-state">
        <p>No threads yet.</p>
        <p>Click <strong>+ New thread</strong> to start chatting.</p>
      </div>
    {:else}
      <div class="thread-list">
        {#each threads as t (t.id)}
          <div
            class="thread-item"
            class:active={activeTid === t.id}
            class:archived={t.status === 'archived'}
          >
            <button
              class="thread-btn"
              onclick={() => (activeTid = t.id)}
            >
              <span class="thread-title">{t.title}</span>
              <span class="thread-meta">
                <span class="thread-status" class:status-archived={t.status === 'archived'}>
                  {t.status}
                </span>
                <span class="thread-date">{relDate(t.updated_at)}</span>
              </span>
            </button>
            {#if t.status === 'active'}
              <button
                class="archive-btn"
                onclick={() => archiveThread(t.id)}
                title="Archive this thread"
                aria-label="Archive thread"
              >
                Archive
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </aside>

  <!-- ── Right: chat or empty state ────────────────────────────────────── -->
  <div class="chat-pane">
    {#if activeTid}
      <RefineChat tid={activeTid} />
    {:else}
      <div class="chat-empty-state">
        <p>Select a thread on the left, or create a new one to start refining this story with the agent.</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .refine-tab {
    display: flex;
    height: 100%;
    min-height: 0;
    gap: 0;
  }

  /* ── Left pane ─────────────────────────────────────────────────────────── */
  .thread-list-pane {
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

  /* Discovery run picker */
  .run-picker-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .picker {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 11px;
    cursor: pointer;
  }

  /* Thread list */
  .thread-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 4px 0;
  }

  .thread-item {
    display: flex;
    align-items: center;
    border-radius: 0;
    transition: background 100ms;
    position: relative;
  }
  .thread-item:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .thread-item.active {
    background: color-mix(in srgb, var(--accent) 13%, transparent);
  }
  .thread-item.archived {
    opacity: 0.65;
  }

  .thread-btn {
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
  .thread-title {
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .thread-item.active .thread-title {
    color: var(--accent);
  }

  .thread-meta {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .thread-status {
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .thread-status.status-archived {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .thread-date {
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
  .thread-item:hover .archive-btn {
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
    max-width: 320px;
    margin: 0;
  }
</style>
