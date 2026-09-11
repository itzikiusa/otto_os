<script lang="ts">
  // JSON result view: one JSON object PER ROW (not one big array) so row
  // boundaries are unmistakable — each row is its own bordered, numbered,
  // copyable block. Same data the server returned; only the rendering differs.
  // Disclosure follows the same expansion plan as the Vertical view (node
  // budget + sticky per-path toggles + Expand/Collapse all), so what is open
  // here is open there.
  import Icon from '../../lib/components/Icon.svelte';
  import JsonTree from './JsonTree.svelte';
  import type { QueryResult, DbEngine } from '../../lib/api/types';
  import type { EditFlow } from './EditFlow.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import {
    estimateNodes,
    newExpansionState,
    planExpansion,
    setExpansionMode,
    type ExpansionState,
  } from './expansion-plan';
  import { ALT_BATCH, cellStr, copyText, prettyJson } from './results-format';

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
    /** Shared nested-document expansion state (ResultsGrid's `$state`); a
     *  local one is used when absent so the view works standalone. */
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
  // `uniqueColNames` / `engine` are accepted for prop parity with VerticalView
  // (the JSON view renders `obj`, already keyed by those names) and unused.

  // ── Expansion plans (see VerticalView — same state, same planner) ───────────
  const localExpansion = $state(newExpansionState());
  const exp = $derived(expansion ?? localExpansion);
  const plans = $derived.by<Set<string>[]>(() => {
    void exp.version;
    return objRows.map((r) => planExpansion(r.obj, exp));
  });
  const resettable = $derived.by(() => {
    void exp.version;
    return exp.mode !== 'budget' || exp.overrides.size > 0;
  });
  const EXPAND_ALL_WARN = 20_000;
  function expandAll(): void {
    const total = objRows.reduce((n, r) => n + estimateNodes(r.obj, EXPAND_ALL_WARN), 0);
    if (total >= EXPAND_ALL_WARN) {
      toasts.info('Expanding everything', `${objRows.length} records with ~${total.toLocaleString()}+ fields — this can take a moment`);
    }
    setExpansionMode(exp, 'all');
  }

  // ── Record menu (⋯): insert / export / compare — the same set as Vertical ──
  const canEdit = $derived(flow.editable && !flow.reviewSql);
  function recordId(obj: Record<string, unknown>, ri: number): string {
    const id = obj._id;
    if (id === undefined || id === null) return String(ri + 1);
    const s = cellStr(id);
    const m = s.match(/^ObjectId\("(.+)"\)$/);
    return (m ? m[1] : s).replace(/[^\w.-]+/g, '_').slice(0, 64);
  }
  // Local copy of ResultsGrid's `download()` (see VerticalView) — flagged for
  // the reviewer to fold into one helper.
  function download(text: string, name: string, mime: string): void {
    const blob = new Blob([text], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1500);
  }
  let comparePick = $state<number | null>(null);
  $effect(() => {
    void result;
    comparePick = null;
  });
  function recordMenu(e: MouseEvent, obj: Record<string, unknown>, idx: number, ri: number): void {
    const items: MenuItem[] = [];
    if (canEdit) {
      items.push(
        { label: 'Insert document…', icon: 'plus', action: () => flow.openDocEditor(-1) },
        { label: 'Replace document (JSON)…', icon: 'edit', action: () => flow.openDocEditor(idx) },
        { separator: true },
      );
    }
    items.push({
      label: 'Export…',
      icon: 'arrowDown',
      action: () =>
        download(prettyJson(obj), `${flow.editTable ?? flow.copyTarget?.table ?? 'record'}-${recordId(obj, ri)}.json`, 'application/json'),
    });
    if (oncompare) {
      items.push({
        label: comparePick === null ? 'Compare with…' : comparePick === idx ? 'Cancel compare' : 'Compare with the picked record',
        icon: 'split',
        action: () => {
          if (comparePick === idx) comparePick = null;
          else if (comparePick === null) {
            comparePick = idx;
            toasts.info('Compare', 'Pick a second record from its ⋯ menu');
          } else {
            const left = comparePick;
            comparePick = null;
            oncompare(left, idx);
          }
        },
      });
    }
    ctxMenu.show(e, items);
  }
</script>

<div class="alt-view">
  {#if !mini}
    <div class="vv-tools">
      <button class="vv-tool" onclick={expandAll} title="Open every nested field of the drawn records">Expand all</button>
      <button class="vv-tool" onclick={() => setExpansionMode(exp, 'none')} title="Close every nested field">Collapse all</button>
      <button class="vv-tool" onclick={() => setExpansionMode(exp, 'budget')} title="Back to the default: open what fits the node budget" disabled={!resettable}>Reset</button>
    </div>
  {/if}
  {#if viewTruncated}<div class="alt-note dim">Showing first {viewCap} of {totalRows} rows.</div>{/if}
  {#each objRows as { obj, idx }, ri (ri)}
    <div class="jrec" class:compare-pick={comparePick === idx}>
      <div class="jrec-head mono">
        <span class="jrec-n">#{ri + 1}</span>
        {#if comparePick === idx}<span class="jrec-tag">comparing</span>{/if}
        <span class="grow"></span>
        {#if canEdit}
          <button class="jrec-copy" title="Edit this document (opens a review before running)" aria-label="Edit document" onclick={() => flow.openDocEditor(idx)}><Icon name="edit" size={10} /></button>
        {/if}
        <button class="jrec-copy" title="Copy this row as JSON" aria-label="Copy row JSON" onclick={() => copyText(prettyJson(obj))}><Icon name="file" size={10} /></button>
        {#if !mini}
          <button class="jrec-copy jrec-more" title="Record actions" aria-label="Record actions" onclick={(e) => recordMenu(e, obj, idx, ri)}>⋯</button>
        {/if}
      </div>
      <!-- Collapsible tree, NOT a stringified blob: a closed branch renders
           one summary line, so a 90KB document costs a handful of nodes.
           Controlled by the shared plan (sticky toggles across records). -->
      <div class="alt-json mono"><JsonTree value={obj} path="" plan={plans[ri] ?? new Set<string>()} expansion={exp} /></div>
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
  /* Expand / Collapse / Reset strip above the records (same as VerticalView). */
  .vv-tools {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px 8px;
    flex-wrap: wrap;
  }
  .vv-tool {
    height: 20px;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .vv-tool:hover:not(:disabled) {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .vv-tool:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .jrec.compare-pick {
    border-color: color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .jrec-tag {
    font-size: 10px;
    color: var(--accent);
  }
  .jrec-more {
    font-size: 13px;
    line-height: 1;
    padding: 0 5px;
  }
  .jrec-head {
    display: flex;
    align-items: center;
    gap: 6px;
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
