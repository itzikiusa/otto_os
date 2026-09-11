<script lang="ts">
  // Review-SQL modal (shared by cell edits, row duplication, deletes and the
  // doc editor). The textarea is the source of truth for what runs — it is
  // controlled: every keystroke goes back to the owner through `onsql`.
  // `diff` is accepted for the upcoming diff table and not rendered yet.
  import Icon from '../../lib/components/Icon.svelte';
  import type { DiffLine } from './EditFlow.svelte';
  import { dialogKeys } from './dialog-keys';

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

  function onReviewKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onrun();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="cell-viewer-backdrop"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) onclose();
  }}
  onkeydown={onReviewKeydown}
>
  <div
    class="review-modal"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    use:dialogKeys={onclose}
  >
    <div class="cv-head">
      <span>{title}</span>
      <button class="icon-btn" onclick={onclose} disabled={running} aria-label="Close">✕</button>
    </div>
    <div class="review-body">
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
</div>

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
    color: var(--accent);
  }
  .cell-viewer-backdrop {
    position: fixed;
    inset: 0;
    z-index: 250;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
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
  /* ── Review-SQL modal ── */
  .review-modal {
    width: min(640px, 92vw);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .review-body {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .review-hint {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-dim);
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
    padding: 10px 14px;
    border-top: 1px solid var(--border);
  }
  .review-kbd {
    font-size: 10px;
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
