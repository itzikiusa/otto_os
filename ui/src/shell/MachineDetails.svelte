<script lang="ts">
  // The machine chip's details dialog (MachineChip.svelte). Split out so the
  // shell chunk carries only the chip; this body loads the first time someone
  // opens it (shell/lazyDialogs.ts).
  import Icon from '../lib/components/Icon.svelte';
  import Modal from '../lib/components/Modal.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { router } from '../lib/router.svelte';
  import { machineStatus } from '../modules/settings/machineStatus.svelte';
  import { detailRows, queuedLine } from '../modules/settings/machineStatusText';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  function openSettings(): void {
    onclose();
    router.go('settings/daemon');
  }
</script>

{#if machineStatus.status}
  {@const s = machineStatus.status}
  <Modal title="Machine" {onclose}>
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
      <button class="btn small primary" onclick={onclose}>Done</button>
    {/snippet}
  </Modal>
{/if}

<style>
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
