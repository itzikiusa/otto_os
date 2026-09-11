<script lang="ts">
  // Whole-document editor (JSON / Vertical views): the full row as one JSON
  // object; Save builds a Mongo replaceOne (or a per-changed-column SQL UPDATE)
  // and opens the normal review modal. With `rowIdx === -1` it is the INSERT
  // editor (insertOne / INSERT from the typed JSON). Mounted by ResultsGrid
  // while `flow.docEditor` is set.
  import type { EditFlow } from './EditFlow.svelte';
  import { dialogKeys } from './dialog-keys';

  interface Props {
    flow: EditFlow;
  }
  let { flow }: Props = $props();
  const inserting = $derived((flow.docEditor?.rowIdx ?? 0) < 0);
</script>

{#if flow.docEditor}
  <div
    class="cell-viewer-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) flow.docEditor = null;
    }}
  >
    <div
      class="cell-viewer"
      role="dialog"
      aria-modal="true"
      aria-label={inserting ? 'Insert document' : 'Edit document'}
      use:dialogKeys={() => (flow.docEditor = null)}
    >
      <div class="cv-head">
        {#if inserting}
          <span>Insert document <span class="dim">— {flow.engine === 'mongodb' ? 'insertOne' : 'INSERT'} is reviewed before it runs</span></span>
        {:else}
          <span>Edit document <span class="dim">— row is replaced/updated after review</span></span>
        {/if}
        <span class="grow"></span>
        <button class="icon-btn" onclick={() => (flow.docEditor = null)} aria-label="Close">✕</button>
      </div>
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        class="cv-edit mono"
        bind:value={flow.docEditor.draft}
        spellcheck="false"
        autofocus
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
  </div>
{/if}

<style>
  /* Scoped copy of the shared dialog rules (see CellViewer.svelte). */
  .cell-viewer-backdrop {
    position: fixed;
    inset: 0;
    z-index: 250;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
  }
  .cell-viewer {
    width: min(720px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .cv-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
    font-weight: 600;
  }
  /* In-viewer editor (JSON/long text): fills the same band as .cv-body. */
  .cv-edit {
    flex: 1;
    min-height: 220px;
    margin: 10px 14px 0;
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
    padding: 10px 14px;
  }
  .cv-err {
    color: var(--status-exited);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
