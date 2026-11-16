<script lang="ts">
  // The composer's round buttons. While a turn runs, Stop is ALWAYS there:
  // with text in the composer Send (queue / steer, as before) sits beside it,
  // never in its place. While the agent waits on the person (the attention
  // dock), Stop steps back to a quiet outline so the dock's answer is the one
  // primary action. Stop's keys (Esc / ⌘.) are stopKeys.ts.
  import Icon from '../../../lib/components/Icon.svelte';
  import { isMacPlatform } from '../../../lib/keys';

  interface Props {
    running: boolean;
    /** Something to send or queue is in the composer. */
    canSend: boolean;
    sendDisabled: boolean;
    stopping: boolean;
    /** The agent waits on an answer in the dock: Stop is secondary. */
    waiting: boolean;
    stopTestId: string;
    sendTestId: string;
    onstop: () => void;
    onsend: () => void;
  }
  let { running, canSend, sendDisabled, stopping, waiting, stopTestId, sendTestId, onstop, onsend }: Props = $props();
  const stopKeys = isMacPlatform() ? 'Esc · ⌘.' : 'Esc · Ctrl+.';
</script>

{#if running}
  <button
    type="button"
    class="rt-send stop"
    class:quiet={waiting || canSend}
    aria-label="Stop the turn"
    aria-keyshortcuts="Escape"
    title="Stop ({stopKeys})"
    data-testid={stopTestId}
    disabled={stopping}
    onclick={onstop}
  >
    <Icon name="stop" size={14} />
  </button>
{/if}
{#if !running || canSend}
  <button
    type="button"
    class="rt-send"
    aria-label={running ? 'Queue message' : 'Send message'}
    title={running ? 'Queue' : 'Send'}
    data-testid={sendTestId}
    disabled={sendDisabled}
    onclick={onsend}
  >
    <Icon name="arrowUp" size={16} />
  </button>
{/if}

<style>
  .rt-send {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    inline-size: 30px;
    block-size: 30px;
    border: 0;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .rt-send.stop {
    background: var(--text);
    color: var(--bg);
  }
  .rt-send.stop.quiet {
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text);
  }
  .rt-send.stop.quiet:hover:not(:disabled) {
    background: var(--hover);
  }
  .rt-send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
  .rt-send:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  /* Touch: the most-pressed controls in the thread get a full 44 px target. */
  @media (pointer: coarse) {
    .rt-send {
      inline-size: calc(var(--hit-min) + 8px);
      block-size: calc(var(--hit-min) + 8px);
    }
  }
</style>
