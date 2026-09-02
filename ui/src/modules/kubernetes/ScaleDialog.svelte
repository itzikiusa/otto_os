<script lang="ts">
  // Scale sheet: replicas stepper (current value from `extra.desired`). Scaling
  // to 0 goes through the typed-name confirm in `runAction`.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import type { K8sRow } from '../../lib/api/types';

  interface Props {
    row: K8sRow;
    kindLabel: string;
    onsubmit: (replicas: number) => void;
    onclose: () => void;
  }
  let { row, kindLabel, onsubmit, onclose }: Props = $props();

  const current = $derived(Number.parseInt(row.extra?.desired ?? '', 10));
  // Seeded once from the row the sheet opened with (the sheet is remounted per row).
  const seed = untrack(() => Number.parseInt(row.extra?.desired ?? '', 10));
  let replicas = $state(Number.isFinite(seed) ? seed : 1);
  const valid = $derived(Number.isInteger(replicas) && replicas >= 0 && replicas <= 1000);

  function submit(): void {
    if (!valid) return;
    onsubmit(replicas);
  }
</script>

<Modal title="Scale {kindLabel}" width={420} {onclose}>
  <div class="scale">
    <div class="target mono">{row.namespace ? `${row.namespace}/` : ''}{row.name}</div>
    <div class="field">
      <label for="k8s-scale-n">Replicas {#if Number.isFinite(current)}<span class="dim">(currently {current})</span>{/if}</label>
      <div class="stepper">
        <button class="btn" onclick={() => (replicas = Math.max(0, replicas - 1))} aria-label="Fewer replicas">−</button>
        <input id="k8s-scale-n" class="input mono" type="number" min="0" max="1000" bind:value={replicas} onkeydown={(e) => { if (e.key === 'Enter') submit(); }} />
        <button class="btn" onclick={() => (replicas = Math.min(1000, replicas + 1))} aria-label="More replicas">+</button>
      </div>
      {#if replicas === 0}<span class="hint danger">Scaling to 0 stops every pod. You'll be asked to type the name to confirm.</span>{/if}
    </div>
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={!valid} onclick={submit}>Scale</button>
  {/snippet}
</Modal>

<style>
  .scale {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .target {
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .stepper {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .stepper .input {
    width: 90px;
    text-align: center;
  }
  .hint.danger {
    color: var(--status-exited);
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
