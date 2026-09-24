<script lang="ts">
  // A JavaScript dialog (alert / confirm / prompt / beforeunload) the remote
  // page opened. The page is blocked until it's answered (the daemon
  // auto-dismisses after 60 s). Drawn in the live pane, attributed to the
  // page's host — it is the SITE talking, never Otto, so it is not an Otto
  // Modal and its buttons don't use Otto's action verbs for Otto things.
  interface Props {
    kind: 'alert' | 'confirm' | 'prompt' | 'beforeunload';
    message: string;
    defaultPrompt: string;
    url: string;
    onanswer: (accept: boolean, promptText?: string) => void;
    /** Watch-only viewers can see it but not answer. */
    canAnswer: boolean;
  }
  let { kind, message, defaultPrompt, url, onanswer, canAnswer }: Props = $props();

  let text = $state('');
  $effect(() => {
    text = defaultPrompt;
  });

  const host = $derived.by(() => {
    try {
      return new URL(url).host || 'This page';
    } catch {
      return 'This page';
    }
  });
  let cardEl = $state<HTMLElement | null>(null);
  $effect(() => {
    const _m = message;
    cardEl?.focus({ preventScroll: true });
  });

  function onkeydown(e: KeyboardEvent): void {
    if (!canAnswer) return;
    if (e.key === 'Escape' && kind !== 'alert') {
      e.preventDefault();
      onanswer(false);
    }
  }
</script>

<div
  class="dialog"
  role="alertdialog"
  aria-modal="false"
  aria-label={`${host} says`}
  tabindex="-1"
  bind:this={cardEl}
  {onkeydown}
  data-testid="live-page-dialog"
>
  <p class="from"><span dir="ltr">{host}</span> says</p>
  {#if kind === 'beforeunload'}
    <p class="msg">Leave this page? Changes you made may not be saved.</p>
  {:else}
    <p class="msg">{message}</p>
  {/if}
  {#if kind === 'prompt'}
    <input class="input" bind:value={text} aria-label="Answer to the page" disabled={!canAnswer} />
  {/if}
  <div class="actions">
    {#if !canAnswer}
      <span class="dim">Waiting for the person driving this page.</span>
    {:else if kind === 'alert'}
      <button class="btn primary" onclick={() => onanswer(true)}>Close</button>
    {:else if kind === 'beforeunload'}
      <button class="btn" onclick={() => onanswer(false)}>Stay</button>
      <button class="btn primary" onclick={() => onanswer(true)}>Leave</button>
    {:else if kind === 'prompt'}
      <button class="btn" onclick={() => onanswer(false)}>Cancel</button>
      <button class="btn primary" onclick={() => onanswer(true, text)}>Send answer</button>
    {:else}
      <button class="btn" onclick={() => onanswer(false)}>Cancel</button>
      <button class="btn primary" onclick={() => onanswer(true)}>Confirm</button>
    {/if}
  </div>
</div>

<style>
  .dialog {
    width: min(400px, 100%);
    max-height: 100%;
    overflow-y: auto;
    padding: 14px 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow);
    color: var(--text);
    font-size: var(--fs-m);
  }
  .dialog:focus {
    outline: none;
  }
  .dialog:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .from {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .msg {
    margin: 6px 0 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .input {
    width: 100%;
    margin-top: 10px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 6px;
    margin-top: 12px;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
