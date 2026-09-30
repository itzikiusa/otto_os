<script lang="ts">
  // Whole-document editor (JSON / Vertical views): the full row as one JSON
  // object; Save builds a Mongo replaceOne (or a per-changed-column SQL UPDATE)
  // and opens the normal review modal. With `rowIdx === -1` it is the INSERT
  // editor (insertOne / INSERT from the typed JSON). Mounted by ResultsGrid
  // while `flow.docEditor` is set, inside the shared Modal. The draft lives in
  // a CodeEditor (JSON mode) so a large document is searchable: ⌘F and the
  // Find button open its search panel; Esc there closes the panel, not the modal.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import type { EditFlow } from './EditFlow.svelte';
  import { claimFind, keepEditorEsc } from './doc-modal';

  interface Props {
    flow: EditFlow;
  }
  let { flow }: Props = $props();
  const inserting = $derived((flow.docEditor?.rowIdx ?? 0) < 0);

  let editor = $state<ReturnType<typeof CodeEditor> | null>(null);
  let wrap = $state(false);
  // The editor seeds from the draft once; later `draft` writes are its own
  // edits echoing back (a reopen remounts this component).
  const initial = untrack(() => flow.docEditor?.draft ?? '');

  $effect(() => claimFind(() => editor?.openSearch()));
  // After the Modal's own initial focus (it only knows form controls).
  $effect(() => {
    if (!editor) return;
    const raf = requestAnimationFrame(() => editor?.focus());
    return () => cancelAnimationFrame(raf);
  });
</script>

{#if flow.docEditor}
  <Modal title={inserting ? 'Insert document' : 'Edit document'} width={720} onclose={() => (flow.docEditor = null)}>
    <div class="cell-viewer">
      <div class="cv-tools">
        <p class="cv-hint">
          {#if inserting}
            {flow.engine === 'mongodb' ? 'insertOne' : 'INSERT'} is reviewed before it runs.
          {:else}
            The row is replaced/updated after review.
          {/if}
        </p>
        <span class="grow"></span>
        <button class="icon-btn" onclick={() => editor?.openSearch()} aria-label="Find in document" title="Find in document (⌘F)">
          <Icon name="search" size={13} />
        </button>
        <button class="icon-btn wrap-toggle" aria-pressed={wrap} onclick={() => (wrap = !wrap)} aria-label="Wrap long lines" title="Wrap long lines">
          <Icon name="text" size={13} />
        </button>
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="cv-edit" onkeydown={keepEditorEsc}>
        <CodeEditor
          bind:this={editor}
          findOwner={false}
          path="document.json"
          root=""
          content={initial}
          readOnly={false}
          {wrap}
          highlightLineLimit={20000}
          onchange={(v) => { if (flow.docEditor) flow.docEditor.draft = v; }}
          onsubmit={() => flow.saveDocEdit()}
        />
      </div>
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
  .cv-tools {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .cv-hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .wrap-toggle[aria-pressed='true'] {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  /* The editor band: a fixed height (CodeMirror virtualizes inside it), capped
     so the sheet's own max-height still leaves the footer on screen. */
  .cv-edit {
    height: clamp(220px, 60vh, 560px);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .cv-foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cv-err {
    color: var(--status-exited);
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
