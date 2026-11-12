<script lang="ts">
  // Status-bar machine chip: the gauge alone while the Mac is calm; "Machine
  // busy · N queued" (warning) while background agents (reviews, workflow
  // steps, scheduled tasks…) would wait for it; "Low disk · N GB free" below
  // the floor. A click opens the details (no hover-only content: the same
  // dialog on a phone). Reads the shared, visibility-paused machine poller.
  // The details body is its own chunk (shell/lazyDialogs.ts), loaded on first open.
  import Icon from '../lib/components/Icon.svelte';
  import LazyMount from '../lib/components/LazyMount.svelte';
  import { machineStatus } from '../modules/settings/machineStatus.svelte';
  import { chipState } from '../modules/settings/machineStatusText';
  import { MachineDetailsChunk } from './lazyDialogs';

  $effect(() => machineStatus.watch());
  const chip = $derived(chipState(machineStatus.status));
  let open = $state(false);
</script>

{#if chip}
  <button
    class="machine-chip"
    class:warn={chip.tone !== 'calm'}
    onclick={() => (open = true)}
    title={chip.label}
    aria-label={chip.label}
    data-testid="statusbar-machine"
    data-tone={chip.tone}
  >
    <Icon name="gauge" size={12} />
    {#if chip.text}<span class="mc-text">{chip.text}</span>{/if}
  </button>
{/if}

{#if open}
  <LazyMount lazy={MachineDetailsChunk} what="the machine details" quiet onerror={() => (open = false)} props={{ onclose: () => (open = false) }} />
{/if}

<style>
  .machine-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 1px 4px;
    border-radius: var(--radius-s);
    white-space: nowrap;
  }
  .machine-chip:hover {
    background: var(--hover);
  }
  .machine-chip.warn {
    color: var(--warning);
    font-weight: 600;
  }
  /* Phone / narrow window: the icon alone (aria-label + title keep the words). */
  @media (max-width: 640px) {
    .mc-text {
      display: none;
    }
  }
</style>
