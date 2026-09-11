<script lang="ts">
  // Vertical result view: one bordered block per record, `field: value` rows
  // (like Postgres `\x` / ClickHouse FORMAT Vertical) — with sub-documents as
  // NESTED rows (VerticalTree) instead of a collapsed blob. Not virtualized —
  // rows arrive already batched (`objRows`) and grow on demand via `onshowmore`.
  //
  // Expansion: every record is planned against the shared `expansion` state
  // (node budget, sticky per-path toggles, Expand/Collapse all) so opening
  // `meta.brand_id` in record 1 opens it in every record. Editing: the field
  // rows double-click into the typed inline editor and right-click into the
  // field menu (Set null / Delete / Rename / Add field…); the record menu (⋯)
  // adds whole-record actions (Insert document, Copy / Export JSON, Compare,
  // Replace via the JSON editor). Everything writes through `flow` → the
  // pending bar → the review modal; nothing here runs a statement.
  import Icon from '../../lib/components/Icon.svelte';
  import VerticalTree from './VerticalTree.svelte';
  import type { QueryResult, DbEngine } from '../../lib/api/types';
  import type { EditFlow } from './EditFlow.svelte';
  import type { FieldCtx } from './edit-types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import {
    estimateNodes,
    newExpansionState,
    planExpansion,
    setExpansionMode,
    type ExpansionState,
  } from './expansion-plan';
  import { ALT_BATCH, cellStr, copyText, isComplex, prettyJson, SET_EMPTY, SET_NULL } from './results-format';

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

  // ── Expansion plans ──────────────────────────────────────────────────────────
  const localExpansion = $state(newExpansionState());
  const exp = $derived(expansion ?? localExpansion);
  // One plan per drawn record; `version` is what a toggle bumps (the override
  // Map itself is edited in place). Cheap: a capped BFS per record.
  const plans = $derived.by<Set<string>[]>(() => {
    void exp.version;
    return objRows.map((r) => planExpansion(r.obj, exp));
  });
  const resettable = $derived.by(() => {
    void exp.version;
    return exp.mode !== 'budget' || exp.overrides.size > 0;
  });
  /** Expand-all warning threshold (rows the batch would draw). */
  const EXPAND_ALL_WARN = 20_000;
  function expandAll(): void {
    const total = objRows.reduce((n, r) => n + estimateNodes(r.obj, EXPAND_ALL_WARN), 0);
    if (total >= EXPAND_ALL_WARN) {
      toasts.info('Expanding everything', `${objRows.length} records with ~${total.toLocaleString()}+ fields — this can take a moment`);
    }
    setExpansionMode(exp, 'all');
  }

  // ── Record helpers ───────────────────────────────────────────────────────────
  /** Value a top-level row renders: the column's parked cell draft when there
   *  is one (SQL nested edits live there; the grid's drafts too), else stored. */
  function columnValue(idx: number, vci: number, live: unknown): { value: unknown; draft: boolean } {
    const d = flow.pendingValue(idx, vci);
    if (d === undefined) return { value: live, draft: false };
    if (d === SET_NULL || d === '') return { value: null, draft: true };
    if (d === SET_EMPTY) return { value: '', draft: true };
    if (isComplex(live)) {
      try {
        return { value: JSON.parse(d), draft: true };
      } catch {
        /* a scalar draft over a complex value — shown as text */
      }
    }
    return { value: d, draft: true };
  }
  /** New top-level fields parked as `$set` (phantom rows after the columns). */
  function phantomFields(idx: number): [string, unknown][] {
    return flow.pendingSetUnder(idx, '').filter(([k]) => !result.columns.some((c) => c.name === k));
  }
  /** The record's `_id` (or 1-based position) for file names. */
  function recordId(obj: Record<string, unknown>, ri: number): string {
    const id = obj._id;
    if (id === undefined || id === null) return String(ri + 1);
    const s = cellStr(id);
    const m = s.match(/^ObjectId\("(.+)"\)$/);
    return (m ? m[1] : s).replace(/[^\w.-]+/g, '_').slice(0, 64);
  }
  // Local copy of ResultsGrid's `download()` (results-format.ts is not this
  // view's to extend) — flagged for the reviewer to fold into one helper.
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
  function exportRecord(obj: Record<string, unknown>, ri: number): void {
    const coll = flow.editTable ?? flow.copyTarget?.table ?? 'record';
    download(prettyJson(obj), `${coll}-${recordId(obj, ri)}.json`, 'application/json');
  }

  // ── Compare (max 2; the diff itself is ResultsGrid's RecordDiff) ────────────
  let comparePick = $state<number | null>(null);
  function pickCompare(idx: number): void {
    if (comparePick === null) {
      comparePick = idx;
      toasts.info('Compare', 'Pick a second record from its ⋯ menu');
      return;
    }
    const left = comparePick;
    comparePick = null;
    if (left !== idx) oncompare?.(left, idx);
  }
  // A new result invalidates the pick (indices shift).
  $effect(() => {
    void result;
    comparePick = null;
  });

  // ── Menus ────────────────────────────────────────────────────────────────────
  const canEdit = $derived(flow.editable && !flow.reviewSql);

  async function addField(idx: number, colIdx: number, parentPath: string): Promise<void> {
    const name = (
      await confirmer.promptText(parentPath ? `New field under ${parentPath}` : 'New top-level field', {
        title: 'Add field',
        confirmLabel: 'Add',
        placeholder: 'field name',
      })
    )?.trim();
    if (!name) return;
    if (name.includes('.') || name.startsWith('$')) {
      toasts.warn('Invalid field name', 'Names can’t contain "." or start with "$"');
      return;
    }
    const path = parentPath ? `${parentPath}.${name}` : name;
    // Parked (as an empty string) so the row exists, then straight into its
    // editor — Escape there leaves the empty field, which Delete removes.
    flow.parkPath(idx, colIdx, path, { kind: 'string', raw: '' });
    flow.beginPathEdit(idx, colIdx, path);
  }
  async function renameField(ctx: FieldCtx): Promise<void> {
    const name = (
      await confirmer.promptText(`Rename ${ctx.path}`, {
        title: 'Rename field',
        confirmLabel: 'Rename',
        initial: ctx.label,
      })
    )?.trim();
    if (!name || name === ctx.label) return;
    if (name.includes('.') || name.startsWith('$')) {
      toasts.warn('Invalid field name', 'Names can’t contain "." or start with "$"');
      return;
    }
    const parent = ctx.path.includes('.') ? ctx.path.slice(0, ctx.path.lastIndexOf('.')) : '';
    flow.renamePath(ctx.rowIdx, ctx.colIdx, ctx.path, parent ? `${parent}.${name}` : name);
  }
  async function deleteField(ctx: FieldCtx): Promise<void> {
    const ok = await confirmer.ask(
      engine === 'mongodb'
        ? `Remove “${ctx.path}” from this document? ($unset — reviewed before it runs)`
        : `Remove “${ctx.path}” from this column's JSON? (reviewed before it runs)`,
      { title: 'Delete field', danger: true },
    );
    if (ok) flow.unsetPath(ctx.rowIdx, ctx.colIdx, ctx.path);
  }

  function fieldMenu(e: MouseEvent, ctx: FieldCtx): void {
    if (mini) return;
    const items: MenuItem[] = [];
    const editableHere = canEdit && (ctx.colIdx === -1 || flow.isEditableCell(ctx.colIdx));
    // SQL: a top-level row IS a column — it can be set, never removed/renamed.
    const sqlColumn = engine !== 'mongodb' && ctx.topLevel;
    const parent = ctx.path.includes('.') ? ctx.path.slice(0, ctx.path.lastIndexOf('.')) : '';
    if (editableHere) {
      if (!ctx.container) items.push({ label: 'Edit value', icon: 'edit', action: ctx.edit });
      items.push({
        label: 'Set null',
        icon: 'x',
        disabled: ctx.value === null,
        action: () => flow.parkPath(ctx.rowIdx, ctx.colIdx, ctx.path, { kind: 'null', raw: '' }),
      });
      if (!sqlColumn) {
        items.push({
          label: engine === 'mongodb' ? 'Delete field ($unset)…' : 'Delete field…',
          icon: 'trash',
          danger: true,
          action: () => void deleteField(ctx),
        });
        items.push({
          label: 'Rename field…',
          icon: 'edit',
          disabled: engine === 'mongodb' && ctx.inArray,
          action: () => void renameField(ctx),
        });
      }
      // Inside this container, or beside this field. A SQL row can't grow a
      // column, so only paths inside a JSON column offer it.
      const addParent = ctx.container ? ctx.path : parent;
      if (engine === 'mongodb' || addParent !== '') {
        items.push({
          label: ctx.container ? 'Add field inside…' : 'Add field here…',
          icon: 'plus',
          // A new TOP-LEVEL field belongs to no column (-1).
          action: () => void addField(ctx.rowIdx, addParent === '' ? -1 : ctx.colIdx, addParent),
        });
      }
      items.push({ separator: true });
    }
    items.push(
      { label: 'Copy path', icon: 'file', action: () => copyText(ctx.path) },
      {
        label: 'Copy value',
        icon: 'file',
        action: () =>
          copyText(ctx.value === null || ctx.value === undefined ? '' : isComplex(ctx.value) ? prettyJson(ctx.value) : cellStr(ctx.value)),
      },
    );
    if (sqlColumn && canEdit) {
      items.push({ separator: true }, { label: "Columns can't be removed from a row", disabled: true });
    }
    ctxMenu.show(e, items);
  }

  function recordMenu(e: MouseEvent, obj: Record<string, unknown>, idx: number, ri: number): void {
    const items: MenuItem[] = [];
    if (canEdit) {
      if (engine === 'mongodb') items.push({ label: 'Add field…', icon: 'plus', action: () => void addField(idx, -1, '') });
      items.push(
        { label: 'Insert document…', icon: 'plus', action: () => flow.openDocEditor(-1) },
        { label: 'Replace document (JSON)…', icon: 'edit', action: () => flow.openDocEditor(idx) },
        { separator: true },
      );
    }
    items.push(
      { label: 'Copy as JSON', icon: 'file', action: () => copyText(prettyJson(obj), ['Copied', 'Record JSON copied']) },
      { label: 'Export…', icon: 'arrowDown', action: () => exportRecord(obj, ri) },
    );
    if (oncompare) {
      items.push({
        label: comparePick === null ? 'Compare with…' : comparePick === idx ? 'Cancel compare' : 'Compare with the picked record',
        icon: 'split',
        action: () => (comparePick === idx ? (comparePick = null) : pickCompare(idx)),
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
      {#if canEdit}<span class="vv-hint dim"><Icon name="edit" size={10} />double-click a value to edit · right-click a field for more</span>{/if}
    </div>
  {/if}
  {#if viewTruncated}<div class="alt-note dim">Showing first {viewCap} of {totalRows} rows.</div>{/if}
  {#each objRows as { obj, idx }, ri (ri)}
    {@const plan = plans[ri] ?? new Set<string>()}
    <div class="vrec" class:compare-pick={comparePick === idx}>
      <div class="vrec-head mono">
        <span>#{ri + 1}</span>
        {#if comparePick === idx}<span class="vrec-tag">comparing</span>{/if}
        <span class="grow"></span>
        {#if canEdit}
          <button class="jrec-copy" title="Edit this record (opens a review before running)" aria-label="Edit record" onclick={() => flow.openDocEditor(idx)}><Icon name="edit" size={10} /></button>
        {/if}
        {#if !mini}
          <button class="jrec-copy vrec-more" title="Record actions" aria-label="Record actions" onclick={(e) => recordMenu(e, obj, idx, ri)}>⋯</button>
        {/if}
      </div>
      {#each result.columns as c, vci (vci)}
        {@const cv = columnValue(idx, vci, obj[uniqueColNames[vci]])}
        <VerticalTree
          value={cv.value}
          path={c.name}
          depth={0}
          label={uniqueColNames[vci]}
          expansion={exp}
          {plan}
          editable={canEdit}
          rowIdx={idx}
          colIdx={vci}
          {flow}
          {engine}
          cellDraft={cv.draft}
          onfieldmenu={fieldMenu}
        />
      {/each}
      {#each phantomFields(idx) as [k] (k)}
        <VerticalTree value={undefined} path={k} depth={0} label={k} expansion={exp} {plan} editable={canEdit} rowIdx={idx} colIdx={-1} {flow} {engine} onfieldmenu={fieldMenu} />
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
  /* Expand / Collapse / Reset strip above the records. */
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
  .vv-hint {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: auto;
    font-size: 10.5px;
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
  /* The record-header pencil / ⋯ (shared idiom with the JSON view's .jrec-head). */
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
  .vrec-more {
    font-size: 13px;
    line-height: 1;
    padding: 0 5px;
  }
  .vrec {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    margin-bottom: 8px;
    overflow: hidden;
  }
  .vrec.compare-pick {
    border-color: color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .vrec-head {
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11px;
    padding: 3px 8px;
    border-bottom: 1px solid var(--border);
  }
  .vrec-tag {
    font-size: 10px;
    color: var(--accent);
  }
</style>
