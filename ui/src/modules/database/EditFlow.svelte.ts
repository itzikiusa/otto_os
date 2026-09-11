// The inline-edit flow behind the results views: editability detection, cell
// drafts and dotted-path ops parked as pending changes, row selection + delete,
// "Copy as INSERT", the cell viewer / whole-document (and insert) editor, and
// the Review-SQL modal that every write funnels through. ONE instance per
// ResultsGrid; the views (GridView / VerticalView / VerticalTree / JsonView /
// CellViewer / DocEditor / ReviewModal) read and call it through their `flow`
// prop.
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
import {
  adapterFor,
  type DiffLine,
  type EditCtx,
  type EditTarget,
  type RowPatch,
  type TypedValue,
} from './edit-types';
import { parseSimpleSelect, qid, typedCellDraft, valueLiteral, whereByPk } from './edit-sql';
import { mongoCollectionForEdit, mongoIdFilterFor } from './edit-mongo';
import { applyPatchAtPath, flattenPaths, valueKind } from './expansion-plan';

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

    // Mongo and Redis resolve synchronously through their adapters (a Redis
    // "row" is one key); everything else needs the SQL capability.
    if (this.engine !== 'mongodb' && this.engine !== 'redis' && database.capabilities?.sql !== true) return;

    const r = adapterFor(this.engine).target(
      sql,
      cols.map((c) => c.name),
      { engine: this.engine, activeDb: database.activeDb },
    );
    if (!r.target) {
      this.editReason = r.reason;
      return;
    }
    if (this.engine === 'mongodb' || this.engine === 'redis') {
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

  // Everything parked, keyed by liveRows index: whole-cell drafts (`cells`,
  // by column index — the grid / cell viewer) plus the Vertical editor's
  // dotted-path ops (`set` / `unset` / `rename`). The Maps are replaced
  // wholesale on every mutation (never mutated in place) so the runes notice.
  // Any upstream result change shifts the indexes, so pending edits are
  // cleared alongside the row selection (resetForResult).
  pending: Map<number, RowPatch> = $state(new Map());
  /** The cell drafts alone (row → col → raw draft) — what GridView /
   *  CellViewer read. A row with only path ops is present with an empty map,
   *  so `.size` still counts every pending row. */
  readonly pendingEdits: Map<number, Map<number, string>> = $derived(
    new Map([...this.pending].map(([r, p]) => [r, p.cells])),
  );
  readonly pendingCells: number = $derived(
    [...this.pending.values()].reduce(
      (n, p) => n + p.cells.size + p.set.size + p.unset.size + p.rename.size,
      0,
    ),
  );

  pendingValue(rowIdx: number, colIdx: number): string | undefined {
    return this.pending.get(rowIdx)?.cells.get(colIdx);
  }
  discardPending(): void {
    this.pending = new Map();
  }

  /** Copy-on-write edit of one row's patch; an emptied patch drops the row. */
  private withRow(rowIdx: number, edit: (p: RowPatch) => void): void {
    const cur = this.pending.get(rowIdx);
    const p: RowPatch = {
      cells: new Map(cur?.cells),
      set: new Map(cur?.set),
      unset: new Set(cur?.unset),
      rename: new Map(cur?.rename),
    };
    edit(p);
    const next = new Map(this.pending);
    if (p.cells.size + p.set.size + p.unset.size + p.rename.size === 0) next.delete(rowIdx);
    else next.set(rowIdx, p);
    this.pending = next;
  }

  /** Park a pending value directly (the context menu's Set NULL / Set empty
   *  string) — same review flow as a typed draft, just skipping the input. */
  parkValue(rowIdx: number, colIdx: number, value: string): void {
    this.withRow(rowIdx, (p) => {
      this.dropOverlapping(p, this.columnPath(colIdx) ?? '');
      p.cells.set(colIdx, value);
    });
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
    this.withRow(rowIdx, (p) => {
      // A whole-cell draft supersedes any path op parked inside that column.
      const replaced = this.dropOverlapping(p, this.columnPath(colIdx) ?? '');
      if (replaced.length) toasts.info('Replaced the pending change', `on ${replaced.join(', ')}`);
      if (value === prevStr) p.cells.delete(colIdx);
      else p.cells.set(colIdx, value);
    });
    this.editing = null;
  }

  // ── Path-level editing (Vertical view) ─────────────────────────────────────
  // Paths are dotted and relative to the row (`items.0.qty`); a top-level path
  // is the column name. Mongo parks them as `$set` / `$unset` / `$rename` ops
  // on the row (one updateOne per row, alongside any cell drafts). SQL engines
  // have no path ops: a top-level path IS the cell (beginEdit/commitEdit), and
  // a path inside a JSON column is applied to the column's value client-side
  // and parked as a whole-cell draft — one `UPDATE … SET col = '<json>'`, the
  // same statement the grid produces, never a second write path.
  //
  // Conflict rules at park time: a path that is being renamed can't be edited
  // ("rename it first"); an op on `a` and one on `a.b` can't coexist (Mongo
  // rejects the update) — the later action replaces the earlier, with a toast.

  /** The column's real name (path root) for a column index. */
  private columnPath(colIdx: number): string | null {
    return this.result?.columns[colIdx]?.name ?? null;
  }
  /** Column index a dotted path belongs to (its first segment). */
  private columnOfPath(path: string): number {
    const root = path.split('.')[0];
    return this.result?.columns.findIndex((c) => c.name === root) ?? -1;
  }
  private static overlaps(a: string, b: string): boolean {
    return a === b || a.startsWith(b + '.') || b.startsWith(a + '.');
  }
  /** Remove every set/unset/cell op at, above or below `path`; returns the
   *  paths that were dropped (for the "replaced" toast). Renames are left
   *  alone — `renameBlocks` rejects the edit before this runs. */
  private dropOverlapping(p: RowPatch, path: string): string[] {
    const dropped: string[] = [];
    for (const k of [...p.set.keys()]) {
      if (EditFlow.overlaps(k, path)) {
        p.set.delete(k);
        dropped.push(k);
      }
    }
    for (const k of [...p.unset]) {
      if (EditFlow.overlaps(k, path)) {
        p.unset.delete(k);
        dropped.push(k);
      }
    }
    for (const ci of [...p.cells.keys()]) {
      const name = this.columnPath(ci);
      if (name !== null && EditFlow.overlaps(name, path)) {
        p.cells.delete(ci);
        dropped.push(name);
      }
    }
    return dropped.filter((d) => d !== path);
  }
  /** True (with a toast) when a pending rename touches `path` — either side. */
  private renameBlocks(p: RowPatch, path: string): boolean {
    for (const [from, to] of p.rename) {
      if (EditFlow.overlaps(from, path) || EditFlow.overlaps(to, path)) {
        toasts.warn('Field is being renamed', `Rename it first, then edit ${to} — or discard the pending change`);
        return true;
      }
    }
    return false;
  }
  /** `colIdx === -1` is a NEW top-level field (Mongo only — a SQL row can't
   *  grow a column); otherwise the column must be editable (not the key). */
  private canEditPath(rowIdx: number, colIdx: number, path: string): boolean {
    if (!this.result || !this.editable || this.reviewSql || rowIdx < 0 || rowIdx >= this.liveRows.length) return false;
    if (colIdx === -1) return this.engine === 'mongodb' && !path.includes('.') && !this.editPkCols.includes(path);
    const col = this.columnPath(colIdx);
    return col !== null && (path === col || path.startsWith(col + '.')) && this.isEditableCell(colIdx);
  }

  /** The Vertical view's inline editor — one open at a time, like `editing`. */
  pathEditing: { rowIdx: number; colIdx: number; path: string } | null = $state(null);
  beginPathEdit(rowIdx: number, colIdx: number, path: string): void {
    if (!this.canEditPath(rowIdx, colIdx, path)) return;
    this.pathEditing = { rowIdx, colIdx, path };
  }
  cancelPathEdit(): void {
    this.pathEditing = null;
  }
  /** Pending `$set` entries DIRECTLY under `parentPath` ('' = the row root),
   *  as `[key, value]` — the tree renders the ones that don't exist yet. */
  pendingSetUnder(rowIdx: number, parentPath: string): [string, TypedValue][] {
    const p = this.pending.get(rowIdx);
    if (!p) return [];
    const out: [string, TypedValue][] = [];
    for (const [path, tv] of p.set) {
      if (parentPath === '') {
        if (!path.includes('.')) out.push([path, tv]);
      } else if (path.startsWith(parentPath + '.') && !path.slice(parentPath.length + 1).includes('.')) {
        out.push([path.slice(parentPath.length + 1), tv]);
      }
    }
    return out;
  }
  /** The column value a SQL nested edit applies to: an already-parked draft
   *  (so successive nested edits accumulate in one UPDATE) or the live value. */
  private sqlColumnBase(rowIdx: number, colIdx: number): unknown {
    const draft = this.pendingValue(rowIdx, colIdx);
    if (draft !== undefined && draft !== SET_NULL && draft !== SET_EMPTY) {
      try {
        return JSON.parse(draft);
      } catch {
        /* a scalar draft — fall through to the live value */
      }
    }
    return this.liveRows[rowIdx]?.[colIdx];
  }
  /** Park a whole-column draft for a SQL JSON edit through commitEdit (so the
   *  no-change check un-parks a value edited back to what is stored). */
  private parkSqlColumn(rowIdx: number, colIdx: number, next: unknown): void {
    const live = this.liveRows[rowIdx]?.[colIdx];
    const same = compactJson(next ?? null) === compactJson(live ?? null);
    this.editing = {
      rowIdx,
      colIdx,
      value: same
        ? live === null || live === undefined
          ? ''
          : cellStr(live)
        : next === null || next === undefined
          ? SET_NULL
          : isComplex(next)
            ? compactJson(next)
            : String(next),
    };
    this.commitEdit();
  }

  /** Park a typed value at a dotted path. */
  parkPath(rowIdx: number, colIdx: number, path: string, tv: TypedValue): void {
    this.pathEditing = null;
    if (!this.canEditPath(rowIdx, colIdx, path)) return;
    const col = this.columnPath(colIdx);
    if (this.engine !== 'mongodb' && col !== null) {
      if (path === col) {
        this.editing = { rowIdx, colIdx, value: typedCellDraft(this.engine, tv) };
        this.commitEdit();
        return;
      }
      const rest = path.slice(col.length + 1);
      this.parkSqlColumn(rowIdx, colIdx, applyPatchAtPath(this.sqlColumnBase(rowIdx, colIdx), rest, 'set', typedToJs(tv)));
      return;
    }
    const live = this.liveValueAt(rowIdx, path);
    const unchanged = live !== undefined && valueKind(live) === tv.kind && typedRaw(live) === tv.raw;
    this.withRow(rowIdx, (p) => {
      if (this.renameBlocks(p, path)) return;
      const replaced = this.dropOverlapping(p, path);
      if (replaced.length) toasts.info('Replaced the pending change', `on ${replaced.join(', ')}`);
      // Edited back to the stored value → un-park (like a cell draft).
      if (!unchanged) p.set.set(path, tv);
    });
  }

  /** Park a field removal (`$unset`; SQL: only inside a JSON column). */
  unsetPath(rowIdx: number, colIdx: number, path: string): void {
    this.pathEditing = null;
    if (!this.canEditPath(rowIdx, colIdx, path)) return;
    const col = this.columnPath(colIdx);
    if (this.engine !== 'mongodb' && col !== null) {
      if (path === col) {
        toasts.warn("Columns can't be removed from a row", 'Set the value to NULL instead');
        return;
      }
      const rest = path.slice(col.length + 1);
      this.parkSqlColumn(rowIdx, colIdx, applyPatchAtPath(this.sqlColumnBase(rowIdx, colIdx), rest, 'unset'));
      return;
    }
    // A field that only exists as a pending `$set` (added here) just goes away.
    const exists = this.liveValueAt(rowIdx, path) !== undefined;
    this.withRow(rowIdx, (p) => {
      if (this.renameBlocks(p, path)) return;
      const replaced = this.dropOverlapping(p, path);
      if (replaced.length && exists) toasts.info('Replaced the pending change', `on ${replaced.join(', ')}`);
      if (exists) p.unset.add(path);
    });
  }

  /** Park a field rename (`$rename` old → new; SQL: only inside a JSON column).
   *  `newPath` is the full dotted path of the new name (same parent). */
  renamePath(rowIdx: number, colIdx: number, oldPath: string, newPath: string): void {
    this.pathEditing = null;
    if (!this.canEditPath(rowIdx, colIdx, oldPath) || oldPath === newPath || !newPath) return;
    const col = this.columnPath(colIdx);
    if (this.engine !== 'mongodb' && col !== null) {
      if (oldPath === col) {
        toasts.warn("Columns can't be renamed from a row", 'Use the table designer');
        return;
      }
      const rest = oldPath.slice(col.length + 1);
      const restNew = newPath.slice(col.length + 1);
      this.parkSqlColumn(
        rowIdx,
        colIdx,
        applyPatchAtPath(this.sqlColumnBase(rowIdx, colIdx), rest, 'rename', undefined, restNew),
      );
      return;
    }
    if (/(^|\.)\d+(\.|$)/.test(oldPath) || /(^|\.)\d+(\.|$)/.test(newPath)) {
      toasts.warn("Can't rename inside an array", '$rename does not traverse array elements');
      return;
    }
    this.withRow(rowIdx, (p) => {
      // Any other pending op touching either side must go first — Mongo
      // rejects a $rename that conflicts with a $set/$unset in the same update.
      const busy = [...p.set.keys(), ...p.unset, ...[...p.cells.keys()].map((ci) => this.columnPath(ci) ?? '')].find(
        (k) => EditFlow.overlaps(k, oldPath) || EditFlow.overlaps(k, newPath),
      );
      if (busy !== undefined) {
        toasts.warn('Field has a pending change', `Review or discard the change on ${busy} before renaming`);
        return;
      }
      for (const [from, to] of [...p.rename]) {
        if (from === oldPath || to === oldPath || to === newPath) p.rename.delete(from); // later wins
      }
      p.rename.set(oldPath, newPath);
    });
  }

  /** The stored value at a dotted path of a row (undefined when absent). */
  private liveValueAt(rowIdx: number, path: string): unknown {
    const ci = this.columnOfPath(path);
    if (ci < 0) return undefined;
    const [, ...rest] = path.split('.');
    let cur: unknown = this.liveRows[rowIdx]?.[ci];
    for (const seg of rest) {
      if (cur === null || typeof cur !== 'object') return undefined;
      cur = (cur as Record<string, unknown>)[seg];
    }
    return cur;
  }

  /** What is parked at exactly `path` on a row: a typed value (a path `$set`,
   *  or a top-level cell draft in typed form), `'unset'`, `{ renamedTo }`, or
   *  undefined. `colIdx` disambiguates duplicate column names. */
  pendingAt(rowIdx: number, path: string, colIdx?: number): TypedValue | 'unset' | { renamedTo: string } | undefined {
    const p = this.pending.get(rowIdx);
    if (!p) return undefined;
    const tv = p.set.get(path);
    if (tv) return tv;
    if (p.unset.has(path)) return 'unset';
    const to = p.rename.get(path);
    if (to !== undefined) return { renamedTo: to };
    if (!path.includes('.')) {
      const ci = colIdx ?? this.columnOfPath(path);
      const raw = p.cells.get(ci);
      if (raw !== undefined) return cellDraftTyped(raw, this.liveRows[rowIdx]?.[ci]);
    }
    return undefined;
  }
  /** True when any op is parked at or BELOW `path` (a container row / grid
   *  cell with a nested change inside it). */
  hasPendingUnder(rowIdx: number, path: string): boolean {
    const p = this.pending.get(rowIdx);
    if (!p) return false;
    const under = (k: string): boolean => k === path || k.startsWith(path + '.');
    for (const k of p.set.keys()) if (under(k)) return true;
    for (const k of p.unset) if (under(k)) return true;
    for (const k of p.rename.keys()) if (under(k)) return true;
    for (const ci of p.cells.keys()) {
      const name = this.columnPath(ci);
      if (name !== null && under(name)) return true;
    }
    return false;
  }

  /** Build ONE statement per pending row — every parked column in a single
   * SET (`$set` for Mongo; ClickHouse uses `ALTER TABLE … UPDATE`, a mutation)
   * — and open the review modal. Multiple rows become a multi-statement batch
   * (each driver splits and runs them in order). */
  reviewPending(): void {
    if (!this.result || !this.editTable || this.pending.size === 0) return;
    const ctx = this.editCtx();
    if (!ctx) return;
    const rows = [...this.pending.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([rowIdx, patch]) => ({ rowIdx, patch }));
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
    this.pending = new Map();
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
  // a per-changed-column SQL UPDATE) and opens the normal review modal. With
  // `rowIdx === -1` it is the INSERT editor: the draft starts as `{}` (SQL: a
  // template of the columns, a single key omitted so it regenerates) and Save
  // builds an insertOne / INSERT instead.
  docEditor: { rowIdx: number; draft: string; err: string | null } | null = $state(null);

  openDocEditor(rowIdx: number): void {
    if (!this.editable || !this.result) return;
    const o: Record<string, unknown> = {};
    if (rowIdx < 0) {
      if (this.engine !== 'mongodb') {
        const omitPk = this.editPkCols.length === 1;
        for (const c of this.result.columns) if (!(omitPk && this.editPkCols.includes(c.name))) o[c.name] = null;
      }
    } else {
      this.result.columns.forEach((c, i) => (o[c.name] = this.liveRows[rowIdx]?.[i]));
    }
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
    if (rowIdx < 0) {
      const sql = adapterFor(this.engine).buildInsertDoc?.(doc, ctx) ?? null;
      if (!sql) {
        this.docEditor.err = 'No field matches a column of this table';
        return;
      }
      this.docEditor = null;
      const diff: DiffLine[] = [...flattenPaths(doc)].map(([path, v]) => ({
        row: -1,
        path,
        op: 'set',
        before: '∅',
        after: v === null || v === undefined ? 'null' : cellStr(v),
      }));
      this.openReview(this.engine === 'mongodb' ? 'Review insertOne' : 'Review INSERT', sql, diff);
      return;
    }
    // Null = nothing changed (SQL) — just close the editor.
    const built = adapterFor(this.engine).buildReplace?.(rowIdx, doc, ctx) ?? null;
    this.docEditor = null;
    if (built) this.openReview(built.title, built.sql, built.diff);
  }
}

// ── Typed-value helpers (module-private) ─────────────────────────────────────

/** The editor text a stored value pre-fills for its `valueKind`. */
export function typedRaw(v: unknown): string {
  switch (valueKind(v)) {
    case 'null':
      return '';
    case 'objectId':
      return String((v as Record<string, unknown>).$oid);
    case 'date': {
      const d = (v as Record<string, unknown>).$date;
      if (typeof d === 'string') return d;
      if (typeof d === 'number') return new Date(d).toISOString();
      if (d && typeof d === 'object' && '$numberLong' in d) {
        const ms = Number((d as Record<string, unknown>).$numberLong);
        return Number.isFinite(ms) ? new Date(ms).toISOString() : String(d);
      }
      return String(d);
    }
    case 'long':
      return String((v as Record<string, unknown>).$numberLong);
    case 'decimal':
      return String((v as Record<string, unknown>).$numberDecimal);
    case 'json':
      return compactJson(v);
    default:
      return String(v);
  }
}

/** A typed value as the JS value it denotes (BSON kinds as their EJSON
 *  sentinel) — what a SQL JSON column receives via `applyPatchAtPath`. */
function typedToJs(tv: TypedValue): unknown {
  switch (tv.kind) {
    case 'null':
      return null;
    case 'bool':
      return tv.raw === 'true';
    case 'number':
      return Number(tv.raw);
    case 'json':
      try {
        return JSON.parse(tv.raw);
      } catch {
        return tv.raw;
      }
    case 'objectId':
      return { $oid: tv.raw };
    case 'date':
      return { $date: tv.raw };
    case 'long':
      return { $numberLong: tv.raw };
    case 'decimal':
      return { $numberDecimal: tv.raw };
    default:
      return tv.raw;
  }
}

/** A parked cell draft in typed form (the Vertical view shows cell drafts the
 *  grid parked): the sentinels map to null / '', otherwise the kind follows
 *  the stored value the way `sqlLiteral` / `mongoLiteral` will type it. */
function cellDraftTyped(raw: string, live: unknown): TypedValue {
  if (raw === SET_NULL || raw === '') return { kind: 'null', raw: '' };
  if (raw === SET_EMPTY) return { kind: 'string', raw: '' };
  if (typeof live === 'number' && /^-?\d+(\.\d+)?$/.test(raw)) return { kind: 'number', raw };
  if (typeof live === 'boolean' && (raw === 'true' || raw === 'false')) return { kind: 'bool', raw };
  if (isComplex(live)) {
    try {
      JSON.parse(raw);
      return { kind: 'json', raw };
    } catch {
      /* not JSON — a string */
    }
  }
  return { kind: 'string', raw };
}
