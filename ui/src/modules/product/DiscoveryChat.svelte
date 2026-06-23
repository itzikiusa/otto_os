<script lang="ts">
  // DiscoveryChat — the chat pane for one Discovery Chat. A conversational agent
  // that works from an EMPTY/Untitled draft to help with early discovery &
  // research, and proposes Apply-able action cards.
  //
  // Forks RefineChat's optimistic-bubble + thinking-indicator pattern, and adds:
  //   • an EmptyState + starter-prompt chip row (chips PREFILL the composer,
  //     never auto-send) when the chat has zero messages;
  //   • ActionCards parsed from each agent message's `actions_json`.
  // Props: { cid } (the chat id). Parent (ChatTab) controls which chat is active.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { renderMarkdown } from '../../lib/md';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ActionCard from './ActionCard.svelte';
  import type { DiscoveryChatMessage, DiscoveryAction } from './types';

  let { cid }: { cid: string } = $props();

  // ── Local state ────────────────────────────────────────────────────────────
  let messages = $state<DiscoveryChatMessage[]>([]);
  let messagesEl = $state<HTMLDivElement | null>(null);

  // Keep the newest turn in view (the optimistic user bubble bumps the count on
  // send; the agent reply bumps it again on completion).
  $effect(() => {
    void messages.length;
    if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  });
  let loadError = $state<string | null>(null);
  let loading = $state(false);

  let inputText = $state('');
  let sending = $state(false);

  let composerEl = $state<HTMLTextAreaElement | null>(null);

  // Starter prompts — clicking a chip PREFILLS the composer (does not auto-send).
  const STARTERS = [
    'Help me scope a story for: ',
    'What questions should I answer before building this?',
    'Research how other products solve this problem',
    'What edge cases and failure modes am I missing?',
    'Draft acceptance criteria from what we’ve discussed',
    'Summarize this into a story I can publish',
  ];

  // ── Load / reload when cid changes ────────────────────────────────────────
  $effect(() => {
    // Reactive on cid — re-runs whenever the active chat switches.
    const currentCid = cid;
    void loadChat(currentCid);
  });

  async function loadChat(chatId: string): Promise<void> {
    loading = true;
    loadError = null;
    try {
      const detail = await product.getDiscoveryChat(chatId);
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

    // Optimistic user bubble (temporary — reconciled with the server message).
    const optimisticMsg: DiscoveryChatMessage = {
      id: `optimistic-${Date.now()}`,
      chat_id: cid,
      role: 'user',
      body,
      actions_json: null,
      meta_json: null,
      created_at: new Date().toISOString(),
    };
    messages = [...messages, optimisticMsg];
    inputText = '';
    sending = true;

    try {
      const resp = await product.sendDiscoveryMessage(cid, body);
      // Reconcile: drop the optimistic bubble, append the real user + agent msgs.
      messages = [
        ...messages.filter((m) => m.id !== optimisticMsg.id),
        resp.user_message,
        resp.agent_message,
      ];
    } catch (e) {
      // Roll back the optimistic bubble, restore the typed text, show an error.
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

  function useStarter(text: string): void {
    inputText = text;
    // Focus the composer so the PO can keep typing right where the chip left off.
    composerEl?.focus();
    // Move the caret to the end (important for the "Help me scope a story for: "
    // prefix, which expects the PO to continue typing).
    queueMicrotask(() => {
      if (composerEl) {
        const end = composerEl.value.length;
        composerEl.setSelectionRange(end, end);
      }
    });
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function parseActions(actionsJson: string | null): DiscoveryAction[] {
    if (!actionsJson) return [];
    try {
      const parsed = JSON.parse(actionsJson);
      return Array.isArray(parsed) ? (parsed as DiscoveryAction[]) : [];
    } catch {
      return [];
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

<div class="discovery-chat">
  <!-- ── Messages ──────────────────────────────────────────────────────────── -->
  <div class="messages-area" bind:this={messagesEl}>
    {#if loading && messages.length === 0}
      <div class="muted center-hint">Loading…</div>
    {:else if loadError}
      <div class="error-msg">Could not load chat: {loadError}</div>
    {:else if messages.length === 0}
      <!-- EMPTY STATE — figure out what to build before writing anything. -->
      <div class="empty-wrap">
        <EmptyState
          icon="zap"
          title="Let's figure out what to build"
          body="Tell me the rough idea or a problem you're chasing. I'll research, ask the right questions, and turn it into a story — no need to write anything first."
        />
        <div class="starters" class:scroll-row={viewport.isPhone}>
          {#each STARTERS as s (s)}
            <button class="starter-chip" onclick={() => useStarter(s)} title="Prefill the composer">
              {s.trim()}
            </button>
          {/each}
        </div>
        <p class="see-hint">I can see your draft, mockups, and discovery notes.</p>
      </div>
    {:else}
      {#each messages as m (m.id)}
        {@const actions = m.role === 'agent' ? parseActions(m.actions_json) : []}
        <div class="bubble-row" class:row-user={m.role === 'user'} class:row-agent={m.role === 'agent'}>
          <div class="bubble" class:bubble-user={m.role === 'user'} class:bubble-agent={m.role === 'agent'}>
            <div class="bubble-header">
              <span class="bubble-role">{m.role === 'user' ? 'PO' : 'Agent'}</span>
              <span class="bubble-time">{relDate(m.created_at)}</span>
            </div>
            {#if m.role === 'agent'}
              <div class="bubble-body md-body">{@html renderMarkdown(m.body)}</div>
            {:else}
              <div class="bubble-body plain">{m.body}</div>
            {/if}
            {#if actions.length > 0}
              <div class="action-cards">
                {#each actions as a, i (i)}
                  <ActionCard {cid} action={a} />
                {/each}
              </div>
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

  <!-- ── Composer ──────────────────────────────────────────────────────────── -->
  <div class="input-area">
    <textarea
      bind:this={composerEl}
      class="msg-input"
      placeholder="Describe the idea or problem… (Enter to send, Shift+Enter for newline)"
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
  .discovery-chat {
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

  /* ── Empty state ────────────────────────────────────────────────────────── */
  .empty-wrap {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    max-width: 560px;
  }
  .starters {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 7px;
    padding: 4px 8px 2px;
  }
  /* On phone the chips become a single horizontal-scroll row. */
  .starters.scroll-row {
    flex-wrap: nowrap;
    justify-content: flex-start;
    overflow-x: auto;
    width: 100%;
    -webkit-overflow-scrolling: touch;
    scrollbar-width: thin;
  }
  .starter-chip {
    flex-shrink: 0;
    padding: 5px 11px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms;
  }
  .starter-chip:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .see-hint {
    margin: 6px 0 0;
    font-size: 11.5px;
    color: var(--text-dim);
    font-style: italic;
    text-align: center;
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
    max-width: 80%;
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
  .bubble-body.plain {
    white-space: pre-wrap;
  }

  /* Markdown inside agent bubbles */
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

  .action-cards {
    display: flex;
    flex-direction: column;
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

  /* ── Composer ───────────────────────────────────────────────────────────── */
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
