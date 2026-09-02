<script lang="ts">
  // Ask bar: one line of input that submits a `/browser/ask` turn into an
  // agent session — the page the user is on, the marks on it (fenced
  // server-side, same as send-to-session), and the question. The Browser
  // page hosts it under its embedded agent dock (AgentDock); the agent-mode
  // right panel's v2 Browser tab hosts it standalone, targeting the active
  // session. Marks are included by default so "what does the element I
  // marked do?" resolves without the agent guessing which element is meant.

  import { browser } from '../../lib/stores/browser.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    /** The session the question goes to; null disables the bar with a hint. */
    sessionId: string | null;
    /** Hint shown when no session is bound (differs per host). */
    unboundHint?: string;
  }
  let { sessionId, unboundHint = 'Attach an agent session to ask about this page.' }: Props = $props();

  let text = $state('');
  let includeMarks = $state(true);
  let sending = $state(false);
  let inputEl = $state<HTMLTextAreaElement | null>(null);
  // Set right after a mark lands so the placeholder nudges toward it.
  let justMarked = $state(false);

  const url = $derived(browser.activeTab?.url ?? '');
  const marks = $derived(browser.annotations.filter((a) => a.url === url));
  const session = $derived(sessionId ? (ws.sessions.find((s) => s.id === sessionId) ?? null) : null);
  const status = $derived(sessionId ? (ws.statusMap[sessionId] ?? session?.status ?? null) : null);
  const live = $derived(status === 'running' || status === 'working' || status === 'idle');
  const canSend = $derived(!!sessionId && live && !!url && text.trim() !== '' && !sending);

  // A mark just landed from this device — focus the bar and hint at it, so
  // the natural next step ("what is this?") is one keystroke away.
  let lastTick = browser.markTick;
  $effect(() => {
    const tick = browser.markTick;
    if (tick === lastTick) return;
    lastTick = tick;
    if (!sessionId) return;
    justMarked = true;
    inputEl?.focus();
  });
  // Clear the hint once the user starts typing or moves to another page.
  $effect(() => {
    void url;
    if (text !== '') justMarked = false;
  });

  const placeholder = $derived(
    !sessionId
      ? unboundHint
      : !url
        ? 'Open a page first.'
        : !live
          ? 'The attached session is not live — resume it to ask.'
          : justMarked
            ? 'Ask about the element you just marked… (⏎ to send)'
            : 'Ask the agent about this page… (⏎ to send, ⇧⏎ for a new line)',
  );

  async function send(): Promise<void> {
    if (!canSend || !sessionId) return;
    const q = text.trim();
    const ids = includeMarks ? marks.slice(-20).map((a) => a.id) : [];
    sending = true;
    try {
      await browser.ask({ session_id: sessionId, url, text: q, annotation_ids: ids });
      text = '';
      justMarked = false;
    } catch (e) {
      toasts.error('Ask failed', e instanceof Error ? e.message : undefined);
    } finally {
      sending = false;
    }
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  // Grow with content up to a few lines; the terminal above keeps the rest.
  function autosize(): void {
    if (!inputEl) return;
    inputEl.style.height = 'auto';
    inputEl.style.height = `${Math.min(inputEl.scrollHeight, 120)}px`;
  }
</script>

<div class="askbar" class:disabled={!sessionId}>
  <textarea
    bind:this={inputEl}
    bind:value={text}
    rows="1"
    {placeholder}
    disabled={!sessionId || !url}
    onkeydown={onkeydown}
    oninput={autosize}
    spellcheck="false"
    aria-label="Ask the agent about this page"
  ></textarea>
  <label class="chip" class:off={!includeMarks} title="Include this page's marks in the question so the agent knows which elements you mean">
    <input type="checkbox" bind:checked={includeMarks} disabled={marks.length === 0} />
    <Icon name="target" size={11} />
    <span>{marks.length} mark{marks.length === 1 ? '' : 's'}</span>
  </label>
  <button class="send" onclick={send} disabled={!canSend} title="Send to the agent (⏎)" aria-label="Send">
    <Icon name="send" size={13} />
  </button>
</div>

<style>
  .askbar {
    display: flex;
    align-items: flex-end;
    gap: 0.4rem;
    padding: 0.4rem 0.6rem;
    border-top: 1px solid var(--border);
    background: var(--surface);
  }
  textarea {
    flex: 1;
    min-height: 30px;
    max-height: 120px;
    resize: none;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 0.35rem 0.55rem;
    font: inherit;
    font-size: 0.85rem;
    line-height: 1.35;
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent);
  }
  textarea:disabled {
    opacity: 0.6;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    height: 30px;
    padding: 0 0.5rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: 0.72rem;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
  }
  .chip.off {
    color: var(--text-dim);
    background: transparent;
  }
  .chip input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .send {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-fg, #fff);
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>
