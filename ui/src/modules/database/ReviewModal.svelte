<script lang="ts">
  // Review-SQL modal (shared by cell edits, row duplication, deletes and the
  // doc editor). The textarea is the source of truth for what runs — it is
  // controlled: every keystroke goes back to the owner through `onsql`.
  // `diff` (path · before → after per change) is shown above it as a compact
  // table so a `$set`/`$unset` on a dotted path or a replaceOne reads as WHAT
  // changes, not as a statement to parse; editing the statement does not
  // update the table (it describes what the builder produced).
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import type { DiffLine } from './EditFlow.svelte';

  /** Rows shown before the table folds the rest into a count. */
  const DIFF_MAX = 200;

  interface Props {
    title: string;
    sql: string;
    diff?: DiffLine[];
    running: boolean;
    onsql: (sql: string) => void;
    onrun: () => void;
    onclose: () => void;
  }
  let { title, sql, diff, running, onsql, onrun, onclose }: Props = $props();

  const lines = $derived(diff ?? []);
  const shownLines = $derived(lines.slice(0, DIFF_MAX));
  // The row column only earns its place when the batch spans several rows.
  const multiRow = $derived(new Set(lines.map((d) => d.row)).size > 1);
  const OP_LABEL: Record<DiffLine['op'], string> = { set: '$set', unset: '$unset', rename: '$rename', cell: 'set' };

  // Esc (and the ✕) is the shared Modal's; a running statement can't be
  // dismissed mid-flight.
  function close(): void {
    if (!running) onclose();
  }

  function onReviewKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onrun();
    }
  }
</script>

<Modal {title} width={640} onclose={close}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="review-modal" onkeydown={onReviewKeydown}>
    <div class="review-body">
      {#if lines.length > 0}
        <div class="review-diff-wrap">
          <table class="review-diff mono">
            <thead>
              <tr>
                {#if multiRow}<th class="rd-row">row</th>{/if}
                <th class="rd-op"><span class="rd-badge">{lines.length} change{lines.length === 1 ? '' : 's'}</span></th>
                <th>path</th>
                <th>before</th>
                <th class="rd-arrow" aria-hidden="true"></th>
                <th>after</th>
              </tr>
            </thead>
            <tbody>
              {#each shownLines as d, i (i)}
                <tr class="op-{d.op}">
                  {#if multiRow}<td class="rd-row">{d.row < 0 ? 'new' : `#${d.row + 1}`}</td>{/if}
                  <td class="rd-op">{OP_LABEL[d.op]}</td>
                  <td class="rd-path">{d.path}</td>
                  <td class="rd-val" class:rd-absent={d.before === '∅'}>{d.before}</td>
                  <td class="rd-arrow" aria-hidden="true">→</td>
                  <td class="rd-val" class:rd-absent={d.after === '∅'}>{d.after}</td>
                </tr>
              {/each}
              {#if lines.length > DIFF_MAX}
                <tr><td colspan={multiRow ? 6 : 5} class="rd-more">+{lines.length - DIFF_MAX} more changes (all in the statement below)</td></tr>
              {/if}
            </tbody>
          </table>
        </div>
      {/if}
      <p class="review-hint">Review and edit the statement before running. This will run against the connection.</p>
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        class="review-sql mono"
        value={sql}
        oninput={(e) => onsql(e.currentTarget.value)}
        disabled={running}
        spellcheck="false"
        autofocus
        rows="5"
      ></textarea>
    </div>
    <div class="review-foot">
      <span class="review-kbd mono">⌘↵ to run · Esc to cancel</span>
      <span class="grow"></span>
      <button class="tb-btn" onclick={onclose} disabled={running}>Cancel</button>
      <button class="tb-btn primary" onclick={onrun} disabled={running || !sql.trim()}>
        <Icon name="play" size={11} />{running ? 'Running…' : 'Run'}
      </button>
    </div>
  </div>
</Modal>

<style>
  /* Scoped copy of the shared dialog rules (see CellViewer.svelte). */
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
    font-size: 11.5px;
    cursor: pointer;
  }
  .tb-btn:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent-text);
  }
  /* ── Review-SQL modal ── */
  .review-modal {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .review-body {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .review-hint {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  /* ── Change table (path · before → after) ── */
  .review-diff-wrap {
    max-height: 32vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .review-diff {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
  }
  .review-diff th {
    position: sticky;
    top: 0;
    text-align: start;
    padding: 4px 8px;
    font-weight: 600;
    color: var(--text-dim);
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  .review-diff td {
    padding: 3px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    vertical-align: top;
    word-break: break-word;
  }
  .review-diff tbody tr:last-child td {
    border-bottom: none;
  }
  .rd-badge {
    display: inline-block;
    padding: 0 6px;
    border-radius: 9px;
    font-size: var(--fs-xs);
    color: var(--accent-contrast);
    background: var(--accent);
  }
  .rd-row,
  .rd-op {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .rd-path {
    font-weight: 600;
    color: var(--accent-text);
    white-space: nowrap;
  }
  .rd-val {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rd-absent {
    color: var(--text-dim);
  }
  .rd-arrow {
    width: 1ch;
    color: var(--text-dim);
    padding: 3px 2px;
  }
  .rd-more {
    color: var(--text-dim);
    font-style: italic;
  }
  tr.op-unset .rd-path {
    color: var(--status-exited);
    text-decoration: line-through;
  }
  tr.op-rename .rd-path {
    color: var(--status-warn);
  }
  tr.op-set td.rd-val:last-child,
  tr.op-cell td.rd-val:last-child {
    color: var(--success);
  }
  .review-sql {
    width: 100%;
    resize: vertical;
    min-height: 92px;
    padding: 9px 11px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12px;
    line-height: 1.5;
    outline: none;
    white-space: pre;
    overflow: auto;
  }
  .review-sql:focus {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .review-sql:disabled {
    opacity: 0.6;
  }
  .review-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .review-kbd {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tb-btn.primary {
    border-color: transparent;
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
  }
  .tb-btn.primary:hover {
    color: var(--accent-contrast);
    background: color-mix(in srgb, var(--accent) 88%, black);
  }
  .tb-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
