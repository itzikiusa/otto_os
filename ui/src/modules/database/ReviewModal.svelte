<script lang="ts">
  import Badge from '../../lib/components/Badge.svelte';
  import { plural } from '../../lib/plural';
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
      // Same gate as the Run button: a second ⌘↵ while the first statement is
      // still in flight must not fire it again (an UPDATE/DELETE twice).
      if (!running && sql.trim()) onrun();
    }
  }
</script>

<Modal {title} width={640} onclose={close} dismissable={!running}>
  <!-- A keydown catcher (⌘↩ runs) around the dialog's own controls. -->
  <div class="review-modal" role="presentation" onkeydown={onReviewKeydown}>
    <div class="review-body">
      {#if lines.length > 0}
        <div class="review-diff-wrap">
          <table class="review-diff mono">
            <thead>
              <tr>
                {#if multiRow}<th class="rd-row">row</th>{/if}
                <th class="rd-op"><Badge tone="accent" label={plural(lines.length, 'change')} /></th>
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
      <textarea
        class="review-sql mono"
        value={sql}
        oninput={(e) => onsql(e.currentTarget.value)}
        disabled={running}
        spellcheck="false"
        data-autofocus
        rows="5"
      ></textarea>
    </div>
  </div>
  {#snippet footer()}
    <span class="review-kbd mono">⌘↵ to run · Esc to cancel</span>
    <span class="grow"></span>
    <button class="btn" onclick={onclose} disabled={running}>Cancel</button>
    <button class="btn primary" onclick={onrun} disabled={running || !sql.trim()}>
      <Icon name="play" size={12} />{running ? 'Running…' : 'Run'}
    </button>
  {/snippet}
</Modal>

<style>
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
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
    padding: 2px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    vertical-align: top;
    word-break: break-word;
  }
  .review-diff tbody tr:last-child td {
    border-bottom: none;
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
    padding: 2px 2px;
  }
  .rd-more {
    color: var(--text-dim);
    font-style: italic;
  }
  tr.op-unset .rd-path {
    color: var(--danger);
    text-decoration: line-through;
  }
  tr.op-rename .rd-path {
    color: var(--warning);
  }
  tr.op-set td.rd-val:last-child,
  tr.op-cell td.rd-val:last-child {
    color: var(--success);
  }
  .review-sql {
    width: 100%;
    resize: vertical;
    min-height: 92px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    line-height: 1.5;
    outline: none;
    white-space: pre;
    overflow: auto;
  }
  .review-sql:focus {
    border-color: var(--accent-line-strong);
  }
  .review-sql:disabled {
    opacity: 0.6;
  }
  .review-kbd {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
