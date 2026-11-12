<script lang="ts">
  // Status-bar machine chip: the gauge alone while the Mac is calm; "Machine
  // busy · N queued" (warning) while background agents (reviews, workflow
  // steps, scheduled tasks…) would wait for it; "Low disk · N GB free" below
  // the floor. A click opens the details (no hover-only content: the same
  // dialog on a phone). Reads the shared, visibility-paused machine poller.
  import Icon from '../lib/components/Icon.svelte';
  import Modal from '../lib/components/Modal.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { router } from '../lib/router.svelte';
  import { machineStatus } from '../modules/settings/machineStatus.svelte';
  import { chipState, detailRows, queuedLine } from '../modules/settings/machineStatusText';

  $effect(() => machineStatus.watch());
  const chip = $derived(chipState(machineStatus.status));
  let open = $state(false);

  function openSettings(): void {
    open = false;
    router.go('settings/daemon');
  }
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

{#if open && machineStatus.status}
  {@const s = machineStatus.status}
  <Modal title="Machine" onclose={() => (open = false)}>
    <div class="mc-body" data-testid="machine-details">
      {#if s.busy && s.busy_reason}
        <p class="mc-note warn"><Icon name="warning" size={12} /> Busy: {s.busy_reason}. New background agents wait their turn; your own sessions never do.</p>
      {/if}
      {#if s.low_disk}
        <p class="mc-note warn"><Icon name="warning" size={12} /> Low disk space. Otto deletes nothing on its own.</p>
      {/if}
      <dl class="mc-rows">
        {#each detailRows(s) as row (row.label)}
          <dt>{row.label}</dt>
          <dd>{row.value}</dd>
        {/each}
      </dl>
      <h3 class="mc-h">Queued background agents</h3>
      {#if s.queued.length}
        <ul class="mc-queue">
          {#each s.queued as q, i (i)}
            <li>{queuedLine(q)}</li>
          {/each}
        </ul>
      {:else}
        <p class="mc-dim">Nothing is waiting.</p>
      {/if}
    </div>
    {#snippet footer()}
      {#if auth.isRoot}
        <button class="btn small" onclick={openSettings}>Machine resource settings</button>
      {/if}
      <button class="btn small primary" onclick={() => (open = false)}>Done</button>
    {/snippet}
  </Modal>
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
  .mc-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: var(--fs-s);
  }
  .mc-note {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
  }
  .mc-note.warn {
    color: var(--warning);
  }
  .mc-rows {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 4px 12px;
    margin: 0;
  }
  .mc-rows dt {
    color: var(--text-dim);
  }
  .mc-rows dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .mc-h {
    margin: 4px 0 0;
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .mc-queue {
    margin: 0;
    padding-inline-start: 18px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .mc-dim {
    margin: 0;
    color: var(--text-dim);
  }
</style>
