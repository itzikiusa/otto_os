<script lang="ts">
  // The composer's ONE primary button (composerSend.ts): Send when idle;
  // Stop while the agent works and nothing is typed; with a draft, the
  // mid-turn delivery it will do (Queue / Steer, named in its icon, tooltip
  // and label), and Stop steps back to a small quiet control beside it.
  // While the agent waits on the person (the attention dock), Stop is quiet
  // too, so the dock's answer is the one primary action. Stop's keys (Esc /
  // ⌘.) are stopKeys.ts. A press on it keeps the caret in the composer, so
  // a phone's keyboard stays up for the next message.
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import { isMacPlatform } from '../../../lib/keys';
  import { primaryLabel, type PrimaryAction } from './composerSend';

  interface Props {
    action: PrimaryAction;
    /** The agent takes a message mid-turn (`caps.steer`). */
    canSteer: boolean;
    agent: string;
    sendDisabled: boolean;
    stopping: boolean;
    /** The agent waits on an answer in the dock: Stop is secondary. */
    waiting: boolean;
    /** Stop shows beside the primary (a turn runs with a draft typed). */
    running: boolean;
    stopTestId: string;
    sendTestId: string;
    onstop: () => void;
    onsend: () => void;
  }
  let { action, canSteer, agent, sendDisabled, stopping, waiting, running, stopTestId, sendTestId, onstop, onsend }: Props = $props();
  const mac = isMacPlatform();
  const stopKeys = mac ? 'Esc · ⌘.' : 'Esc · Ctrl+.';
  const ICON: Record<Exclude<PrimaryAction, 'stop'>, IconName> = { send: 'arrowUp', queue: 'clock', steer: 'zap' };
  const named = $derived(primaryLabel(action, { agent, canSteer, mac }));
</script>

{#if running}
  <button
    type="button"
    class="rt-send stop"
    class:quiet={waiting || action !== 'stop'}
    class:small={action !== 'stop'}
    aria-label="Stop the turn"
    aria-keyshortcuts="Escape"
    title="Stop ({stopKeys})"
    data-testid={stopTestId}
    disabled={stopping}
    onclick={onstop}
  >
    <Icon name="stop" size={action === 'stop' ? 14 : 12} />
  </button>
{/if}
{#if action !== 'stop'}
  <button
    type="button"
    class="rt-send"
    class:worded={action !== 'send'}
    aria-label={named.label}
    title={named.title}
    data-testid={sendTestId}
    data-delivery={action}
    disabled={sendDisabled}
    onmousedown={(e) => e.preventDefault()}
    onclick={onsend}
  >
    <Icon name={ICON[action]} size={action === 'send' ? 16 : 14} />
    <!-- Queue / Steer say their word (a narrow phone keeps the icon + its label). -->
    {#if action !== 'send'}<span class="rt-word" aria-hidden="true">{action === 'queue' ? 'Queue' : 'Steer'}</span>{/if}
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
  /* Queue / Steer: a pill with the word beside the icon. */
  .rt-send.worded {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    inline-size: auto;
    padding-inline: 10px 12px;
    border-radius: 999px;
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
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
  .rt-send.small {
    inline-size: 24px;
    block-size: 24px;
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
  /* Touch: the primary gets a full 44 px+ target; the secondary Stop a 44 px one. */
  @media (pointer: coarse) {
    .rt-send {
      inline-size: calc(var(--hit-min) + 8px);
      block-size: calc(var(--hit-min) + 8px);
    }
    .rt-send.worded {
      inline-size: auto;
      min-inline-size: calc(var(--hit-min) + 8px);
    }
    .rt-send.small {
      inline-size: calc(var(--hit-min) + 8px);
      block-size: calc(var(--hit-min) + 8px);
    }
  }
  /* A phone: icon only (the button's label still names it). */
  @media (max-width: 640px) {
    .rt-word {
      display: none;
    }
    .rt-send.worded {
      justify-content: center;
      padding-inline: 0;
    }
  }
</style>
