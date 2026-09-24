<script lang="ts">
  // Whole-document editor (JSON / Vertical views): the full row as one JSON
  // object; Save builds a Mongo replaceOne (or a per-changed-column SQL UPDATE)
  // and opens the normal review modal. With `rowIdx === -1` it is the INSERT
  // editor (insertOne / INSERT from the typed JSON). Mounted by ResultsGrid
  // while `flow.docEditor` is set, inside the shared Modal.
  import Modal from '../../lib/components/Modal.svelte';
  import type { EditFlow } from './EditFlow.svelte';

  interface Props {
    flow: EditFlow;
  }
  let { flow }: Props = $props();
  const inserting = $derived((flow.docEditor?.rowIdx ?? 0) < 0);
</script>

{#if flow.docEditor}
  <Modal title={inserting ? 'Insert document' : 'Edit document'} width={720} onclose={() => (flow.docEditor = null)}>
    <div class="cell-viewer">
      <p class="cv-hint">
        {#if inserting}
          {flow.engine === 'mongodb' ? 'insertOne' : 'INSERT'} is reviewed before it runs.
        {:else}
          The row is replaced/updated after review.
        {/if}
      </p>
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        class="cv-edit mono"
        bind:value={flow.docEditor.draft}
        spellcheck="false"
        autofocus
        aria-label="Document JSON"
        onkeydown={(e) => {
          if (e.key === 'Escape') { e.stopPropagation(); flow.docEditor = null; }
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) flow.saveDocEdit();
        }}
      ></textarea>
      <div class="cv-foot">
        {#if flow.docEditor.err}<span class="cv-err">{flow.docEditor.err}</span>{/if}
        <span class="grow"></span>
        <button class="btn small ghost" onclick={() => (flow.docEditor = null)}>Cancel</button>
        <button class="btn small primary" onclick={() => flow.saveDocEdit()} title="Validate and review the statement (⌘⏎)">Save…</button>
      </div>
    </div>
  </Modal>
{/if}

<style>
  /* Scoped copy of the shared dialog rules (see CellViewer.svelte). */
  .cell-viewer {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
  }
  .cv-hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* In-viewer editor (JSON/long text): fills the same band as .cv-body. */
  .cv-edit {
    flex: 1;
    min-height: 220px;
    padding: 10px;
    font-size: 12px;
    line-height: 1.55;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    resize: none;
    white-space: pre;
  }
  .cv-foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cv-err {
    color: var(--status-exited);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
