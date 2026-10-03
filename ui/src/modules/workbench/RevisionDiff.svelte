<script lang="ts">
  // A revision diff: the daemon's line-diff stats (+N −M) over the shared
  // DiffView (which renders `before` → `after` client-side).
  import DiffView from '../../lib/components/DiffView.svelte';
  import { diffWorkbenchRevisions } from '../../lib/api/workbench';

  interface Props {
    ws: string;
    docId: string;
    /** Older side (a revision seq). */
    from: number;
    /** Newer side: a seq, or undefined = the current content. */
    to?: number;
    before: string;
    after: string;
    fromLabel: string;
    toLabel: string;
  }
  let { ws, docId, from, to, before, after, fromLabel, toLabel }: Props = $props();

  let stats: { added: number; removed: number } | null = $state(null);

  $effect(() => {
    const key = `${ws}/${docId}/${from}/${to ?? 'cur'}`;
    void after; // re-count when the current text changes (debounced by save)
    stats = null;
    let gone = false;
    diffWorkbenchRevisions(ws, docId, from, to)
      .then((d) => {
        if (!gone && key === `${ws}/${docId}/${from}/${to ?? 'cur'}`) stats = { added: d.added, removed: d.removed };
      })
      .catch(() => {
        /* stats are decorative; the diff below still renders */
      });
    return () => {
      gone = true;
    };
  });
</script>

<div class="wb-rd" data-testid="wb-rev-diff">
  <div class="wb-rd-head">
    <span class="wb-rd-side">{fromLabel}</span>
    <span aria-hidden="true">→</span>
    <span class="wb-rd-side">{toLabel}</span>
    {#if stats}
      <span class="wb-rd-stats">
        <span class="add">+{stats.added}</span>
        <span class="del">−{stats.removed}</span>
      </span>
    {/if}
  </div>
  {#if before === after}
    <p class="wb-rd-same">No differences.</p>
  {:else}
    <div class="wb-rd-body"><DiffView {before} {after} mode="line" contextLines={3} /></div>
  {/if}
</div>

<style>
  .wb-rd {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .wb-rd-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-wrap: wrap;
  }
  .wb-rd-side {
    font-weight: 600;
    color: var(--text);
  }
  .wb-rd-stats {
    margin-inline-start: auto;
    display: flex;
    gap: 6px;
    font-family: var(--font-mono);
  }
  .add {
    color: var(--success);
  }
  .del {
    color: var(--danger);
  }
  .wb-rd-body {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: auto;
    max-height: 50vh;
  }
  .wb-rd-same {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
