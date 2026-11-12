<script lang="ts">
  import { announceEdge } from '../../lib/announce.svelte';
  import { onDestroy, tick, untrack } from 'svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  // RefineChat — one story Chat conversation (a refinement thread): the
  // transcript, the composer with a compact agent picker, the agent's cards
  // (questions / notes / a diagram, applied one by one) inline, and a link to
  // Rewrite for each suggested version a turn wrote. Props: { tid }; the
  // parent (ChatTab) controls which conversation is active.
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import { product } from '../../lib/stores/product.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { renderMarkdown } from '../../lib/md';
  import AgentByline from '../../lib/components/AgentByline.svelte';
  import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import ActionCard from './ActionCard.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { chatMessageCards, chatMessageSuggestion } from './chatActions';
  import type { RefinementMessage } from './types';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';

  let { tid }: { tid: string } = $props();

  // Provider for this turn's agent. Each turn replays the thread history in the
  // prompt, so a per-turn provider is self-contained — sourced from the live
  // registry so custom providers (e.g. grok) work.
  let provider = $state(defaultAgentProvider());

  // ── Local state ────────────────────────────────────────────────────────────
  let messages = $state<RefinementMessage[]>([]);
  /** The thread's model alias, when it has one — shown in the agent byline. */
  let threadModel = $state<string | null>(null);
  let messagesEl = $state<HTMLDivElement | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(false);

  let inputText = $state('');
  let sending = $state(false);
  let stopping = $state(false);
  let activeTurnId = $state<string | null>(null);
  let stopNotice = $state<string | null>(null);
  let backgroundTurn = $state(false);
  /** "Reload …" in the after-stop bubble: where focus goes once a stop control removes itself. */
  let reloadBtn = $state<HTMLButtonElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  let loadSeq = 0;
  let activeId = '';
  let alive = true;
  let inFlightTurnId: string | null = null;
  const drafts = new Map<string, string>();
  const stopRequestedTurns = new Set<string>();
  const BACKGROUND_NOTICE = 'Stopped waiting — the agent turn may still finish in the background.';
  // Starter prompts — a chip PREFILLS the composer, it never sends.
  const STARTERS = [
    'Help me scope a story for: ',
    'What questions should I answer before building this?',
    'What edge cases and failure modes am I missing?',
    'Draft acceptance criteria from what we’ve discussed',
    'Rewrite the story so it is ready to publish',
  ];
  onDestroy(() => { alive = false; ++loadSeq; });

  function newTurnId(): string {
    return typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
      ? crypto.randomUUID()
      : `turn-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
  }

  // Keep the newest turn in view on initial load, user send, and agent reply.
  $effect(() => {
    void messages.length;
    void sending;
    if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  });

  // ── Load / reload when tid changes ────────────────────────────────────────
  $effect(() => {
    // Reactive on tid — re-runs whenever the active thread switches.
    const currentTid = tid;
    backgroundTurn = false;
    stopNotice = null;
    // Untracked: the loader's synchronous prefix reads/writes the composer
    // (`inputText`, `sending`, `stopping`); tracked, the first keystroke after a
    // switch re-ran this effect and reloaded the thread mid-typing.
    untrack(() => void loadThread(currentTid));
  });

  async function loadThread(threadId: string, opts?: { preserveMessages?: RefinementMessage[] }): Promise<void> {
    const my = ++loadSeq;
    const current = () => alive && my === loadSeq && tid === threadId;
    if (activeId !== threadId) {
      // An empty draft is an intentional decision too. While sending or stopping,
      // the cleared composer belongs to that in-flight request, not a new draft.
      if (activeId && !sending && !stopping) drafts.set(activeId, inputText);
      activeId = threadId;
      inputText = drafts.get(threadId) ?? '';
      messages = [];
      sending = false;
      stopping = false;
      activeTurnId = null;
      inFlightTurnId = null;
      stopNotice = null;
      backgroundTurn = false;
    }
    loading = true;
    loadError = null;
    try {
      const detail = await product.getRefinementThread(threadId);
      if (!current()) return;
      messages = detail.messages.length === 0 && opts?.preserveMessages?.length
        ? opts.preserveMessages
        : detail.messages;
      threadModel = detail.thread.model;
    } catch (e) {
      if (current()) loadError = loadErrorText(e);
    } finally {
      if (current()) loading = false;
    }
  }

  // ── Send & Stop ───────────────────────────────────────────────────────────

  async function stopTurn(): Promise<void> {
    if ((!sending && !stopping) || stopping || !activeTurnId) return;
    const target = tid;
    const turnId = activeTurnId;
    const generation = loadSeq;
    const ownsTurn = () => alive && tid === target && loadSeq === generation && activeTurnId === turnId;
    stopRequestedTurns.add(turnId);
    stopping = true;
    stopNotice = null;
    try {
      const res = await product.stopRefinementTurn(target, turnId);
      if (!ownsTurn()) return;
      if (res.stopped) {
        const snapshot = messages;
        sending = false;
        stopping = false;
        activeTurnId = null;
        inFlightTurnId = null;
        backgroundTurn = false;
        stopNotice = 'Turn stopped.';
        await loadThread(target, { preserveMessages: snapshot });
      } else {
        // stopped: false means the turn was not active on the server (not yet admitted or already finished),
        // NOT proof of cancellation. Keep the accepted user bubble and report truthfully.
        stopping = false;
        stopNotice = 'Couldn’t confirm stop — the turn was not active on the server yet (or already finished).';
        if (inFlightTurnId !== turnId) {
          sending = false;
          activeTurnId = null;
        }
      }
    } catch (e) {
      if (!ownsTurn()) return;
      stopping = false;
      if (inFlightTurnId !== turnId) {
        sending = false;
        activeTurnId = null;
      }
      toastError('Couldn’t stop the turn', e);
    }
  }

  function stopWaiting(): void {
    if (!sending && !stopping) return;
    if (activeTurnId) stopRequestedTurns.add(activeTurnId);
    ++loadSeq;
    sending = false;
    stopping = false;
    activeTurnId = null;
    inFlightTurnId = null;
    stopNotice = null;
    backgroundTurn = true;
  }

  /** "Reload …" removes itself: once the reload lands, focus the composer. */
  async function reloadAfterStop(): Promise<void> {
    backgroundTurn = false;
    stopNotice = null;
    await loadThread(tid);
    await tick();
    composerEl?.focus();
  }

  // a11y-6w2-02: the stop notice (in the thinking bubble, or the after-stop
  // bubble once the turn is let go) is spoken ONCE, here at its edge. Neither
  // bubble is a live region — a status bubble around a self-announcing line
  // spoke it twice.
  const sayStop = announceEdge();
  function speakStopNotice(): void {
    sayStop(backgroundTurn ? BACKGROUND_NOTICE : stopNotice);
  }
  $effect(speakStopNotice);

  /** "Stop" / "Stop waiting" remove themselves with the thinking bubble: when
   *  that drops focus to <body>, move it to "Reload …" in the after-stop
   *  bubble. Focus the person put elsewhere is left alone. */
  function rescueStopFocus(): void {
    const lost = !document.activeElement || document.activeElement === document.body;
    if (reloadBtn && lost) reloadBtn.focus();
  }
  $effect(rescueStopFocus);

  async function send(): Promise<void> {
    const body = inputText.trim();
    if (!body || sending || stopping || loading || Boolean(loadError)) return;
    const target = tid;
    const turnId = newTurnId();
    const generation = loadSeq;
    const current = () => alive && tid === target && loadSeq === generation && activeTurnId === turnId;
    drafts.delete(target);

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
    activeTurnId = turnId;
    inFlightTurnId = turnId;
    sending = true;
    stopping = false;
    stopNotice = null;
    backgroundTurn = false;

    try {
      const resp = await product.sendRefinementMessage(tid, body, provider, undefined, turnId);
      if (inFlightTurnId === turnId) inFlightTurnId = null;

      if (!current()) return;
      // Reconcile: replace the optimistic bubble with the real one and append agent reply.
      messages = [
        ...messages.filter((m) => m.id !== optimisticMsg.id),
        resp.user_message,
        resp.agent_message,
      ];
      stopNotice = null;
      sending = false;
      stopping = false;
      activeTurnId = null;
    } catch (e) {
      if (inFlightTurnId === turnId) inFlightTurnId = null;
      if (stopRequestedTurns.has(turnId)) {
        // User requested Stop or Stop waiting on this turn: the user message was
        // already accepted, so never restore it to the composer or drafts map.
        if (current() && !stopping) {
          sending = false;
          activeTurnId = null;
        }
        return;
      }
      // Keep a failed send with its original conversation; never overwrite
      // the draft the person is composing in the newly selected one.
      // A newer draft (including an explicit clear) wins over an old failure.
      if (!drafts.has(target)) drafts.set(target, body);
      if (!current()) return;
      // Roll back the optimistic bubble, restore the typed text, and show an error.
      messages = messages.filter((m) => m.id !== optimisticMsg.id);
      inputText = body;
      sending = false;
      stopping = false;
      activeTurnId = null;
      toastError('Couldn’t send the message', e);
    } finally {
      if (inFlightTurnId === turnId) inFlightTurnId = null;
      if (current() && !stopping) {
        sending = false;
        activeTurnId = null;
      }
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
    composerEl?.focus();
    // Caret to the end: "Help me scope a story for: " expects more typing.
    queueMicrotask(() => {
      const end = composerEl?.value.length ?? 0;
      composerEl?.setSelectionRange(end, end);
    });
  }

</script>

<div class="refine-chat">
  <!-- ── Messages ──────────────────────────────────────────────────────────── -->
  <div class="messages-area" bind:this={messagesEl}>
    {#if (loading || loadError) && messages.length === 0}
      <LoadState what="this thread" {loading} error={loadError} empty onretry={() => void loadThread(tid)} />
    {:else if messages.length === 0}
      <div class="empty-wrap">
        <EmptyState
          icon="comment"
          title="Let’s shape this story"
          body="Describe the rough idea or ask for a change. Proposed text is saved as a suggestion you review in Rewrite; questions, notes and diagrams come as cards you apply."
        />
        <div class="starters" class:scroll-row={viewport.isPhone} class:scroll-thin={viewport.isPhone}>
          {#each STARTERS as s (s)}
            <button class="starter-chip" type="button" onclick={() => useStarter(s)} title="Prefill the composer">
              {s.trim()}
            </button>
          {/each}
        </div>
      </div>
    {:else}
      {#if loadError}
        <LoadState what="this thread" {loading} error={loadError} onretry={() => void loadThread(tid)} />
      {/if}
      {#each messages as m (m.id)}
        {@const versionNo = m.role === 'agent' ? chatMessageSuggestion(m) : null}
        {@const cards = m.role === 'agent' ? chatMessageCards(m) : []}
        <div class="bubble-row" class:row-user={m.role === 'user'} class:row-agent={m.role === 'agent'}>
          <div class="bubble" class:bubble-user={m.role === 'user'} class:bubble-agent={m.role === 'agent'}>
            <div class="bubble-header">
              {#if m.role === 'agent'}
                <!-- Agent turn: attributed (patterns.md §2). -->
                <AgentByline model={threadModel} at={m.created_at} />
              {:else}
                <span class="bubble-role">You</span>
                <RelTime iso={m.created_at} class="bubble-time" />
              {/if}
            </div>
            {#if m.role === 'agent'}
              <div class="bubble-body md-body">{@html renderMarkdown(m.body)}</div>
            {:else}
              <div class="bubble-body plain">{m.body}</div>
            {/if}
            {#if versionNo !== null}
              <button
                class="story-updated-chip"
                type="button"
                onclick={() => void product.changeTab('rewrite')}
                title="Open Rewrite to compare this suggestion with the story and publish it"
              >
                <Icon name="edit" size={12} /> Suggestion v{versionNo} · Review in Rewrite
              </button>
            {/if}
            {#if cards.length > 0}
              <div class="action-cards">
                {#each cards as a, i (i)}
                  <ActionCard {tid} action={a} />
                {/each}
              </div>
            {/if}
          </div>
        </div>
      {/each}

      <!-- Thinking indicator while a turn is in flight -->
      {#if sending || stopping}
        <div class="bubble-row row-agent">
          <div class="bubble bubble-agent thinking">
            <div class="bubble-header"><AgentByline {provider} model={threadModel} /></div>
            <div class="thinking-row">
              <span class="thinking-dots">
                <LiveWorkingDot label={stopping ? 'Stopping…' : 'Thinking…'} />
              </span>
              <div class="thinking-actions">
                <button
                  class="btn small"
                  type="button"
                  onclick={() => void stopTurn()}
                  disabled={stopping || !activeTurnId}
                  title="Cancel this in-flight agent turn on the server"
                >
                  {stopping ? 'Stopping…' : 'Stop'}
                </button>
                <button
                  class="btn small"
                  type="button"
                  onclick={stopWaiting}
                  title="Stop waiting in the UI (the agent turn may still finish in the background)"
                >
                  Stop waiting
                </button>
              </div>
            </div>
            {#if stopNotice}
              <!-- Spoken once by speakStopNotice(); not a live region. -->
              <p class="stop-notice">{stopNotice}</p>
            {/if}
          </div>
        </div>
      {:else if backgroundTurn || stopNotice}
        <div class="bubble-row row-agent">
          <!-- Not a live region: speakStopNotice() says this text once. -->
          <div class="bubble bubble-agent thinking">
            <div class="thinking-row">
              <span class="thinking-dots">{backgroundTurn ? BACKGROUND_NOTICE : stopNotice}</span>
              <button class="btn small" type="button" bind:this={reloadBtn} onclick={() => void reloadAfterStop()}>
                Reload thread
              </button>
            </div>
          </div>
        </div>
      {/if}
    {/if}
  </div>

  <!-- ── Input ─────────────────────────────────────────────────────────────── -->
  <div class="input-area">
    <textarea dir="auto" aria-label="Message"
      bind:this={composerEl}
      class="msg-input"
      placeholder="Describe the idea or ask for a change… (Enter to send, Shift+Enter for newline)"
      bind:value={inputText}
      onkeydown={handleKeydown}
      disabled={sending || stopping || loading || Boolean(loadError)}
      rows={3}
    ></textarea>
    <label class="rc-provider" title="The agent that answers the next message">
      <span class="dim">Agent</span>
      <select bind:value={provider} aria-label="Chat agent" disabled={sending || stopping || loading || Boolean(loadError)}>
        {#each agentProviders() as p (p)}<option value={p}>{p}</option>{/each}
      </select>
    </label>
    <button
      class="send-btn"
      onclick={() => void send()}
      disabled={sending || stopping || loading || Boolean(loadError) || !inputText.trim()}
    >
      {stopping ? 'Stopping…' : sending ? 'Sending…' : 'Send'}
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
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .bubble-user {
    background: var(--accent-soft-strong);
    border: 1px solid var(--accent-line);
    border-end-end-radius: 3px;
  }
  .bubble-agent {
    background: var(--surface);
    border: 1px solid var(--border);
    border-inline-start: 2px solid var(--accent-line-strong);
    border-end-start-radius: 3px;
  }

  .bubble-header {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .bubble-role {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .bubble-header :global(.bubble-time) {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  .bubble-body {
    font-size: var(--fs-m);
    line-height: 1.55;
    color: var(--text);
    overflow-wrap: break-word;
  }
  .bubble-body.plain {
    white-space: pre-wrap;
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
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    border-radius: var(--radius-s);
    padding: 1px 4px;
  }

  /* Thinking bubble */
  .thinking {
    color: var(--text-dim);
  }
  .thinking-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .thinking-dots {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .thinking-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .stop-notice {
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  /* ── Empty state: starter chips ─────────────────────────────────────────── */
  .empty-wrap {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    max-width: 560px;
    min-width: 0;
  }
  .starters {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    padding: 4px 8px 2px;
  }
  /* On phone the chips become a single horizontal-scroll row. */
  .starters.scroll-row {
    flex-wrap: nowrap;
    justify-content: flex-start;
    overflow-x: auto;
    width: 100%;
  }
  .starter-chip {
    flex-shrink: 0;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--dur-fast), border-color var(--dur-fast);
  }
  .starter-chip:hover {
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }

  .action-cards {
    display: flex;
    flex-direction: column;
  }

  /* Story-updated chip */
  .story-updated-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    margin-top: 4px;
    padding: 2px 8px;
    border: 1px solid var(--accent-line-strong);
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
    cursor: pointer;
    transition: background var(--dur-fast);
    white-space: nowrap;
  }
  .story-updated-chip:hover {
    background: var(--accent-soft-strong);
  }

  /* ── Input area ─────────────────────────────────────────────────────────── */
  .input-area {
    flex-shrink: 0;
    display: flex;
    gap: 8px;
    padding: 8px 8px 0;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .rc-provider {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    margin-inline-end: auto;
    min-width: 0;
  }
  .rc-provider .dim {
    color: var(--text-dim);
  }
  .rc-provider select {
    padding: 2px 6px;
  }

  .msg-input {
    flex: 1 1 100%;
    min-width: 0;
    resize: none;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-m);
    font-family: inherit;
    line-height: 1.5;
    transition: border-color var(--dur-fast);
    outline: none;
  }
  .msg-input:focus {
    border-color: var(--accent-text); box-shadow: 0 0 0 3px var(--accent-soft-strong)
  }
  .msg-input:disabled {
    opacity: var(--disabled-opacity);
    cursor: not-allowed;
  }

  .send-btn {
    flex-shrink: 0;
    align-self: flex-end;
    padding: 6px 14px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: opacity var(--dur-fast);
  }
  .send-btn:hover:not(:disabled) {
    opacity: 0.88;
  }
  .send-btn:disabled {
    opacity: var(--disabled-opacity);
    cursor: not-allowed;
  }
</style>
