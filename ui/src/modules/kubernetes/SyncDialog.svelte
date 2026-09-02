<script lang="ts">
  // ArgoCD sync sheet: optional revision override + prune. Prune is
  // destructive (deletes resources not in git) → typed-name confirm in
  // `runAction`.
  import Modal from '../../lib/components/Modal.svelte';
  import type { K8sRow } from '../../lib/api/types';

  interface Props {
    row: K8sRow;
    onsubmit: (params: { revision?: string; prune: boolean }) => void;
    onclose: () => void;
  }
  let { row, onsubmit, onclose }: Props = $props();

  let revision = $state('');
  let prune = $state(false);

  function submit(): void {
    onsubmit({ revision: revision.trim() || undefined, prune });
  }
</script>

<Modal title="Sync application" width={460} {onclose}>
  <div class="sync">
    <div class="target mono">{row.namespace ? `${row.namespace}/` : ''}{row.name}</div>
    <dl class="facts">
      {#if row.extra?.sync}<dt>Sync</dt><dd>{row.extra.sync}</dd>{/if}
      {#if row.extra?.health}<dt>Health</dt><dd>{row.extra.health}</dd>{/if}
      {#if row.extra?.revision}<dt>Revision</dt><dd class="mono">{row.extra.revision}</dd>{/if}
      {#if row.extra?.repo}<dt>Repo</dt><dd class="mono">{row.extra.repo}{row.extra.path ? ` · ${row.extra.path}` : ''}</dd>{/if}
    </dl>
    <div class="field">
      <label for="k8s-sync-rev">Revision <span class="dim">(blank = the app's targetRevision)</span></label>
      <input id="k8s-sync-rev" class="input mono" bind:value={revision} placeholder="HEAD, a branch, tag or SHA" onkeydown={(e) => { if (e.key === 'Enter') submit(); }} />
    </div>
    <label class="checkbox-row">
      <input type="checkbox" bind:checked={prune} />
      Prune resources that are no longer in git
    </label>
    {#if prune}<span class="hint danger">Prune deletes live resources. You'll be asked to type the application name to confirm.</span>{/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={submit}>Sync</button>
  {/snippet}
</Modal>

<style>
  .sync {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .target {
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 12px;
    margin: 0;
    font-size: 12px;
  }
  .facts dt {
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint.danger {
    color: var(--status-exited);
    font-size: 11.5px;
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
