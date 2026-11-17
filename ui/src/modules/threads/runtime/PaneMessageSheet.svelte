<!--
  The sheet behind "Message every pane…" / "Ask every seat…": a paragraph,
  not a line (⌘↵ sends), with who receives it and who can't said above it.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { paneMessage } from './paneMessage.svelte';

  onMount(() => {
    paneMessage.hosts += 1;
    return () => {
      paneMessage.hosts -= 1;
      paneMessage.done(null);
    };
  });

  let text = $state('');
  let error = $state('');
  $effect(() => {
    if (paneMessage.req) {
      text = '';
      error = '';
    }
  });

  function send(): void {
    if (!text.trim()) {
      error = 'Write a message first';
      return;
    }
    paneMessage.done(text);
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      send();
    }
  }
</script>

{#if paneMessage.req}
  {@const req = paneMessage.req}
  <Modal title={req.title} width={520} onclose={() => paneMessage.done(null)}>
    <div class="pm" data-testid="pane-message-sheet">
      <p class="pm-intro" data-testid="pane-message-intro">{req.intro}</p>
      {#if req.note}<p class="pm-note" data-testid="pane-message-note">{req.note}</p>{/if}
      <textarea
        bind:value={text}
        rows="5"
        dir="auto"
        aria-label="Message"
        aria-invalid={error ? 'true' : undefined}
        aria-describedby="pane-message-error"
        placeholder={req.placeholder}
        data-autofocus
        data-testid="pane-message-input"
        oninput={() => (error = '')}
        onkeydown={onKey}
      ></textarea>
      <p class="pm-error" id="pane-message-error" aria-live="polite">{error}</p>
    </div>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => paneMessage.done(null)}>Cancel</button>
      <button type="button" class="btn primary" data-testid="pane-message-send" onclick={send}>{req.confirmLabel}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .pm {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pm-intro,
  .pm-note,
  .pm-error {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.45;
  }
  .pm-note {
    color: var(--text-dim);
  }
  .pm-error {
    color: var(--danger);
  }
  .pm-error:empty {
    display: none;
  }
  textarea {
    inline-size: 100%;
    box-sizing: border-box;
    min-block-size: 96px;
    max-block-size: 40vh;
    resize: vertical;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent-line-strong);
  }
  @media (pointer: coarse) {
    textarea {
      font-size: 16px; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
    }
  }
</style>
