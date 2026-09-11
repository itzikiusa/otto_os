<script lang="ts">
  // JSON result view: one JSON object PER ROW (not one big array) so row
  // boundaries are unmistakable — each row is its own bordered, numbered,
  // copyable block. Same data the server returned; only the rendering differs.
  import Icon from '../../lib/components/Icon.svelte';
  import JsonTree from './JsonTree.svelte';
  import type { QueryResult, DbEngine } from '../../lib/api/types';
  import type { EditFlow } from './EditFlow.svelte';
  import type { ExpansionState } from './expansion-plan';
  import { ALT_BATCH, copyText, prettyJson } from './results-format';

  interface Props {
    result: QueryResult;
    /** Filtered/sorted rows as objects, capped; `idx` is the ORIGINAL liveRows index. */
    objRows: { obj: Record<string, unknown>; idx: number }[];
    /** Column names with duplicates disambiguated (`name (2)`, …). */
    uniqueColNames: string[];
    engine: DbEngine | null;
    flow: EditFlow;
    mini: boolean;
    /** Records still drawable below the current batch (excludes the hard-capped tail). */
    altRemaining: number;
    viewTruncated: boolean;
    viewCap: number;
    /** Total rows in the view (the "Showing first N of M rows" note). */
    totalRows: number;
    onshowmore: () => void;
    /** Nested-document expansion state (unused until the Vertical rendering work). */
    expansion?: ExpansionState;
    /** "Compare two records" request from a record header (mounted by ResultsGrid). */
    oncompare?: (left: number, right: number) => void;
  }
  let {
    result,
    objRows,
    uniqueColNames,
    engine,
    flow,
    mini,
    altRemaining,
    viewTruncated,
    viewCap,
    totalRows,
    onshowmore,
    expansion,
    oncompare,
  }: Props = $props();
  // `engine`, `mini`, `expansion` and `oncompare` are accepted for the nested-
  // document rendering / record-compare work and unused here so far.
</script>

<div class="alt-view">
  {#if viewTruncated}<div class="alt-note dim">Showing first {viewCap} of {totalRows} rows.</div>{/if}
  {#each objRows as { obj, idx }, ri (ri)}
    <div class="jrec">
      <div class="jrec-head mono">
        <span class="jrec-n">#{ri + 1}</span>
        {#if flow.editable && !flow.reviewSql}
          <button class="jrec-copy" title="Edit this document (opens a review before running)" aria-label="Edit document" onclick={() => flow.openDocEditor(idx)}><Icon name="edit" size={10} /></button>
        {/if}
        <button class="jrec-copy" title="Copy this row as JSON" aria-label="Copy row JSON" onclick={() => copyText(prettyJson(obj))}><Icon name="file" size={10} /></button>
      </div>
      <!-- Collapsible tree, NOT a stringified blob: a closed branch renders
           one summary line, so a 90KB document costs a handful of nodes. -->
      <div class="alt-json mono"><JsonTree value={obj} /></div>
    </div>
  {/each}
  {#if altRemaining > 0}
    <button class="alt-more" onclick={onshowmore}>
      Show {Math.min(ALT_BATCH, altRemaining)} more · {altRemaining} not rendered
    </button>
  {/if}
</div>

<style>
  .alt-view {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 4px 2px;
  }
  .alt-note {
    font-size: 11px;
    padding: 4px 6px 8px;
  }
  /* Container for the collapsible tree (JsonTree owns its own token colours). */
  .alt-json {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text);
  }
  /* Batch pager for the non-virtualized alt views. */
  .alt-more {
    display: block;
    width: 100%;
    padding: 6px 10px;
    margin: 2px 0 10px;
    font-size: 11px;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .alt-more:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  /* Per-row JSON card (json view): a bordered, numbered block per row so each
     row's start/end is obvious. Mirrors the vertical view's .vrec idiom. */
  .jrec {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    margin-bottom: 8px;
    overflow: hidden;
  }
  .jrec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11px;
    padding: 3px 8px;
    border-bottom: 1px solid var(--border);
  }
  .jrec-copy {
    display: inline-flex;
    align-items: center;
    padding: 2px;
    color: var(--text-dim);
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .jrec-copy:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .jrec .alt-json {
    padding: 6px 8px;
  }

  /* Vertical / JSON views are the comfiest on a narrow phone — bump them too. */
  @media (max-width: 640px) {
    .alt-json {
      font-size: 13px;
    }
  }
</style>
