<script lang="ts">
  import { focusOnMount } from '../../lib/focusOnMount';
  // The expandable cell viewer: full value in a modal, SQL formatting toggle,
  // Copy, and — when the cell belongs to an editable column — an in-viewer
  // editor whose Save hands the draft to the normal cell-edit review flow.
  // Mounted by ResultsGrid while `flow.viewer` is set; the shared Modal owns
  // the sheet, focus trap, Esc and the pushModal registration.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import type { EditFlow } from './EditFlow.svelte';
  import { copyText, fmtBytes } from './results-format';

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

  // A multi-MB value (a big Mongo document, a blob) laid out in one wrapping
  // <pre> took seconds to open. Show the head; "Show all" renders the rest on
  // request. Copy (and the editor) always use the FULL value.
  const VIEW_MAX = 256 * 1024;
  /** The viewer object the user asked to see in full (resets per cell). */
  let expandedFor = $state<object | null>(null);
  const clipped = $derived(
    !!flow.viewer && expandedFor !== flow.viewer && flow.viewerText.length > VIEW_MAX,
  );
  const shownText = $derived(clipped ? flow.viewerText.slice(0, VIEW_MAX) : flow.viewerText);
</script>

{#if flow.viewer}
  <Modal title="Cell value" width={720} onclose={() => (flow.viewer = null)}>
    <div class="cell-viewer">
      <div class="cv-tools">
        {#if flow.viewer.sql}
          <button
            class="btn small cv-toggle"
            class:active={flow.viewer.formatted}
            aria-pressed={!!flow.viewer.formatted}
            onclick={() => (flow.viewer && (flow.viewer.formatted = !flow.viewer.formatted))}
            title="Toggle SQL formatting"
          >
            <Icon name="grid" size={12} />{flow.viewer.formatted ? 'Formatted' : 'Raw'}
          </button>
        {/if}
        {#if flow.viewer.edit && !flow.viewerEditing}
          <button class="btn small" onclick={() => flow.startViewerEdit()} title="Edit this cell value">
            <Icon name="edit" size={12} />Edit
          </button>
        {/if}
        <button class="btn small" onclick={() => copyText(flow.viewerText, ['Copied', 'Full cell value copied'])} title="Copy full value"><Icon name="file" size={12} />Copy</button>
      </div>
      {#if nestedPending}
        <div class="cv-pending">Nested change pending on this field — saving a whole value here replaces it.</div>
      {/if}
      {#if flow.viewerEditing}
        <textarea dir="ltr" aria-label="Cell value"
          class="cv-edit mono"
          bind:value={flow.viewerDraft}
          spellcheck="false"
          use:focusOnMount
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
        <pre class="cv-body mono">{shownText}</pre>
        {#if clipped}
          <div class="cv-more">
            <span>Showing the first {fmtBytes(VIEW_MAX)} of {fmtBytes(flow.viewerText.length)} — Copy copies the full value.</span>
            <button class="btn small ghost" onclick={() => (expandedFor = flow.viewer)}>Show all</button>
          </div>
        {/if}
      {/if}
    </div>
  </Modal>
{/if}

<style>
  /* The Raw/Formatted toggle's pressed state (the button itself is the shared .btn.small). */
  .cv-toggle.active {
    border-color: var(--accent-line-strong);
    background: var(--accent-soft);
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
    color: var(--danger);
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cv-more {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .cv-pending {
    padding: 4px 8px;
    font-size: var(--fs-xs);
    color: var(--warning);
    background: color-mix(in srgb, var(--status-warn) 12%, transparent);
    border-radius: var(--radius-s);
  }
</style>
