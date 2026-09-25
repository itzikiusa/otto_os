<script lang="ts">
  // The expandable cell viewer: full value in a modal, SQL formatting toggle,
  // Copy, and — when the cell belongs to an editable column — an in-viewer
  // editor whose Save hands the draft to the normal cell-edit review flow.
  // Mounted by ResultsGrid while `flow.viewer` is set; the shared Modal owns
  // the sheet, focus trap, Esc and the pushModal registration.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import type { EditFlow } from './EditFlow.svelte';
  import { copyText } from './results-format';

  interface Props {
    flow: EditFlow;
  }
  let { flow }: Props = $props();
  // A path-level change parked on this column (Vertical view): saving a
  // whole-cell draft from here replaces it (EditFlow's conflict rule).
  const nestedPending = $derived.by(() => {
    const e = flow.viewer?.edit;
    const name = e ? flow.result?.columns[e.colIdx]?.name : undefined;
    return !!e && !!name && flow.pendingValue(e.rowIdx, e.colIdx) === undefined && flow.hasPendingUnder(e.rowIdx, name);
  });
</script>

{#if flow.viewer}
  <Modal title="Cell value" width={720} onclose={() => (flow.viewer = null)}>
    <div class="cell-viewer">
      <div class="cv-tools">
        {#if flow.viewer.sql}
          <button
            class="tb-btn"
            class:active={flow.viewer.formatted}
            onclick={() => (flow.viewer && (flow.viewer.formatted = !flow.viewer.formatted))}
            title="Toggle SQL formatting"
          >
            <Icon name="grid" size={12} />{flow.viewer.formatted ? 'Formatted' : 'Raw'}
          </button>
        {/if}
        {#if flow.viewer.edit && !flow.viewerEditing}
          <button class="tb-btn" onclick={() => flow.startViewerEdit()} title="Edit this cell value">
            <Icon name="edit" size={12} />Edit
          </button>
        {/if}
        <button class="tb-btn" onclick={() => copyText(flow.viewerText, ['Copied', 'Full cell value copied'])} title="Copy full value"><Icon name="file" size={12} />Copy</button>
      </div>
      {#if nestedPending}
        <div class="cv-pending">Nested change pending on this field — saving a whole value here replaces it.</div>
      {/if}
      {#if flow.viewerEditing}
        <!-- svelte-ignore a11y_autofocus -->
        <textarea
          class="cv-edit mono"
          bind:value={flow.viewerDraft}
          spellcheck="false"
          autofocus
          onkeydown={(e) => {
            if (e.key === 'Escape') { e.stopPropagation(); flow.viewerEditing = false; flow.viewerErr = null; }
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) flow.saveViewerEdit();
          }}
        ></textarea>
        <div class="cv-foot">
          {#if flow.viewerErr}<span class="cv-err">{flow.viewerErr}</span>{/if}
          <span class="grow"></span>
          <button class="btn small ghost" onclick={() => { flow.viewerEditing = false; flow.viewerErr = null; }}>Cancel</button>
          <button class="btn small primary" onclick={() => flow.saveViewerEdit()} title="Validate and review the update (⌘⏎)">Save…</button>
        </div>
      {:else}
        <pre class="cv-body mono">{flow.viewerText}</pre>
      {/if}
    </div>
  </Modal>
{/if}

<style>
  /* Scoped copy of the shared dialog rules — DocEditor and ReviewModal each
     carry their own (Svelte styles don't cross components, and a global sheet
     would leak into brokers/ClusterViewer's .cv-head). */
  .tb-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .tb-btn:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent-text);
  }
  .tb-btn.active {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent-text);
  }
  .cell-viewer {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
  }
  .cv-tools {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
  }
  .cv-body {
    margin: 0;
    overflow: auto;
    font-size: var(--fs-s);
    line-height: 1.55;
    user-select: text;
    white-space: pre-wrap;
    word-break: break-word;
  }
  /* In-viewer editor (JSON/long text): fills the same band as .cv-body. */
  .cv-edit {
    flex: 1;
    min-height: 220px;
    padding: 10px;
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cv-pending {
    padding: 4px 8px;
    font-size: var(--fs-xs);
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 12%, transparent);
    border-radius: var(--radius-s);
  }
</style>
