// The inline-edit flow behind the results views: editability detection, cell
// drafts parked as pending changes, row selection + delete, "Copy as INSERT",
// the cell viewer / whole-document editor, and the Review-SQL modal that every
// write funnels through. ONE instance per ResultsGrid; the views (GridView /
// VerticalView / JsonView / CellViewer / DocEditor / ReviewModal) read and
// call it through their `flow` prop.
//
// Runes live on class fields only — a `.svelte.ts` class must not create
// effects at construction — so ResultsGrid keeps two thin `$effect`s: one that
// pushes the inputs via `update()`, one that re-runs `resolveTarget()` on the
// statement / connection / column-shape dependencies. Every public method is a
// plain prototype method (unbound): call it as `() => flow.method()`, never
// pass `flow.method` itself as a handler.
import type { QueryResult, DbForeignKey, DbEngine } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { database } from '../../lib/stores/database.svelte';
import {
  cellStr,
  compactJson,
  copyText,
  formatSql,
  isComplex,
  looksLikeSql,
  prettyJson,
  SET_EMPTY,
  SET_NULL,
} from './results-format';
import { adapterFor, type DiffLine, type EditCtx, type EditTarget, type RowPatch } from './edit-types';
import { parseSimpleSelect, qid, valueLiteral, whereByPk } from './edit-sql';
import { mongoCollectionForEdit, mongoIdFilterFor } from './edit-mongo';

export type { TypedKind, TypedValue, RowPatch, DiffLine } from './edit-types';
export { SET_NULL, SET_EMPTY } from './results-format';

/** Everything the flow needs from ResultsGrid, pushed on every change. The
 *  arrays are replaced wholesale (never mutated in place), so they are held as
 *  `$state.raw` — no deep proxy over 100k rows. */
export interface EditFlowInputs {
  result: QueryResult | null;
  liveRows: unknown[][];
  statement: string | undefined;
  connectionId: string | null | undefined;
  engine: DbEngine | null;
  canModify: boolean;
  uniqueColNames: string[];
  mini: boolean;
  /** liveRows indices in the CURRENT visible (filtered + sorted) order —
   *  shift-click ranges and "select all in view" follow it. */
  viewOrder: number[];
  /** Number of result sets in the batch (>1 = multi-statement, never editable). */
  resultCount: number;
}

export class EditFlow {
  // ── Inputs ─────────────────────────────────────────────────────────────────
  result: QueryResult | null = $state.raw(null);
  liveRows: unknown[][] = $state.raw([]);
  statement: string | undefined = $state.raw(undefined);
  connectionId: string | null | undefined = $state.raw(undefined);
  engine: DbEngine | null = $state.raw(null);
  canModify: boolean = $state.raw(false);
  uniqueColNames: string[] = $state.raw([]);
  mini: boolean = $state.raw(false);
  viewOrder: number[] = $state.raw([]);
  resultCount: number = $state.raw(0);

  update(i: EditFlowInputs): void {
    this.result = i.result;
    this.liveRows = i.liveRows;
    this.statement = i.statement;
    this.connectionId = i.connectionId;
    this.engine = i.engine;
    this.canModify = i.canModify;
    this.uniqueColNames = i.uniqueColNames;
    this.mini = i.mini;
    this.viewOrder = i.viewOrder;
    this.resultCount = i.resultCount;
  }

  // ── Editability detection ──────────────────────────────────────────────────
  // Editable iff: a connection id is present AND the statement is a plain
  // single-table SELECT (no JOIN/GROUP BY/UNION/DISTINCT/aggregate) AND the
  // table has exactly one primary-key column present in the result columns.
  editDb: string | null = $state(null);
  editTable: string | null = $state(null);
  editPkCols: string[] = $state([]); // pk column name(s) (when editable)
  editReason: string | null = $state(null); // why editing is unavailable
  // Foreign keys of the resolved single-table result (0003a in-grid FK nav).
  // Populated alongside the PK in resolveTarget; reused (no extra fetch).
  editFks: DbForeignKey[] = $state([]);
  /** Generation counter for `resolveTarget` — a stale `fetchObject` reply must
   *  never overwrite a newer target. */
  private targetGen = 0;

  readonly editable: boolean = $derived(this.canModify && this.editPkCols.length > 0 && this.editTable !== null);

  /** Target table/collection for "Copy as INSERT".
   *
   *  Deliberately NOT the same gate as `editable`. Editing needs a primary key
   *  because it has to TARGET an existing row; generating INSERTs only needs a
   *  name plus the values already on screen. Sharing the gate meant the action
   *  disappeared for every ClickHouse table whose `is_in_primary_key` doesn't
   *  resolve (Log/Memory engines, views) — and appeared on Mongo only to emit
   *  SQL. Resolved synchronously: no `object_detail` round-trip needed. */
  readonly copyTarget: { db: string | null; table: string } | null = $derived.by(() => {
    const sql = this.statement;
    const result = this.result;
    if (!sql || !this.connectionId || !result || result.columns.length === 0) return null;
    if (result.masked) return null; // INSERTs of redacted placeholders = data loss
    if (this.engine === 'mongodb') {
      const coll = mongoCollectionForEdit(sql);
      return coll ? { db: database.activeDb, table: coll } : null;
    }
    if (database.capabilities?.sql !== true) return null; // Redis etc.
    const parsed = parseSimpleSelect(sql);
    if (!parsed) return null;
    const db = parsed.db ?? (database.schemaRoot.find((n) => n.kind === 'database')?.label ?? null);
    return { db, table: parsed.table };
  });

  /** Resolve the primary key for the current statement/connection/result.
   *  Called from a ResultsGrid `$effect` — the synchronous prefix reads every
   *  dependency the old effect tracked (statement, connection, columns, the
   *  active DB, capabilities, schema root). */
  async resolveTarget(): Promise<void> {
    const gen = ++this.targetGen;
    // dependencies
    const sql = this.statement;
    const conn = this.connectionId;
    const cols = this.result?.columns;
    this.editDb = null;
    this.editTable = null;
    this.editPkCols = [];
    this.editReason = null;
    this.editFks = [];
    if (!sql || !conn || !cols || cols.length === 0) return;

    // A multi-statement batch is never editable — the shown result set can't be
    // safely attributed to one statement's table (see parseSimpleSelect).
    if (this.resultCount > 1 || /;\s*\S/.test(sql.trim().replace(/;\s*$/, ''))) {
      this.editReason = 'Editing is unavailable for multi-statement batches.';
      return;
    }
    // Masked values are REDACTED placeholders — writing them back would destroy
    // the real data, so a masked result is read-only.
    if (this.result?.masked) {
      this.editReason = 'Editing is disabled while server-side masking is applied.';
      return;
    }

    // SQL engines only beyond here (Redis etc. aren't editable) — Mongo resolves
    // synchronously through its adapter. The Redis stub adapter is wired but
    // unreachable until this gate is lifted with its implementation.
    if (this.engine !== 'mongodb' && database.capabilities?.sql !== true) return;

    const r = adapterFor(this.engine).target(
      sql,
      cols.map((c) => c.name),
      { engine: this.engine, activeDb: database.activeDb },
    );
    if (!r.target) {
      this.editReason = r.reason;
      return;
    }
    if (this.engine === 'mongodb') {
      this.editTable = r.target.table;
      this.editPkCols = r.target.pkCols;
      this.editDb = r.target.db;
      this.editReason = null;
      return;
    }

    // Build a default db from the schema root when the SQL omits it.
    const dbName =
      r.target.db ??
      (database.schemaRoot.find((n) => n.kind === 'database')?.label ?? null);
    const table = r.target.table;
    const path = dbName ? `db:${dbName}/table:${table}` : `table:${table}`;

    const detail = await database.fetchObject(path);
    if (gen !== this.targetGen || !detail) return;
    // Need a primary key (one or more columns), all present in the result so
    // we can target the exact row. Composite keys are supported.
    if (detail.primary_key.length === 0) {
      this.editReason = `“${table}” has no primary key, so rows can't be safely targeted for edits.`;
      return;
    }
    const missing = detail.primary_key.filter((pk) => !cols.some((c) => c.name === pk));
    if (missing.length > 0) {
      const plural = detail.primary_key.length > 1 ? 's' : '';
      this.editReason = `Include the primary key column${plural} (${detail.primary_key.join(', ')}) in your SELECT to enable editing.`;
      return;
    }
    this.editDb = dbName;
    this.editTable = table;
    this.editPkCols = detail.primary_key;
    this.editFks = detail.foreign_keys ?? [];
    this.editReason = null;
  }

  /** Adapter context over the given target and the current columns/rows. */
  private ctx(target: EditTarget): EditCtx {
    return {
      engine: this.engine,
      columns: this.result?.columns ?? [],
      liveRows: this.liveRows,
      target,
      qid: (s) => qid(this.engine, s),
    };
  }
  /** Context over the resolved EDIT target (null until one is resolved). */
  private editCtx(): EditCtx | null {
    if (!this.editTable) return null;
    return this.ctx({ db: this.editDb, table: this.editTable, pkCols: this.editPkCols });
  }

  // ── Inline editing ─────────────────────────────────────────────────────────
  // Edits are NOT applied directly. Committing a cell PARKS it as a pending
  // change (the cell renders the draft with a dirty marker) so several fields —
  // in one row or across rows — are prepared as ONE statement per row, every
  // changed column in a single SET/$set. "Review & apply" (the bar above the
  // footer) builds the statements and opens the "Review SQL" modal; nothing
  // runs until the user confirms there. After a successful run the grid
  // refreshes by re-running the active query, so values reflect the database
  // (no optimistic patching).
  editing: { rowIdx: number; colIdx: number; value: string } | null = $state(null);

  // Pending cell drafts, keyed by liveRows index (row → col → raw draft).
  // Any upstream result change shifts the indexes, so pending edits are
  // cleared alongside the row selection (resetForResult).
  pendingEdits: Map<number, Map<number, string>> = $state(new Map());
  readonly pendingCells: number = $derived([...this.pendingEdits.values()].reduce((n, m) => n + m.size, 0));

  pendingValue(rowIdx: number, colIdx: number): string | undefined {
    return this.pendingEdits.get(rowIdx)?.get(colIdx);
  }
  discardPending(): void {
    this.pendingEdits = new Map();
  }

  /** Park a pending value directly (the context menu's Set NULL / Set empty
   *  string) — same review flow as a typed draft, just skipping the input. */
  parkValue(rowIdx: number, colIdx: number, value: string): void {
    const next = new Map(this.pendingEdits);
    const row = new Map(next.get(rowIdx) ?? []);
    row.set(colIdx, value);
    next.set(rowIdx, row);
    this.pendingEdits = next;
  }

  isEditableCell(colIdx: number): boolean {
    if (!this.editable) return false;
    const name = this.result?.columns[colIdx]?.name;
    return !!name && !this.editPkCols.includes(name); // PK column(s) read-only
  }

  beginEdit(rowIdx: number, colIdx: number): void {
    if (!this.isEditableCell(colIdx) || this.reviewSql) return;
    // Re-editing a parked cell resumes its draft, not the stored value; the
    // Set-NULL/empty sentinels resume as an empty input.
    const parked = this.pendingValue(rowIdx, colIdx);
    const draft = parked === SET_NULL || parked === SET_EMPTY ? '' : parked;
    const v = this.liveRows[rowIdx]?.[colIdx];
    this.editing = {
      rowIdx,
      colIdx,
      value: draft ?? (v === null || v === undefined ? '' : cellStr(v)),
    };
  }
  cancelEdit(): void {
    this.editing = null;
  }

  /** Park the in-progress cell edit as a pending change. A draft that matches
   * the stored value again un-parks the cell. The statement is built later, in
   * `reviewPending` — every parked column of a row in ONE UPDATE / updateOne. */
  commitEdit(): void {
    if (!this.editing || !this.result || !this.editTable || this.editPkCols.length === 0) {
      this.editing = null;
      return;
    }
    const { rowIdx, colIdx, value } = this.editing;
    const prev = this.liveRows[rowIdx][colIdx];
    const prevStr = prev === null || prev === undefined ? '' : cellStr(prev);
    const next = new Map(this.pendingEdits);
    const row = new Map(next.get(rowIdx) ?? []);
    if (value === prevStr) row.delete(colIdx);
    else row.set(colIdx, value);
    if (row.size === 0) next.delete(rowIdx);
    else next.set(rowIdx, row);
    this.pendingEdits = next;
    this.editing = null;
  }

  /** Build ONE statement per pending row — every parked column in a single
   * SET (`$set` for Mongo; ClickHouse uses `ALTER TABLE … UPDATE`, a mutation)
   * — and open the review modal. Multiple rows become a multi-statement batch
   * (each driver splits and runs them in order). */
  reviewPending(): void {
    if (!this.result || !this.editTable || this.pendingEdits.size === 0) return;
    const ctx = this.editCtx();
    if (!ctx) return;
    const rows = [...this.pendingEdits.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([rowIdx, cells]): { rowIdx: number; patch: RowPatch } => ({
        rowIdx,
        patch: { cells, set: new Map(), unset: new Set(), rename: new Map() },
      }));
    const built = adapterFor(this.engine).buildUpdate(rows, ctx);
    if (built) this.openReview(built.title, built.sql, built.diff);
  }

  /** Build an INSERT / insertOne cloning a row (identity omitted so it
   * regenerates) and open the review modal. */
  duplicateRow(rowIdx: number): void {
    if (!this.result || !this.editTable || this.editPkCols.length === 0) return;
    const ctx = this.editCtx();
    if (!ctx) return;
    const built = adapterFor(this.engine).buildDuplicate(rowIdx, ctx);
    if (built) this.openReview(built.title, built.sql);
  }

  onEditKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      this.commitEdit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      this.cancelEdit();
    }
  }

  // ── Review-SQL modal (shared by cell edits + row duplication) ──────────────
  // The textarea is the source of truth for what runs.
  reviewSql: { title: string; sql: string; diff?: DiffLine[] } | null = $state(null);
  runningReview: boolean = $state(false);

  openReview(title: string, sql: string, diff?: DiffLine[]): void {
    this.reviewSql = { title, sql, diff };
  }
  /** The modal's textarea is controlled — it writes back through here. */
  setReviewSql(sql: string): void {
    if (this.reviewSql) this.reviewSql.sql = sql;
  }
  closeReview(): void {
    if (this.runningReview) return;
    this.reviewSql = null;
  }
  async runReview(): Promise<void> {
    if (!this.reviewSql || !this.connectionId) return;
    const sql = this.reviewSql.sql.trim();
    if (!sql) return;
    this.runningReview = true;
    try {
      // Scope to the active database (Mongo needs it to resolve `db.coll.…`).
      // Routed through the store so the production / read-only write-gate applies
      // — a guarded connection prompts for a typed confirmation first.
      const res = await database.runManagedStatement(sql, database.activeDb || null);
      if (res === null) {
        // Write was cancelled at the confirmation prompt — keep the modal open.
        toasts.info('Write cancelled');
        return;
      }
      toasts.success('Applied', 'Statement ran successfully');
      this.reviewSql = null;
      // Refresh what's ON SCREEN: re-run the statement that produced this grid
      // (`statement` is the tab's ran_statement, not the live editor buffer —
      // which may have been rewritten since) and stay on the current page.
      await database.runQuery(this.statement ?? undefined, undefined, {
        transient: true,
        keepOffset: true,
      });
    } catch (e) {
      toasts.error('Statement failed', e instanceof Error ? e.message : String(e));
      // keep the modal open so the user can fix the SQL and retry
    } finally {
      this.runningReview = false;
    }
  }

  // ── Row selection & delete (review-gated) ──────────────────────────────────
  // Selection is tracked by stable liveRows index. It's only meaningful when the
  // result is editable (single table/collection with a resolved key). Deleting
  // builds a statement and opens the SAME review modal as edits — nothing runs
  // until the user confirms there.
  selected: Set<number> = $state(new Set());
  lastClickedIdx: number | null = $state(null);

  /** Clear the selection whenever the upstream result changes (incl. the
   *  re-query after a delete runs). Pending cell drafts are keyed by liveRows
   *  index, so a result change invalidates them too — cleared together. */
  resetForResult(): void {
    this.selected = new Set();
    this.lastClickedIdx = null;
    this.pendingEdits = new Map();
  }

  readonly allInViewSelected: boolean = $derived(
    this.viewOrder.length > 0 && this.viewOrder.every((i) => this.selected.has(i)),
  );

  toggleRow(idx: number, e: MouseEvent): void {
    e.stopPropagation();
    const next = new Set(this.selected);
    if (e.shiftKey && this.lastClickedIdx !== null) {
      // Range over the CURRENT visible order, so it stays intuitive with a sort
      // or filter active.
      const order = this.viewOrder;
      const a = order.indexOf(this.lastClickedIdx);
      const b = order.indexOf(idx);
      if (a !== -1 && b !== -1) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        for (let k = lo; k <= hi; k++) next.add(order[k]);
        this.selected = next;
        this.lastClickedIdx = idx;
        return;
      }
    }
    if (next.has(idx)) next.delete(idx);
    else next.add(idx);
    this.selected = next;
    this.lastClickedIdx = idx;
  }

  toggleAllInView(viewIdxs: number[] = this.viewOrder): void {
    const next = new Set(this.selected);
    if (this.allInViewSelected) viewIdxs.forEach((i) => next.delete(i));
    else viewIdxs.forEach((i) => next.add(i));
    this.selected = next;
  }

  clearSelection(): void {
    this.selected = new Set();
    this.lastClickedIdx = null;
  }

  /** `{"_id": …}` filter for a row — ObjectId hex → `{"$oid": …}`, else raw. */
  mongoIdFilter(rowIdx: number): string {
    const idIdx = this.result!.columns.findIndex((c) => c.name === '_id');
    return mongoIdFilterFor(this.liveRows[rowIdx][idIdx]);
  }

  /** Build a DELETE / deleteMany targeting the given rows (by liveRows index). */
  private buildDelete(indices: number[]): { title: string; sql: string } | null {
    if (!this.result || !this.editable || indices.length === 0) return null;
    const ctx = this.editCtx();
    if (!ctx) return null;
    return adapterFor(this.engine).buildDelete(indices, ctx);
  }

  deleteRows(indices: number[]): void {
    const built = this.buildDelete(indices);
    if (built) this.openReview(built.title, built.sql);
  }
  deleteSelected(): void {
    this.deleteRows([...this.selected].filter((i) => i >= 0 && i < this.liveRows.length));
  }

  // ── Generate SQL from selected rows (0003b) ────────────────────────────────
  // For an editable single-table result, turn the selection into reusable SQL
  // using the same escaping as inline edits/deletes (`valueLiteral`). One opens a
  // new tab (INSERTs); the other copies a `pk IN (…)` predicate to the clipboard.

  /** Selected liveRows indices, in the current visible order, bounds-checked. */
  selectedIndices(viewOrder: number[] = this.viewOrder): number[] {
    const order = viewOrder.filter((i) => this.selected.has(i));
    return order.filter((i) => i >= 0 && i < this.liveRows.length);
  }

  /** Open the given rows as insert statements in a new tab — NOT run. */
  copyRowsAsInsert(idxs: number[]): void {
    const target = this.copyTarget;
    if (!target || !this.result || idxs.length === 0) return;
    const text = adapterFor(this.engine).buildInsert(
      idxs,
      this.ctx({ db: target.db, table: target.table, pkCols: this.editPkCols }),
    );
    if (!text) return;
    void database.openInNewTab(text, {
      name: `INSERT ${target.table}`,
      node: database.activeDb ?? undefined,
    });
    const n = idxs.length;
    toasts.success(
      'Generated',
      this.engine === 'mongodb'
        ? `insertMany with ${n} document${n === 1 ? '' : 's'}`
        : `${n} INSERT statement${n === 1 ? '' : 's'}`,
    );
  }

  copySelectedAsInsert(): void {
    this.copyRowsAsInsert(this.selectedIndices());
  }

  /** Build a `pk IN (…)` (single-PK) or OR-of-ANDs (composite) predicate for the
   *  selected rows and copy it to the clipboard. */
  copySelectedWhere(): void {
    if (!this.editable || this.editPkCols.length === 0) return;
    const idxs = this.selectedIndices();
    if (idxs.length === 0) return;
    const ctx = this.editCtx();
    if (!ctx) return;
    let where: string;
    if (this.editPkCols.length === 1) {
      const pk = this.editPkCols[0];
      const ci = this.result!.columns.findIndex((c) => c.name === pk);
      const list = idxs.map((i) => valueLiteral(this.engine, this.liveRows[i][ci])).join(', ');
      where = `${ctx.qid(pk)} IN (${list})`;
    } else {
      where = idxs.map((i) => `(${whereByPk(ctx, i)})`).join(' OR ');
    }
    void copyText(where);
    toasts.success('Copied', `WHERE for ${idxs.length} row${idxs.length === 1 ? '' : 's'}`);
  }

  // ── Cell viewer ────────────────────────────────────────────────────────────
  // The expandable cell viewer. `raw` is the unformatted text; `formatted`
  // holds a prettified copy (SQL or JSON) the user can toggle to.
  viewer: {
    raw: string;
    sql: boolean;
    formatted: boolean;
    /** Set when the viewed cell belongs to an editable result column — enables
     *  the in-viewer editor (the inline one-line input is useless for JSON). */
    edit: { rowIdx: number; colIdx: number } | null;
  } | null = $state(null);
  viewerEditing: boolean = $state(false);
  viewerDraft: string = $state('');
  viewerErr: string | null = $state(null);
  readonly viewerText: string = $derived(
    this.viewer
      ? this.viewer.formatted
        ? this.viewer.sql
          ? formatSql(this.viewer.raw)
          : this.viewer.raw
        : this.viewer.raw
      : '',
  );

  openCell(v: unknown, rowIdx = -1, colIdx = -1): void {
    const edit =
      rowIdx >= 0 && colIdx >= 0 && !this.reviewSql && this.isEditableCell(colIdx)
        ? { rowIdx, colIdx }
        : null;
    this.viewerEditing = false;
    this.viewerErr = null;
    if (typeof v === 'string') {
      this.viewer = { raw: v, sql: looksLikeSql(v), formatted: looksLikeSql(v), edit };
    } else if (v === null || v === undefined) {
      this.viewer = { raw: 'NULL', sql: false, formatted: false, edit };
    } else {
      this.viewer = { raw: prettyJson(v), sql: false, formatted: false, edit };
    }
  }

  startViewerEdit(): void {
    if (!this.viewer?.edit) return;
    const prev = this.liveRows[this.viewer.edit.rowIdx]?.[this.viewer.edit.colIdx];
    // Seed the draft with what the cell actually holds: pretty JSON for
    // complex values, the raw string for scalars, empty for NULL.
    this.viewerDraft =
      prev === null || prev === undefined ? '' : isComplex(prev) ? prettyJson(prev) : cellStr(prev);
    this.viewerErr = null;
    this.viewerEditing = true;
  }

  /** Validate + hand the viewer draft to the normal cell-edit review flow
   *  (commitEdit builds the engine-correct UPDATE / updateOne). */
  saveViewerEdit(): void {
    if (!this.viewer?.edit) return;
    const { rowIdx, colIdx } = this.viewer.edit;
    const prev = this.liveRows[rowIdx]?.[colIdx];
    let value = this.viewerDraft;
    if (isComplex(prev) && this.viewerDraft.trim() !== '') {
      try {
        // Canonicalize to compact JSON so the no-change check and the
        // generated statement both work off the same form.
        value = compactJson(JSON.parse(this.viewerDraft));
      } catch (e) {
        this.viewerErr = `Invalid JSON: ${e instanceof Error ? e.message : String(e)}`;
        return;
      }
    }
    this.viewer = null;
    this.viewerEditing = false;
    this.editing = { rowIdx, colIdx, value };
    this.commitEdit();
  }

  // ── Whole-document editor (JSON / Vertical views) ──────────────────────────
  // Edits the full row as one JSON object; Save builds a Mongo replaceOne (or
  // a per-changed-column SQL UPDATE) and opens the normal review modal.
  docEditor: { rowIdx: number; draft: string; err: string | null } | null = $state(null);

  openDocEditor(rowIdx: number): void {
    if (!this.editable || !this.result) return;
    const o: Record<string, unknown> = {};
    this.result.columns.forEach((c, i) => (o[c.name] = this.liveRows[rowIdx]?.[i]));
    this.docEditor = { rowIdx, draft: prettyJson(o), err: null };
  }

  saveDocEdit(): void {
    if (!this.docEditor || !this.result || !this.editTable) return;
    const { rowIdx } = this.docEditor;
    let doc: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(this.docEditor.draft);
      if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
        this.docEditor.err = 'Document must be a JSON object';
        return;
      }
      doc = parsed as Record<string, unknown>;
    } catch (e) {
      this.docEditor.err = `Invalid JSON: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    const ctx = this.editCtx();
    if (!ctx) return;
    // Null = nothing changed (SQL) — just close the editor.
    const built = adapterFor(this.engine).buildReplace?.(rowIdx, doc, ctx) ?? null;
    this.docEditor = null;
    if (built) this.openReview(built.title, built.sql, built.diff);
  }
}
