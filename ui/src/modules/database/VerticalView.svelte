<script lang="ts">
  // Vertical result view: one bordered block per record, `field: value` rows
  // (like Postgres `\x` / ClickHouse FORMAT Vertical). Not virtualized — rows
  // arrive already batched (`objRows`) and grow on demand via `onshowmore`.
  import Icon from '../../lib/components/Icon.svelte';
  import JsonTree from './JsonTree.svelte';
  import type { QueryResult, DbEngine } from '../../lib/api/types';
  import type { EditFlow } from './EditFlow.svelte';
  import type { ExpansionState } from './expansion-plan';
  import { ALT_BATCH, cellStr, vvTree } from './results-format';

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
    <div class="vrec">
      <div class="vrec-head mono">
        #{ri + 1}
        {#if flow.editable && !flow.reviewSql}
          <button class="jrec-copy" title="Edit this record (opens a review before running)" aria-label="Edit record" onclick={() => flow.openDocEditor(idx)}><Icon name="edit" size={10} /></button>
        {/if}
      </div>
      {#each result.columns as _c, vci (vci)}
        {@const vName = uniqueColNames[vci]}
        {@const vVal = obj[vName]}
        <div class="vrow">
          <span class="vk mono">{vName}</span>
          {#if vVal === null || vVal === undefined}
            <span class="vv mono">∅</span>
          {:else if vvTree(vVal)}
            <!-- Embedded documents/arrays (and very long text) render as a
                 COLLAPSED tree. The old `cellStr` stringified them in full —
                 one ~87KB text node per record was the other half of the
                 freeze, and it made the record unreadable besides. -->
            <span class="vv mono tree"><JsonTree value={vVal} /></span>
          {:else}
            <span class="vv mono">{cellStr(vVal)}</span>
          {/if}
        </div>
      {/each}
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
  /* The record-header pencil (shared idiom with the JSON view's .jrec-head). */
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
  .vrec {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    margin-bottom: 8px;
    overflow: hidden;
  }
  .vrec-head {
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11px;
    padding: 3px 8px;
    border-bottom: 1px solid var(--border);
  }
  .vrow {
    display: grid;
    grid-template-columns: minmax(120px, 0.3fr) 1fr;
    gap: 10px;
    padding: 3px 8px;
    font-size: 12px;
  }
  .vrow:nth-child(even) {
    background: color-mix(in srgb, var(--text-dim) 4%, transparent);
  }
  .vk {
    color: var(--text-dim);
    font-weight: 600;
  }
  .vv {
    color: var(--text);
    word-break: break-word;
    white-space: pre-wrap;
  }
  /* A tree child manages its own layout — pre-wrap here would turn the markup's
     indentation into stray blank lines. */
  .vv.tree {
    white-space: normal;
    min-width: 0;
  }

  /* Vertical / JSON views are the comfiest on a narrow phone — bump them too. */
  @media (max-width: 640px) {
    .vk,
    .vv {
      font-size: 13px;
    }
  }
</style>
