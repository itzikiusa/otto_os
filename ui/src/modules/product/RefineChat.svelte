<script lang="ts">
  // RefineChat — displays a single refinement thread transcript and handles
  // sending new messages to the agent.  Props: { tid } (the thread id).
  // Parent (RefineTab) controls which thread is active.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdown } from '../../lib/md';
  import type { RefinementMessage } from './types';

  let { tid }: { tid: string } = $props();

  // ── Local state ────────────────────────────────────────────────────────────
  let messages = $state<RefinementMessage[]>([]);
  let loadError = $state<string | null>(null);
  let loading = $state(false);

  let inputText = $state('');
  let sending = $state(false);

  // ── Load / reload when tid changes ────────────────────────────────────────
  $effect(() => {
    // Reactive on tid — re-runs whenever the active thread switches.
    const currentTid = tid;
    void loadThread(currentTid);
  });

  async function loadThread(threadId: string): Promise<void> {
    loading = true;
    loadError = null;
    try {
      const detail = await product.getRefinementThread(threadId);
      messages = detail.messages;
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // ── Send ──────────────────────────────────────────────────────────────────

  async function send(): Promise<void> {
    const body = inputText.trim();
    if (!body || sending) return;

    // Optimistic user bubble (temporary — will be reconciled with server msg).
    const optimisticMsg: RefinementMessage = {
      id: `optimistic-${Date.now()}`,
      thread_id: tid,
      role: 'user',
      body,
      meta_json: null,
      created_at: new Date().toISOString(),
    };
    messages = [...messages, optimisticMsg];
    inputText = '';
    sending = true;

    try {
      const resp = await product.sendRefinementMessage(tid, body);

      // Reconcile: replace the optimistic bubble with the real one and append agent reply.
      messages = [
        ...messages.filter((m) => m.id !== optimisticMsg.id),
        resp.user_message,
        resp.agent_message,
      ];
    } catch (e) {
      // Roll back the optimistic bubble, restore the typed text, and show an error.
      messages = messages.filter((m) => m.id !== optimisticMsg.id);
      inputText = body;
      toasts.error('Send failed', e instanceof Error ? e.message : String(e));
    } finally {
      sending = false;
    }
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function parseMeta(metaJson: string | null): Record<string, unknown> {
    if (!metaJson) return {};
    try {
      return JSON.parse(metaJson) as Record<string, unknown>;
    } catch {
      return {};
    }
  }

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

<div class="refine-chat">
  <!-- ── Messages ──────────────────────────────────────────────────────────── -->
  <div class="messages-area">
    {#if loading && messages.length === 0}
      <div class="muted center-hint">Loading…</div>
    {:else if loadError}
      <div class="error-msg">Could not load thread: {loadError}</div>
    {:else if messages.length === 0}
      <div class="empty-state">
        <p>No messages yet.</p>
        <p>Type a message below to start the conversation with the agent.</p>
      </div>
    {:else}
      {#each messages as m (m.id)}
        {@const meta = m.role === 'agent' ? parseMeta(m.meta_json) : {}}
        {@const storyUpdated = Boolean(meta.story_updated)}
        {@const versionNo = meta.version_no != null ? Number(meta.version_no) : null}
        <div class="bubble-row" class:row-user={m.role === 'user'} class:row-agent={m.role === 'agent'}>
          <div class="bubble" class:bubble-user={m.role === 'user'} class:bubble-agent={m.role === 'agent'}>
            <div class="bubble-header">
              <span class="bubble-role">{m.role === 'user' ? 'PO' : 'Agent'}</span>
              <span class="bubble-time">{relDate(m.created_at)}</span>
            </div>
            <div class="bubble-body md-body">{@html renderMarkdown(m.body)}</div>
            {#if storyUpdated && versionNo !== null}
              <button
                class="story-updated-chip"
                onclick={() => (product.tab = 'rewrite')}
                title="Switch to Rewrite tab to review the suggested version"
              >
                ✓ Story updated → v{versionNo}
              </button>
            {/if}
          </div>
        </div>
      {/each}

      <!-- Thinking indicator while a turn is in flight -->
      {#if sending}
        <div class="bubble-row row-agent">
          <div class="bubble bubble-agent thinking">
            <span class="bubble-role">Agent</span>
            <span class="thinking-dots">thinking…</span>
          </div>
        </div>
      {/if}
    {/if}
  </div>

  <!-- ── Input ─────────────────────────────────────────────────────────────── -->
  <div class="input-area">
    <textarea
      class="msg-input"
      placeholder="Type a message… (Enter to send, Shift+Enter for newline)"
      bind:value={inputText}
      onkeydown={handleKeydown}
      disabled={sending}
      rows={3}
    ></textarea>
    <button
      class="send-btn"
      onclick={() => void send()}
      disabled={sending || !inputText.trim()}
    >
      {sending ? 'Sending…' : 'Send'}
    </button>
  </div>
</div>

<style>
  .refine-chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    gap: 0;
  }

  /* ── Messages area ──────────────────────────────────────────────────────── */
  .messages-area {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 8px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .muted {
    color: var(--text-dim);
    font-size: 13px;
    font-style: italic;
  }
  .center-hint {
    text-align: center;
    padding: 24px 0;
  }
  .error-msg {
    color: #ef4444;
    font-size: 13px;
    padding: 8px 0;
  }
  .empty-state {
    padding: 32px 16px;
    text-align: center;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.6;
  }
  .empty-state p {
    margin: 4px 0;
  }

  /* ── Bubbles ────────────────────────────────────────────────────────────── */
  .bubble-row {
    display: flex;
  }
  .row-user {
    justify-content: flex-end;
  }
  .row-agent {
    justify-content: flex-start;
  }

  .bubble {
    max-width: 76%;
    border-radius: var(--radius-s);
    padding: 8px 11px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .bubble-user {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-bottom-right-radius: 3px;
  }
  .bubble-agent {
    background: var(--surface);
    border: 1px solid var(--border);
    border-bottom-left-radius: 3px;
  }

  .bubble-header {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .bubble-role {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .bubble-time {
    font-size: 10px;
    color: var(--text-dim);
  }

  .bubble-body {
    font-size: 13px;
    line-height: 1.55;
    color: var(--text);
    overflow-wrap: break-word;
  }

  /* Markdown inside bubbles */
  :global(.bubble-body h1, .bubble-body h2, .bubble-body h3) {
    margin: 0.7em 0 0.25em;
    font-weight: 600;
  }
  :global(.bubble-body p) {
    margin: 0.35em 0;
  }
  :global(.bubble-body ul, .bubble-body ol) {
    padding-inline-start: 1.3em;
    margin: 0.35em 0;
  }
  :global(.bubble-body code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.88em;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    border-radius: 3px;
    padding: 1px 4px;
  }

  /* Thinking bubble */
  .thinking {
    opacity: 0.7;
    font-style: italic;
  }
  .thinking-dots {
    font-size: 12.5px;
    color: var(--text-dim);
  }

  /* Story-updated chip */
  .story-updated-chip {
    align-self: flex-start;
    margin-top: 4px;
    padding: 2px 8px;
    border: 1px solid color-mix(in srgb, var(--accent) 50%, transparent);
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: background 100ms;
    white-space: nowrap;
  }
  .story-updated-chip:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }

  /* ── Input area ─────────────────────────────────────────────────────────── */
  .input-area {
    flex-shrink: 0;
    display: flex;
    gap: 8px;
    padding: 8px 8px 0;
    border-top: 1px solid var(--border);
  }

  .msg-input {
    flex: 1;
    min-width: 0;
    resize: none;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 13px;
    font-family: inherit;
    line-height: 1.5;
    transition: border-color 120ms;
    outline: none;
  }
  .msg-input:focus {
    border-color: var(--accent);
  }
  .msg-input:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .send-btn {
    flex-shrink: 0;
    align-self: flex-end;
    padding: 6px 14px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--accent);
    color: #fff;
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: opacity 110ms;
  }
  .send-btn:hover:not(:disabled) {
    opacity: 0.88;
  }
  .send-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>
