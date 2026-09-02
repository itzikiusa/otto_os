<script lang="ts">
  // Tabular result grid: sticky header, monospace cells, NULL as a dimmed ∅,
  // objects/arrays shown as compact JSON with a click-to-expand cell viewer.
  // Columns auto-size to their content and are drag-resizable. A toolbar search
  // filters the loaded rows client-side. When the result comes from a simple
  // single-table SELECT with a known single-column primary key, cells become
  // double-click editable (issues an UPDATE via the connection's query API).
  // Toolbar: search · Copy (TSV) · Export CSV · Export JSON. Footer: rows · ms.
  import { tick } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { buildFilteredQuery, type FilterMode } from './query-filter';
  import { escapeSqlString } from './sql-util';
  import { bsonScalar } from './bson';
  import JsonTree from './JsonTree.svelte';
  import type { QueryResult, DbExportFormat, ExportToPathResp, DbForeignKey } from '../../lib/api/types';
  import { api, postNdjsonStream } from '../../lib/api/client';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import ContextPacketDialog from '../../lib/components/ContextPacketDialog.svelte';
  import ErrorPanel from './ErrorPanel.svelte';

  // ── Send-to-agent dialog (B2a: replaces raw injectInput for DB results) ──────
  let sendToAgentOpen = $state(false);
  let sendToAgentPayload = $state<unknown>(null);

  interface Props {
    result: QueryResult | null;
    error?: string | null;
    /** Compact mode for dashboard widget mini-grids (no toolbar/footer). */
    mini?: boolean;
    /** Active statement — enables editability detection when set. */
    statement?: string;
    /** Connection id the result came from — required for inline editing. */
    connectionId?: string | null;
    /** True while the active tab's query is in flight — drives the running overlay. */
    running?: boolean;
    /** Active tab's current row offset (footer pager). */
    offset?: number;
  }
  let {
    result: resultProp,
    error = null,
    mini = false,
    statement,
    connectionId,
    running = false,
    offset = 0,
  }: Props = $props();

  // ── Multi-result switcher (multi-statement batches) ──────────────────────────
  // The server returns the first statement's result at top level and the rest in
  // `more_results` (each with a statement preview + errored flag). The switcher
  // picks which set is shown; everything below reads `result` (the SHOWN set), so
  // Grid/Vertical/JSON, export, editing and the footer all follow the selection.
  let resultIdx = $state(0);
  const resultSets = $derived<QueryResult[]>(
    resultProp ? [resultProp, ...(resultProp.more_results ?? [])] : [],
  );
  const result = $derived<QueryResult | null>(resultSets[resultIdx] ?? resultProp);
  // A brand-new upstream result resets the selection to the first set.
  $effect(() => {
    void resultProp;
    resultIdx = 0;
  });

  // What to say when a result carries no columns. SQL engines return column
  // metadata even for an empty SELECT, so a column-less result there really is a
  // bare statement ack — but Mongo is schemaless: its columns are inferred from
  // the returned documents, so a `find` that matches NOTHING also arrives with
  // zero columns. Labelling that "Statement OK" reads as "your query ran but the
  // data is hidden". Distinguish by the write metadata the drivers do set:
  // `rows_affected` (and a `message`) mark a write; their absence is a read that
  // matched nothing.
  const emptyResultLabel = $derived(
    result?.message ??
      (result?.rows_affected != null
        ? `${result.rows_affected} row(s) affected`
        : result
          ? 'No rows returned'
          : 'Statement OK'),
  );

  // ── Running overlay elapsed counter ──────────────────────────────────────────
  // Ticks while the active tab's query is in flight so the overlay shows elapsed
  // seconds; stops + resets when the query settles or the component unmounts.
  let elapsed = $state(0);
  $effect(() => {
    if (!running) return;
    elapsed = 0;
    const start = Date.now();
    const iv = setInterval(() => {
      elapsed = Math.floor((Date.now() - start) / 1000);
    }, 250);
    return () => clearInterval(iv);
  });

  // ── Footer pager (single auto-limited result only) ───────────────────────────
  // `auto_limited` (the server's applied LIMIT) lives on the top-level result and
  // is present only for a single paginatable SELECT / Mongo find — never batches.
  const pageSize = $derived(resultSets.length === 1 ? (resultProp?.auto_limited ?? 0) : 0);
  const showPager = $derived(!mini && pageSize > 0 && !!result);
  const pageRowCount = $derived(result?.rows.length ?? 0);
  const pageFrom = $derived(pageRowCount === 0 ? 0 : offset + 1);
  const pageTo = $derived(offset + pageRowCount);
  // A full page implies there may be more; a short page is the last one.
  const hasNextPage = $derived(pageSize > 0 && pageRowCount >= pageSize);
  const hasOrderBy = $derived(/\border\s+by\b/i.test(statement ?? ''));

  // Mini widget grids are previews — cap their rendering. The main grid renders
  // ALL fetched rows via windowed virtualization (only the visible slice is in
  // the DOM), so there's no row cap there.
  const MINI_MAX = 200;

  // The rows we render/filter/sort over. Re-seeded whenever the upstream result
  // changes (edits run against the DB and refresh via re-query, not in place).
  let liveRows = $state<unknown[][]>([]);
  // Column-name signature of the last rendered result (non-reactive — used only
  // to decide whether the view state should reset).
  let prevColKey: string | null = null;
  $effect(() => {
    const cols = result?.columns ?? [];
    const colKey = cols.map((c) => c.name).join('');
    // Rows always re-seed (edits re-query, not patch in place).
    liveRows = result ? (mini ? result.rows.slice(0, MINI_MAX) : result.rows) : [];
    // Preserve sort / search / column widths / scroll when the new result has the
    // SAME columns (a re-run of the same query), so the grid doesn't jump; reset
    // them only when the shape actually changes.
    if (colKey !== prevColKey) {
      search = '';
      colWidths = {};
      sortCol = null;
      sortDir = null;
      scrollTop = 0;
      if (scrollEl) scrollEl.scrollTop = 0;
      prevColKey = colKey;
    }
  });

  // Engine behind this result (drives dialect for inline edits).
  const engine = $derived(database.capabilities?.engine ?? null);

  // ── Windowed virtualization (main grid only) ─────────────────────────────────
  // Render only the rows in (or near) the viewport, with spacer rows preserving
  // the full scroll height. Row height is fixed in CSS (see ROW_H), so the math
  // is exact and we can scroll smoothly through 100k+ rows.
  // "Expand JSON" mode pretty-prints complex cells inline; rows grow to a fixed
  // taller height so the virtualization math stays exact.
  let expandJson = $state(false);
  const ROW_H = $derived(expandJson ? 168 : 26); // must match `.grid tbody td` height
  const OVERSCAN = 12;

  // Result view mode: columnar grid (default), a JSON array, or a vertical
  // row-per-record layout (like Postgres `\x` / ClickHouse FORMAT Vertical).
  type ViewMode = 'grid' | 'json' | 'vertical';
  let viewMode = $state<ViewMode>('grid');
  // Non-grid views aren't virtualized, and one document can be enormous on its
  // own (a `lobby_format_history` doc is ~88KB, so 100 rows ≈ 9MB). A flat 500-row
  // cap is therefore no protection at all — rendering is BATCHED instead: draw
  // ALT_BATCH records, grow on demand. VIEW_CAP stays the hard ceiling.
  const VIEW_CAP = 500;
  const ALT_BATCH = 25;
  let altShown = $state(ALT_BATCH);
  // Collapse the window back whenever the result or the view mode changes —
  // otherwise a big window opened on one result silently applies to the next.
  $effect(() => {
    void result;
    void viewMode;
    altShown = ALT_BATCH;
  });
  let scrollEl = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let viewportH = $state(0);
  const virtualize = $derived(!mini);

  // Track the scroll viewport height with a ResizeObserver rather than a plain
  // `bind:clientHeight`. On mobile the flex height chain isn't settled at first
  // paint, so the bind reads 0 → virtualization computes a tiny/empty window and
  // the grid looks blank. The observer fires again once layout distributes the
  // height (and on every later resize/orientation change), so `viewportH` — and
  // the `endIdx` $derived that reads it — recalculates and rows render.
  $effect(() => {
    const el = scrollEl;
    if (!el) return;
    viewportH = el.clientHeight;
    const ro = new ResizeObserver(() => {
      viewportH = el.clientHeight;
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // ── Search / filter ─────────────────────────────────────────────────────────
  let search = $state('');
  const searchLc = $derived(search.trim().toLowerCase());
  const filtering = $derived(searchLc.length > 0);

  // Per-row scan text, built ONCE per result rather than per keystroke. The old
  // code re-serialized every cell on every character typed — with ~90KB Mongo
  // documents that is megabytes of `JSON.stringify` + `toLowerCase` per keypress,
  // which is what made the filter box lock up on fat collections.
  //
  // Reading `filtering` (a boolean) and not `searchLc` is deliberate: the cache is
  // built when the box goes from empty→non-empty and then reused for every
  // subsequent character.
  /** Cap per row so one blob can't dominate memory; matches past it are not scanned. */
  const SCAN_MAX = 65536;
  const scanRows = $derived.by<string[]>(() => {
    if (!filtering) return [];
    return liveRows.map((row) => {
      let s = '';
      for (const v of row) {
        if (v === null || v === undefined) continue;
        s += cellStr(v) + ' ';
        if (s.length >= SCAN_MAX) break;
      }
      return s.slice(0, SCAN_MAX).toLowerCase();
    });
  });

  function rowMatches(idx: number): boolean {
    return (scanRows[idx] ?? '').includes(searchLc);
  }

  // ── Quick-filter chips, applied CLIENT-SIDE over the loaded rows ─────────────
  // "Filter:"/"Exclude:" (database.filters) narrow the grid IMMEDIATELY here — the
  // same chips also rewrite the statement so pressing Run re-queries the server
  // for the full (uncapped) set. `raw` chips are hand-written SQL we can't
  // evaluate in the browser, so they're skipped client-side.
  const colIndexByName = $derived.by<Map<string, number>>(() => {
    const m = new Map<string, number>();
    result?.columns.forEach((c, i) => {
      if (!m.has(c.name)) m.set(c.name, i);
    });
    return m;
  });
  // A chip is "active" (worth filtering on) only when it has at least one value.
  const activeChips = $derived(
    database.filters.filter((c) => c.kind === 'col' && c.values.length > 0),
  );
  function cellMatchesVal(cell: unknown, val: { raw: string; isNull: boolean }): boolean {
    if (val.isNull) return cell === null || cell === undefined;
    if (cell === null || cell === undefined) return false;
    const s = cellStr(cell);
    return s === val.raw;
  }
  function chipMatches(row: unknown[]): boolean {
    for (const c of activeChips) {
      if (c.kind !== 'col') continue;
      const ci = colIndexByName.get(c.column);
      if (ci === undefined) continue; // column not in this result — can't apply
      const inSet = c.values.some((v) => cellMatchesVal(row[ci], v));
      if (c.op === 'in' && !inSet) return false;
      if (c.op === 'not_in' && inSet) return false;
    }
    return true;
  }

  // Rows passing the filter, carrying their original index so edits target the
  // right entry in `liveRows`. Purely client-side over the fetched rows.
  const filteredRows = $derived.by<{ row: unknown[]; idx: number }[]>(() => {
    const hasChips = activeChips.length > 0;
    if (!filtering && !hasChips) return liveRows.map((row, idx) => ({ row, idx }));
    const out: { row: unknown[]; idx: number }[] = [];
    for (let idx = 0; idx < liveRows.length; idx++) {
      const row = liveRows[idx];
      if (hasChips && !chipMatches(row)) continue;
      if (filtering && !rowMatches(idx)) continue;
      out.push({ row, idx });
    }
    return out;
  });

  // ── Sort (client-side, over the filtered view) ───────────────────────────────
  // One active sort column at a time, cycling none → asc → desc → none. Type-
  // aware: numeric compare when both sides are numbers, else case-insensitive
  // localeCompare; NULL/undefined/complex always sort last in either direction.
  let sortCol = $state<number | null>(null);
  let sortDir = $state<'asc' | 'desc' | null>(null);
  const sorting = $derived(sortCol !== null && sortDir !== null);

  function cycleSort(colIndex: number): void {
    if (sortCol !== colIndex) {
      sortCol = colIndex;
      sortDir = 'asc';
    } else if (sortDir === 'asc') {
      sortDir = 'desc';
    } else {
      sortCol = null;
      sortDir = null;
    }
  }

  function numericVal(v: unknown): number | null {
    if (typeof v === 'number') return Number.isFinite(v) ? v : null;
    if (typeof v === 'bigint') return Number(v);
    if (typeof v === 'string' && v.trim() !== '' && /^-?\d+(\.\d+)?$/.test(v.trim())) {
      return Number(v);
    }
    return null;
  }
  /** NULL/undefined/objects are "empty" → always last regardless of direction. */
  function isEmptyVal(v: unknown): boolean {
    return v === null || v === undefined || isComplex(v);
  }

  // Final displayed rows: filter first, then sort (stable). Both in-memory.
  const viewRows = $derived.by<{ row: unknown[]; idx: number }[]>(() => {
    const base = filteredRows;
    if (!sorting || sortCol === null || sortDir === null) return base;
    const col = sortCol;
    const factor = sortDir === 'asc' ? 1 : -1;
    // Decorate with position for a stable sort, then strip.
    return base
      .map((entry, pos) => ({ entry, pos }))
      .sort((a, b) => {
        const av = a.entry.row[col];
        const bv = b.entry.row[col];
        const aEmpty = isEmptyVal(av);
        const bEmpty = isEmptyVal(bv);
        // Empty values pinned to the bottom in BOTH directions.
        if (aEmpty || bEmpty) {
          if (aEmpty && bEmpty) return a.pos - b.pos;
          return aEmpty ? 1 : -1;
        }
        const an = numericVal(av);
        const bn = numericVal(bv);
        let cmp: number;
        if (an !== null && bn !== null) {
          cmp = an - bn;
        } else {
          cmp = String(av).localeCompare(String(bv), undefined, { sensitivity: 'base' });
        }
        if (cmp !== 0) return cmp * factor;
        return a.pos - b.pos; // stable tiebreak
      })
      .map((d) => d.entry);
  });

  // Filtered/sorted rows as plain objects (for the JSON / vertical views),
  // capped. `idx` is the ORIGINAL liveRows index so per-document edits can
  // target the row's key regardless of filter/sort order.
  /** How many records the alt views may draw right now (batch ∩ hard cap). */
  const altCap = $derived(Math.min(altShown, VIEW_CAP));
  // Duplicate column names (e.g. `SELECT a.id, b.id …` on an engine that keeps
  // both as `id`) must not silently collapse when rows are objectified for the
  // JSON / vertical views — later duplicates become `name (2)`, `name (3)`, …
  const uniqueColNames = $derived.by<string[]>(() => {
    const seen = new Map<string, number>();
    return (result?.columns ?? []).map((c) => {
      const n = (seen.get(c.name) ?? 0) + 1;
      seen.set(c.name, n);
      return n === 1 ? c.name : `${c.name} (${n})`;
    });
  });
  const objRows = $derived.by<{ obj: Record<string, unknown>; idx: number }[]>(() => {
    if (!result || viewMode === 'grid') return [];
    const names = uniqueColNames;
    return viewRows.slice(0, altCap).map(({ row, idx }) => {
      const o: Record<string, unknown> = {};
      names.forEach((n, i) => (o[n] = row[i]));
      return { obj: o, idx };
    });
  });
  /** Records still drawable below the current batch (excludes the hard-capped tail). */
  const altRemaining = $derived(
    viewMode === 'grid' ? 0 : Math.max(0, Math.min(viewRows.length, VIEW_CAP) - altCap),
  );
  const viewTruncated = $derived(viewMode !== 'grid' && viewRows.length > VIEW_CAP);

  // The visible window over viewRows, plus the spacer heights above/below it.
  const total = $derived(viewRows.length);
  const startIdx = $derived(
    virtualize ? Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN) : 0,
  );
  const endIdx = $derived(
    virtualize ? Math.min(total, Math.ceil((scrollTop + viewportH) / ROW_H) + OVERSCAN) : total,
  );
  const windowRows = $derived(virtualize ? viewRows.slice(startIdx, endIdx) : viewRows);
  const padTop = $derived(startIdx * ROW_H);
  const padBottom = $derived(Math.max(0, (total - endIdx) * ROW_H));

  function onScroll(): void {
    if (scrollEl) scrollTop = scrollEl.scrollTop;
  }

  // ── Cell rendering helpers ───────────────────────────────────────────────────
  // The expandable cell viewer. `raw` is the unformatted text; `formatted`
  // holds a prettified copy (SQL or JSON) the user can toggle to.
  let viewer = $state<{
    raw: string;
    sql: boolean;
    formatted: boolean;
    /** Set when the viewed cell belongs to an editable result column — enables
     *  the in-viewer editor (the inline one-line input is useless for JSON). */
    edit: { rowIdx: number; colIdx: number } | null;
  } | null>(null);
  let viewerEditing = $state(false);
  let viewerDraft = $state('');
  let viewerErr = $state<string | null>(null);
  const viewerText = $derived(
    viewer ? (viewer.formatted ? (viewer.sql ? formatSql(viewer.raw) : viewer.raw) : viewer.raw) : '',
  );

  function isComplex(v: unknown): boolean {
    // A BSON sentinel ({"$oid":…}/{"$date":…}/{"$numberDecimal":…}) is a SCALAR
    // for display purposes — it renders as ObjectId("…")/ISODate("…"), not JSON.
    return v !== null && typeof v === 'object' && bsonScalar(v) === null;
  }
  /** Stringify a cell for display/search/copy: a BSON sentinel → its typed form
   *  (ObjectId("…")/ISODate("…")), a complex value → compact JSON, else String. */
  function cellStr(v: unknown): string {
    const b = bsonScalar(v);
    if (b !== null) return b;
    return isComplex(v) ? compactJson(v) : String(v);
  }
  /** Heuristic: does this string look like a SQL statement (DDL/DML/EXPLAIN)? */
  function looksLikeSql(s: string): boolean {
    return /^\s*(create|select|insert|update|alter|with|explain|show|drop|attach|grant)\b/i.test(s);
  }
  function compactJson(v: unknown): string {
    try {
      return JSON.stringify(v);
    } catch {
      return String(v);
    }
  }
  function prettyJson(v: unknown): string {
    try {
      return JSON.stringify(v, null, 2);
    } catch {
      return String(v);
    }
  }
  function cellText(v: unknown): string {
    if (v === null || v === undefined) return '';
    return cellStr(v);
  }
  /** Hard cap on the text ONE grid cell puts in the DOM. Cells are width-clamped
   *  anyway and the full value is one click away in the cell viewer, so pushing a
   *  ~90KB blob into a 60ch box buys nothing and costs layout time on every
   *  scroll. Copy / edit / export deliberately keep using the UNCLIPPED value. */
  const CELL_MAX = 512;
  function clip(s: string): string {
    return s.length > CELL_MAX ? s.slice(0, CELL_MAX) + '…' : s;
  }
  function cellDisplay(v: unknown): string {
    return clip(cellText(v));
  }
  /** Vertical view: render as a collapsible tree rather than raw text when the
   *  value is structured, or a scalar too long to sit inline. */
  function vvTree(v: unknown): boolean {
    if (isComplex(v)) return true;
    return typeof v === 'string' && v.length > 400;
  }
  function openCell(v: unknown, rowIdx = -1, colIdx = -1): void {
    const edit =
      rowIdx >= 0 && colIdx >= 0 && !reviewSql && isEditableCell(colIdx)
        ? { rowIdx, colIdx }
        : null;
    viewerEditing = false;
    viewerErr = null;
    if (typeof v === 'string') {
      viewer = { raw: v, sql: looksLikeSql(v), formatted: looksLikeSql(v), edit };
    } else if (v === null || v === undefined) {
      viewer = { raw: 'NULL', sql: false, formatted: false, edit };
    } else {
      viewer = { raw: prettyJson(v), sql: false, formatted: false, edit };
    }
  }

  function startViewerEdit(): void {
    if (!viewer?.edit) return;
    const prev = liveRows[viewer.edit.rowIdx]?.[viewer.edit.colIdx];
    // Seed the draft with what the cell actually holds: pretty JSON for
    // complex values, the raw string for scalars, empty for NULL.
    viewerDraft =
      prev === null || prev === undefined ? '' : isComplex(prev) ? prettyJson(prev) : cellStr(prev);
    viewerErr = null;
    viewerEditing = true;
  }

  // ── Whole-document editor (JSON / Vertical views) ─────────────────────────
  // Edits the full row as one JSON object; Save builds a Mongo replaceOne (or
  // a per-changed-column SQL UPDATE) and opens the normal review modal.
  let docEditor = $state<{ rowIdx: number; draft: string; err: string | null } | null>(null);

  function openDocEditor(rowIdx: number): void {
    if (!editable || !result) return;
    const o: Record<string, unknown> = {};
    result.columns.forEach((c, i) => (o[c.name] = liveRows[rowIdx]?.[i]));
    docEditor = { rowIdx, draft: prettyJson(o), err: null };
  }

  function saveDocEdit(): void {
    if (!docEditor || !result || !editTable) return;
    const { rowIdx } = docEditor;
    let doc: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(docEditor.draft);
      if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
        docEditor.err = 'Document must be a JSON object';
        return;
      }
      doc = parsed as Record<string, unknown>;
    } catch (e) {
      docEditor.err = `Invalid JSON: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    if (engine === 'mongodb') {
      // replaceOne by _id; the filter carries the id, so drop it from the body
      // (replacing _id is rejected by the server anyway).
      const body = { ...doc };
      delete body._id;
      const cmd = `db.${editTable}.replaceOne(${mongoIdFilter(rowIdx)}, ${JSON.stringify(body)})`;
      docEditor = null;
      openReview('Review replaceOne', cmd);
      return;
    }
    // SQL engines: SET only the columns whose value actually changed.
    const sets: string[] = [];
    result.columns.forEach((c, i) => {
      if (editPkCols.includes(c.name)) return; // key is the row's identity
      if (!(c.name in doc)) return;
      const prev = liveRows[rowIdx]?.[i];
      const next = doc[c.name];
      if (compactJson(prev ?? null) === compactJson(next ?? null)) return;
      sets.push(`${qid(c.name)} = ${valueLiteral(next)}`);
    });
    if (sets.length === 0) {
      docEditor = null; // nothing changed
      return;
    }
    const where = whereByPk(rowIdx);
    const sql =
      engine === 'clickhouse'
        ? `ALTER TABLE ${tableRef()} UPDATE ${sets.join(', ')} WHERE ${where};`
        : `UPDATE ${tableRef()} SET ${sets.join(', ')} WHERE ${where};`;
    docEditor = null;
    openReview(engine === 'clickhouse' ? 'Review ALTER … UPDATE (mutation)' : 'Review UPDATE', sql);
  }

  /** Validate + hand the viewer draft to the normal cell-edit review flow
   *  (commitEdit builds the engine-correct UPDATE / updateOne). */
  function saveViewerEdit(): void {
    if (!viewer?.edit) return;
    const { rowIdx, colIdx } = viewer.edit;
    const prev = liveRows[rowIdx]?.[colIdx];
    let value = viewerDraft;
    if (isComplex(prev) && viewerDraft.trim() !== '') {
      try {
        // Canonicalize to compact JSON so the no-change check and the
        // generated statement both work off the same form.
        value = compactJson(JSON.parse(viewerDraft));
      } catch (e) {
        viewerErr = `Invalid JSON: ${e instanceof Error ? e.message : String(e)}`;
        return;
      }
    }
    viewer = null;
    viewerEditing = false;
    editing = { rowIdx, colIdx, value };
    commitEdit();
  }

  /** Lightweight SQL pretty-printer: newlines before major clauses and one
   * column/arg per line inside the first paren group. String/backtick/comment
   * spans are preserved verbatim. Best-effort and never throws. */
  function formatSql(sql: string): string {
    try {
      const KW = [
        'SELECT', 'FROM', 'LEFT JOIN', 'RIGHT JOIN', 'INNER JOIN', 'OUTER JOIN', 'JOIN',
        'WHERE', 'GROUP BY', 'ORDER BY', 'HAVING', 'LIMIT', 'UNION ALL', 'UNION',
        'SETTINGS', 'PARTITION BY', 'PRIMARY KEY', 'ORDER BY', 'ENGINE', 'AS SELECT',
      ];
      let out = '';
      let depth = 0;
      let i = 0;
      let line = '';
      const flush = () => {
        if (line.trim().length) out += (out ? '\n' : '') + line.replace(/\s+$/, '');
        line = '';
      };
      while (i < sql.length) {
        const ch = sql[i];
        // Preserve quoted / backticked spans verbatim.
        if (ch === "'" || ch === '"' || ch === '`') {
          const q = ch;
          let j = i + 1;
          while (j < sql.length && sql[j] !== q) j++;
          line += sql.slice(i, j + 1);
          i = j + 1;
          continue;
        }
        if (ch === '(') {
          depth++;
          line += ch;
          // Break the column/arg list onto its own indented lines (depth 1 only).
          if (depth === 1) {
            flush();
            line = '  ';
          }
          i++;
          continue;
        }
        if (ch === ')') {
          if (depth === 1) {
            flush();
            line = '';
          }
          depth = Math.max(0, depth - 1);
          line += ch;
          i++;
          continue;
        }
        if (ch === ',' && depth === 1) {
          line += ',';
          flush();
          line = '  ';
          i++;
          continue;
        }
        // Major keyword at depth 0 → start a new line.
        if (depth === 0 && (i === 0 || /\s/.test(sql[i - 1]))) {
          const rest = sql.slice(i).toUpperCase();
          const kw = KW.find((k) => rest.startsWith(k + ' ') || rest === k || rest.startsWith(k + '\n'));
          if (kw) {
            flush();
            line = sql.slice(i, i + kw.length);
            i += kw.length;
            continue;
          }
        }
        line += ch;
        i++;
      }
      flush();
      return out || sql;
    } catch {
      return sql;
    }
  }

  async function copyViewer(): Promise<void> {
    try {
      await navigator.clipboard.writeText(viewerText);
      toasts.success('Copied', 'Full cell value copied');
    } catch {
      toasts.error('Copy failed');
    }
  }

  async function copyText(s: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(s);
    } catch {
      toasts.error('Copy failed');
    }
  }

  // "Query by value" / "Add to query": write the rebuilt query into the editor
  // AND the clipboard — never run it (the user reviews + presses Run). The query
  // is built from the active statement by the same WHERE/find-filter splicer the
  // quick-filter chips use (see ./query-filter + the store's splitStatement).
  function applyFilterQuery(query: string, mode: FilterMode): void {
    // Sets the editor statement AND clears any quick-filter chips (the rewritten
    // query now owns the WHERE) so the chip bar can't later discard the splice.
    database.setStatementFromCellFilter(query); // reflects in the editor (CodeEditor rebuilds)
    void copyText(query); // clipboard parity with the editor
    toasts.success(
      mode === 'set' ? 'Query by value' : 'Added to query',
      'Query updated & copied — press Run to execute',
    );
  }

  // ── Quick-filter context menus (cell + header) ───────────────────────────────
  function shortLabel(v: unknown): string {
    const s = v === null || v === undefined ? 'NULL' : cellStr(v);
    return s.length > 28 ? s.slice(0, 28) + '…' : s;
  }
  /** The FK (if any) whose `columns` include `col` — drives in-grid FK nav. */
  function fkForColumn(col: string): DbForeignKey | null {
    return editFks.find((fk) => fk.columns.includes(col)) ?? null;
  }

  /** Build `SELECT * FROM <ref_table> WHERE <ref_col> = <val> [AND …] LIMIT 1`
   *  targeting the row a single-table FK points at. Every FK column is matched
   *  to its referenced column using THIS row's values (composite-FK safe). A
   *  NULL local value short-circuits to null (no navigable target). */
  function fkTargetSql(fk: DbForeignKey, rowIdx: number): string | null {
    if (!result) return null;
    const conds: string[] = [];
    for (let i = 0; i < fk.columns.length; i++) {
      const localCol = fk.columns[i];
      const refCol = fk.ref_columns[i] ?? fk.ref_columns[0];
      const ci = result.columns.findIndex((c) => c.name === localCol);
      if (ci < 0) return null;
      const v = liveRows[rowIdx][ci];
      if (v === null || v === undefined) return null; // no row referenced
      conds.push(`${qid(refCol)} = ${valueLiteral(v)}`);
    }
    if (conds.length === 0) return null;
    const ref = fk.ref_schema ? `${qid(fk.ref_schema)}.${qid(fk.ref_table)}` : qid(fk.ref_table);
    return `SELECT * FROM ${ref} WHERE ${conds.join(' AND ')} LIMIT 1`;
  }

  function cellMenu(e: MouseEvent, ci: number, v: unknown, rowIdx: number): void {
    if (mini) return;
    const col = result?.columns[ci]?.name;
    if (!col) return;
    const short = shortLabel(v);
    const items: import('../../lib/contextmenu.svelte').MenuItem[] = [
      { label: `Filter:  ${col} = ${short}`, icon: 'search', action: () => database.addQuickFilter(col, v, 'include') },
      { label: `Exclude:  ${col} ≠ ${short}`, icon: 'x', action: () => database.addQuickFilter(col, v, 'exclude') },
    ];
    // In-grid foreign-key navigation (0003a): a cell in an FK column gets a
    // "→ Go to <ref_table>" jump opening a new tab with the referenced row.
    const fk = fkForColumn(col);
    if (fk) {
      const sql = fkTargetSql(fk, rowIdx);
      if (sql) {
        items.push({
          label: `→ Go to ${fk.ref_table}`,
          icon: 'external',
          action: () =>
            void database.openInNewTab(sql, {
              run: true,
              name: fk.ref_table,
              node: database.activeDb ?? undefined,
            }),
        });
      }
    }
    // "Query by value" / "Add to query": rebuild the ACTIVE query filtered by this
    // cell, into the editor + clipboard (never run). SQL engines + Mongo `find`
    // only; both items are hidden when the active statement can't be safely
    // filtered (e.g. non-SELECT, multi-statement, a Mongo aggregate).
    const fe =
      engine === 'mysql' || engine === 'clickhouse' || engine === 'mongodb' || engine === 'postgres'
        ? engine
        : null;
    if (fe) {
      const base = statement ?? '';
      const setQ = buildFilteredQuery(fe, base, col, v, 'set');
      const andQ = buildFilteredQuery(fe, base, col, v, 'and');
      if (setQ || andQ) {
        items.push({ separator: true });
        if (setQ) {
          items.push({ label: `Query by value:  ${col} = ${short}`, icon: 'search', action: () => applyFilterQuery(setQ, 'set') });
        }
        if (andQ) {
          items.push({ label: `Add to query:  AND ${col} = ${short}`, icon: 'plus', action: () => applyFilterQuery(andQ, 'and') });
        }
      }
    }
    items.push(
      { separator: true },
      { label: 'Expand value', icon: 'maximize', action: () => openCell(v, rowIdx, ci) },
      { label: 'Copy value', icon: 'file', action: () => copyText(v === null || v === undefined ? '' : cellStr(v)) },
    );
    // Explicit NULL / '' — the typed editor can't express the difference (an
    // empty draft parks as NULL). Both park a pending change, review-gated.
    if (isEditableCell(ci)) {
      items.push(
        { separator: true },
        { label: 'Set NULL', icon: 'x', disabled: v === null || v === undefined, action: () => parkValue(rowIdx, ci, SET_NULL) },
        { label: 'Set empty string', icon: 'edit', action: () => parkValue(rowIdx, ci, SET_EMPTY) },
      );
    }
    // Copy as INSERT — acts on the whole selection when this row is part of it,
    // otherwise on just this row (so a single row needs no checkbox first).
    if (copyTarget) {
      const rows = selected.has(rowIdx) ? selectedIndices() : [rowIdx];
      items.push({
        label: rows.length > 1 ? `Copy ${rows.length} rows as INSERT` : 'Copy row as INSERT',
        icon: 'file',
        action: () => copyRowsAsInsert(rows),
      });
    }
    // Delete actions — only for editable results (single table/collection with a
    // resolved key). Builds a statement and opens the review modal; never runs
    // immediately.
    if (editable) {
      items.push({ separator: true });
      if (selected.size > 0) {
        items.push({ label: `Delete selected (${selected.size})…`, icon: 'trash', danger: true, action: () => deleteSelected() });
      }
      if (!selected.has(rowIdx)) {
        items.push({ label: 'Delete this row…', icon: 'trash', danger: true, action: () => deleteRows([rowIdx]) });
      }
    }
    ctxMenu.show(e, items);
  }
  function headerMenu(e: MouseEvent, ci: number): void {
    if (mini) return;
    const col = result?.columns[ci]?.name;
    if (!col) return;
    ctxMenu.show(e, [
      { label: 'Sort ascending', icon: 'arrowUp', action: () => { sortCol = ci; sortDir = 'asc'; } },
      { label: 'Sort descending', icon: 'arrowDown', action: () => { sortCol = ci; sortDir = 'desc'; } },
      { label: 'Clear sort', disabled: sortCol !== ci, action: () => { sortCol = null; sortDir = null; } },
      { separator: true },
      { label: `Filter by ${col}…`, icon: 'search', action: () => database.addColumnFilter(col) },
      { label: 'Copy column name', icon: 'file', action: () => copyText(col) },
    ]);
  }

  // Per-chip "add value" input text (keyed by chip index).
  let addValText = $state<Record<number, string>>({});
  function submitFilterValue(i: number): void {
    const text = (addValText[i] ?? '').trim();
    if (!text) return;
    database.addFilterValue(i, text);
    addValText[i] = '';
  }

  // Highlight the matched substring inside a plain cell value. Returns segments.
  function highlightParts(text: string): { t: string; hit: boolean }[] {
    if (!filtering) return [{ t: text, hit: false }];
    const lc = text.toLowerCase();
    const out: { t: string; hit: boolean }[] = [];
    let i = 0;
    let found = lc.indexOf(searchLc);
    while (found !== -1) {
      if (found > i) out.push({ t: text.slice(i, found), hit: false });
      out.push({ t: text.slice(found, found + searchLc.length), hit: true });
      i = found + searchLc.length;
      found = lc.indexOf(searchLc, i);
    }
    if (i < text.length) out.push({ t: text.slice(i), hit: false });
    return out.length ? out : [{ t: text, hit: false }];
  }

  // ── Column widths ────────────────────────────────────────────────────────────
  // Auto-size each column from header + cell content (sampling up to 200 rows),
  // clamped to [MIN, MAX]. NULLs contribute nothing so they never widen a column.
  const MIN_CH = 5;
  const MAX_CH = 48;
  const WIDTH_SAMPLE = 200;

  /** Drag-overridden widths, keyed by column name; seeded from auto widths. */
  let colWidths = $state<Record<string, number>>({});

  function autoWidthCh(colIndex: number): number {
    if (!result) return MIN_CH;
    const col = result.columns[colIndex];
    let max = col.name.length + (col.type_hint && !mini ? col.type_hint.length + 2 : 0);
    const n = Math.min(liveRows.length, WIDTH_SAMPLE);
    for (let r = 0; r < n; r++) {
      const v = liveRows[r][colIndex];
      if (v === null || v === undefined) continue; // ∅ must not widen
      // Never serialize a complex value just to MEASURE it — a Mongo document can
      // be ~90KB and the result is clamped to MAX_CH regardless. Sentinels measure
      // by their rendered form; scalars measure exactly.
      const b = bsonScalar(v);
      const len = b !== null ? b.length : isComplex(v) ? MAX_CH : String(v).length;
      if (len > max) max = len;
      if (max >= MAX_CH) break; // already clamped — nothing longer can change it
    }
    // +2 ch padding allowance; clamp.
    return Math.max(MIN_CH, Math.min(MAX_CH, max + 2));
  }

  const autoWidths = $derived.by<number[]>(() =>
    result ? result.columns.map((_c, i) => autoWidthCh(i)) : [],
  );

  function widthFor(colIndex: number): number {
    const name = result?.columns[colIndex]?.name ?? '';
    return colWidths[name] ?? autoWidths[colIndex] ?? MIN_CH;
  }

  // Pointer-drag resize on a header's right edge.
  let dragName = $state<string | null>(null);
  let dragStartX = 0;
  let dragStartCh = 0;
  const PX_PER_CH = 7.4; // approx for the monospace cell font at 11.5px

  function startResize(e: PointerEvent, colIndex: number): void {
    e.preventDefault();
    e.stopPropagation();
    const name = result?.columns[colIndex]?.name ?? '';
    dragName = name;
    dragStartX = e.clientX;
    dragStartCh = widthFor(colIndex);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onResizeMove(e: PointerEvent): void {
    if (dragName === null) return;
    const deltaCh = (e.clientX - dragStartX) / PX_PER_CH;
    const next = Math.max(MIN_CH, Math.min(80, Math.round(dragStartCh + deltaCh)));
    colWidths = { ...colWidths, [dragName]: next };
  }
  function endResize(e: PointerEvent): void {
    if (dragName === null) return;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* capture may already be gone */
    }
    dragName = null;
  }

  // ── Editability detection ────────────────────────────────────────────────────
  // Editable iff: a connection id is present AND the statement is a plain
  // single-table SELECT (no JOIN/GROUP BY/UNION/DISTINCT/aggregate) AND the
  // table has exactly one primary-key column present in the result columns.
  let editDb = $state<string | null>(null);
  let editTable = $state<string | null>(null);
  let editPkCols = $state<string[]>([]); // pk column name(s) (when editable)
  let editReason = $state<string | null>(null); // why editing is unavailable
  // Foreign keys of the resolved single-table result (0003a in-grid FK nav).
  // Populated alongside the PK in the editability $effect; reused (no extra fetch).
  let editFks = $state<DbForeignKey[]>([]);

  const editable = $derived(editPkCols.length > 0 && editTable !== null);

  /** Target table/collection for "Copy as INSERT".
   *
   *  Deliberately NOT the same gate as `editable`. Editing needs a primary key
   *  because it has to TARGET an existing row; generating INSERTs only needs a
   *  name plus the values already on screen. Sharing the gate meant the action
   *  disappeared for every ClickHouse table whose `is_in_primary_key` doesn't
   *  resolve (Log/Memory engines, views) — and appeared on Mongo only to emit
   *  SQL. Resolved synchronously: no `object_detail` round-trip needed. */
  const copyTarget = $derived.by((): { db: string | null; table: string } | null => {
    const sql = statement;
    if (!sql || !connectionId || !result || result.columns.length === 0) return null;
    if (result.masked) return null; // INSERTs of redacted placeholders = data loss
    if (engine === 'mongodb') {
      const coll = mongoCollectionForEdit(sql);
      return coll ? { db: database.activeDb, table: coll } : null;
    }
    if (database.capabilities?.sql !== true) return null; // Redis etc.
    const parsed = parseSimpleSelect(sql);
    if (!parsed) return null;
    const db = parsed.db ?? (database.schemaRoot.find((n) => n.kind === 'database')?.label ?? null);
    return { db, table: parsed.table };
  });

  /** Parse a simple SELECT … FROM <table>. Returns {db, table} or null. */
  function parseSimpleSelect(sql: string): { db: string | null; table: string } | null {
    const s = sql.trim().replace(/;\s*$/, '');
    // A multi-statement batch can't be attributed to ONE table: the FROM matched
    // below would be statement 1's even while result set 2+ is shown, so edits
    // would target the wrong table. Mirrors the store's splitStatement rejection.
    if (/;\s*\S/.test(s)) return null;
    if (!/^select\b/i.test(s)) return null;
    // Reject anything that makes a row non-1:1 with a base-table row.
    if (/\bjoin\b|\bgroup\s+by\b|\bunion\b|\bdistinct\b|\bhaving\b/i.test(s)) return null;
    // Reject aggregates in the projection (between SELECT and FROM).
    const proj = s.slice(0, s.search(/\bfrom\b/i));
    if (/\b(count|sum|avg|min|max|group_concat|array_agg)\s*\(/i.test(proj)) return null;
    // Capture the first table after FROM: optional `db`.`table` with backticks.
    const m = s.match(
      /\bfrom\s+`?([\w$]+)`?(?:\s*\.\s*`?([\w$]+)`?)?/i,
    );
    if (!m) return null;
    if (m[2]) return { db: m[1], table: m[2] };
    return { db: null, table: m[1] };
  }

  /** Collection name for an editable Mongo result: a `db.<coll>.find(...)` or a
   * single-collection SELECT (which translates to a find). Null otherwise. */
  function mongoCollectionForEdit(s: string): string | null {
    const t = s.trim();
    // Same multi-statement rejection as parseSimpleSelect — a batch's first
    // `db.<coll>.find` must not make a LATER result set's rows "editable".
    if (/;\s*\S/.test(t.replace(/;\s*$/, ''))) return null;
    const m = t.match(/^db\.([A-Za-z0-9_$.-]+)\.find\s*\(/i);
    if (m) return m[1];
    return parseSimpleSelect(t)?.table ?? null;
  }

  /** JSON-encode a value typed into a Mongo cell editor: keep numbers/bools when
   * the prior value was one; valid JSON when editing a nested object/array;
   * empty → null; else a quoted string. */
  function mongoLiteral(raw: string, prev: unknown): string {
    if (raw === '' || raw === SET_NULL) return 'null';
    if (raw === SET_EMPTY) return '""';
    if (typeof prev === 'number' && /^-?\d+(\.\d+)?$/.test(raw)) return raw;
    if (typeof prev === 'boolean' && (raw === 'true' || raw === 'false')) return raw;
    if (isComplex(prev)) {
      try {
        JSON.parse(raw);
        return raw;
      } catch {
        /* not valid JSON — fall through to a string */
      }
    }
    return JSON.stringify(raw);
  }

  /** `{"_id": …}` filter for a row — ObjectId hex → `{"$oid": …}`, else raw. */
  function mongoIdFilter(rowIdx: number): string {
    const idIdx = result!.columns.findIndex((c) => c.name === '_id');
    const idVal = liveRows[rowIdx][idIdx];
    if (typeof idVal === 'string' && /^[a-f0-9]{24}$/i.test(idVal)) {
      return `{"_id": {"$oid": ${JSON.stringify(idVal)}}}`;
    }
    return `{"_id": ${JSON.stringify(idVal)}}`;
  }

  // Resolve the primary key whenever statement/connection/result changes.
  $effect(() => {
    // dependencies
    const sql = statement;
    const conn = connectionId;
    const cols = result?.columns;
    editDb = null;
    editTable = null;
    editPkCols = [];
    editReason = null;
    editFks = [];
    if (!sql || !conn || !cols || cols.length === 0) return;

    // A multi-statement batch is never editable — the shown result set can't be
    // safely attributed to one statement's table (see parseSimpleSelect).
    if (resultSets.length > 1 || /;\s*\S/.test(sql.trim().replace(/;\s*$/, ''))) {
      editReason = 'Editing is unavailable for multi-statement batches.';
      return;
    }
    // Masked values are REDACTED placeholders — writing them back would destroy
    // the real data, so a masked result is read-only.
    if (result?.masked) {
      editReason = 'Editing is disabled while server-side masking is applied.';
      return;
    }

    // Mongo: a single-collection find/SELECT is editable by `_id` — no
    // object_detail lookup (which would error on a SQL-style node path).
    if (engine === 'mongodb') {
      const coll = mongoCollectionForEdit(sql);
      if (!coll) {
        editReason = 'Editing needs a single-collection find or SELECT (no aggregate/join).';
        return;
      }
      if (!cols.some((c) => c.name === '_id')) {
        editReason = 'Include _id in the result to enable editing.';
        return;
      }
      editTable = coll;
      editPkCols = ['_id'];
      editDb = database.activeDb;
      editReason = null;
      return;
    }

    // SQL engines only beyond here (Redis etc. aren't editable).
    if (database.capabilities?.sql !== true) return;

    const parsed = parseSimpleSelect(sql);
    if (!parsed) {
      editReason = 'Editing needs a single-table SELECT (no JOIN, GROUP BY, DISTINCT, UNION or aggregates).';
      return;
    }

    // Build a default db from the schema root when the SQL omits it.
    const dbName =
      parsed.db ??
      (database.schemaRoot.find((n) => n.kind === 'database')?.label ?? null);

    const path = dbName ? `db:${dbName}/table:${parsed.table}` : `table:${parsed.table}`;

    let cancelled = false;
    void (async () => {
      const detail = await database.fetchObject(path);
      if (cancelled || !detail) return;
      // Need a primary key (one or more columns), all present in the result so
      // we can target the exact row. Composite keys are supported.
      if (detail.primary_key.length === 0) {
        editReason = `“${parsed.table}” has no primary key, so rows can't be safely targeted for edits.`;
        return;
      }
      const missing = detail.primary_key.filter((pk) => !cols.some((c) => c.name === pk));
      if (missing.length > 0) {
        const plural = detail.primary_key.length > 1 ? 's' : '';
        editReason = `Include the primary key column${plural} (${detail.primary_key.join(', ')}) in your SELECT to enable editing.`;
        return;
      }
      editDb = dbName;
      editTable = parsed.table;
      editPkCols = detail.primary_key;
      editFks = detail.foreign_keys ?? [];
      editReason = null;
    })();
    return () => {
      cancelled = true;
    };
  });

  // ── Inline editing ───────────────────────────────────────────────────────────
  // Edits are NOT applied directly. Committing a cell PARKS it as a pending
  // change (the cell renders the draft with a dirty marker) so several fields —
  // in one row or across rows — are prepared as ONE statement per row, every
  // changed column in a single SET/$set. "Review & apply" (the bar above the
  // footer) builds the statements and opens the "Review SQL" modal; nothing
  // runs until the user confirms there. After a successful run the grid
  // refreshes by re-running the active query, so values reflect the database
  // (no optimistic patching).
  let editing = $state<{ rowIdx: number; colIdx: number; value: string } | null>(null);

  // Pending cell drafts, keyed by liveRows index (row → col → raw draft).
  // Any upstream result change shifts the indexes, so pending edits are
  // cleared alongside the row selection (same $effect).
  let pendingEdits = $state<Map<number, Map<number, string>>>(new Map());
  const pendingCells = $derived([...pendingEdits.values()].reduce((n, m) => n + m.size, 0));

  function pendingValue(rowIdx: number, colIdx: number): string | undefined {
    return pendingEdits.get(rowIdx)?.get(colIdx);
  }
  function discardPending(): void {
    pendingEdits = new Map();
  }

  /** Park a pending value directly (the context menu's Set NULL / Set empty
   *  string) — same review flow as a typed draft, just skipping the input. */
  function parkValue(rowIdx: number, colIdx: number, value: string): void {
    const next = new Map(pendingEdits);
    const row = new Map(next.get(rowIdx) ?? []);
    row.set(colIdx, value);
    next.set(rowIdx, row);
    pendingEdits = next;
  }

  function isEditableCell(colIdx: number): boolean {
    if (!editable) return false;
    const name = result?.columns[colIdx]?.name;
    return !!name && !editPkCols.includes(name); // PK column(s) read-only
  }

  /** `\`pk1\` = v1 AND \`pk2\` = v2` targeting one row by its primary key. */
  /** Quote a SQL identifier for the active engine — double-quotes for Postgres
   *  (backticks are invalid there), backticks for MySQL/ClickHouse. */
  function qid(name: string): string {
    return engine === 'postgres'
      ? '"' + name.replace(/"/g, '""') + '"'
      : '`' + name.replace(/`/g, '``') + '`';
  }

  function whereByPk(rowIdx: number): string {
    if (!result) return '';
    return editPkCols
      .map((pk) => {
        const ci = result!.columns.findIndex((c) => c.name === pk);
        return `${qid(pk)} = ${valueLiteral(liveRows[rowIdx][ci])}`;
      })
      .join(' AND ');
  }

  function beginEdit(rowIdx: number, colIdx: number): void {
    if (!isEditableCell(colIdx) || reviewSql) return;
    // Re-editing a parked cell resumes its draft, not the stored value; the
    // Set-NULL/empty sentinels resume as an empty input.
    const parked = pendingValue(rowIdx, colIdx);
    const draft = parked === SET_NULL || parked === SET_EMPTY ? '' : parked;
    const v = liveRows[rowIdx]?.[colIdx];
    editing = {
      rowIdx,
      colIdx,
      value: draft ?? (v === null || v === undefined ? '' : cellStr(v)),
    };
  }
  function cancelEdit(): void {
    editing = null;
  }

  // MySQL (default modes) and ClickHouse treat `\` as an escape character inside
  // a string literal — a value containing one must double it or the emitted SQL
  // corrupts (a trailing `\` even swallows the closing quote). Postgres standard
  // strings don't, so only the quote is doubled there.
  const backslashEscapes = $derived(engine === 'mysql' || engine === 'clickhouse');

  // Explicit NULL / empty-string pending markers (context-menu "Set NULL" /
  // "Set empty string"). Typed text can't express the difference — an empty
  // draft parks as NULL — so the two actions park these sentinels instead;
  // sqlLiteral / mongoLiteral render them, the dirty cell displays them.
  const SET_NULL = '\u0000<null>';
  const SET_EMPTY = '\u0000<empty>';

  /** SQL-quote a scalar value typed into the cell editor: numbers bare (when
   * the previous value was numeric), empty → NULL, else 'escaped'. */
  function sqlLiteral(raw: string, asNumber: boolean): string {
    if (raw === '' || raw === SET_NULL) return 'NULL';
    if (raw === SET_EMPTY) return "''";
    if (asNumber && /^-?\d+(\.\d+)?$/.test(raw)) return raw;
    return `'${escapeSqlString(raw, backslashEscapes)}'`;
  }
  /** SQL-quote an existing typed value (for WHERE / INSERT values). */
  function valueLiteral(v: unknown): string {
    if (v === null || v === undefined) return 'NULL';
    if (typeof v === 'number' || typeof v === 'bigint') return String(v);
    if (typeof v === 'boolean') return engine === 'postgres' ? (v ? 'TRUE' : 'FALSE') : v ? '1' : '0';
    if (isComplex(v)) return `'${escapeSqlString(compactJson(v), backslashEscapes)}'`;
    return `'${escapeSqlString(String(v), backslashEscapes)}'`;
  }
  /** Qualified `db.table` (db optional), quoted for the active engine. */
  function tableRef(): string {
    const t = qid(editTable ?? '');
    return editDb ? `${qid(editDb)}.${t}` : t;
  }

  /** Park the in-progress cell edit as a pending change. A draft that matches
   * the stored value again un-parks the cell. The statement is built later, in
   * `reviewPending` — every parked column of a row in ONE UPDATE / updateOne. */
  function commitEdit(): void {
    if (!editing || !result || !editTable || editPkCols.length === 0) {
      editing = null;
      return;
    }
    const { rowIdx, colIdx, value } = editing;
    const prev = liveRows[rowIdx][colIdx];
    const prevStr = prev === null || prev === undefined ? '' : cellStr(prev);
    const next = new Map(pendingEdits);
    const row = new Map(next.get(rowIdx) ?? []);
    if (value === prevStr) row.delete(colIdx);
    else row.set(colIdx, value);
    if (row.size === 0) next.delete(rowIdx);
    else next.set(rowIdx, row);
    pendingEdits = next;
    editing = null;
  }

  /** Build ONE statement per pending row — every parked column in a single
   * SET (`$set` for Mongo; ClickHouse uses `ALTER TABLE … UPDATE`, a mutation)
   * — and open the review modal. Multiple rows become a multi-statement batch
   * (each driver splits and runs them in order). */
  function reviewPending(): void {
    if (!result || !editTable || pendingEdits.size === 0) return;
    const stmts: string[] = [];
    for (const [rowIdx, cols] of [...pendingEdits.entries()].sort((a, b) => a[0] - b[0])) {
      const entries = [...cols.entries()].sort((a, b) => a[0] - b[0]);
      if (engine === 'mongodb') {
        const sets = entries
          .map(([ci, value]) =>
            `${JSON.stringify(result!.columns[ci].name)}: ${mongoLiteral(value, liveRows[rowIdx][ci])}`)
          .join(', ');
        stmts.push(`db.${editTable}.updateOne(${mongoIdFilter(rowIdx)}, {"$set": {${sets}}})`);
        continue;
      }
      const sets = entries
        .map(([ci, value]) =>
          `${qid(result!.columns[ci].name)} = ${sqlLiteral(value, typeof liveRows[rowIdx][ci] === 'number')}`)
        .join(', ');
      const where = whereByPk(rowIdx);
      stmts.push(
        engine === 'clickhouse'
          ? `ALTER TABLE ${tableRef()} UPDATE ${sets} WHERE ${where};`
          : `UPDATE ${tableRef()} SET ${sets} WHERE ${where};`,
      );
    }
    openReview(
      engine === 'mongodb'
        ? 'Review updateOne'
        : engine === 'clickhouse'
          ? 'Review ALTER … UPDATE (mutation)'
          : 'Review UPDATE',
      stmts.join('\n'),
    );
  }

  /** Build an INSERT cloning a row. With a single (likely auto-increment) PK we
   * omit it so identity regenerates; with a composite key we include every
   * column so the user can adjust the key in the review SQL. */
  function duplicateRow(rowIdx: number): void {
    if (!result || !editTable || editPkCols.length === 0) return;
    // Mongo: insertOne of the row's fields, omitting `_id` so a fresh one is
    // generated; opens the review modal like the SQL path.
    if (engine === 'mongodb') {
      const obj: Record<string, unknown> = {};
      result.columns.forEach((c, i) => {
        if (c.name === '_id') return;
        obj[c.name] = liveRows[rowIdx][i];
      });
      openReview('Review insertOne (duplicate row)', `db.${editTable}.insertOne(${JSON.stringify(obj)})`);
      return;
    }
    const omitPk = editPkCols.length === 1;
    const cols: string[] = [];
    const vals: string[] = [];
    result.columns.forEach((c, i) => {
      if (omitPk && editPkCols.includes(c.name)) return; // single PK → regenerate
      cols.push(qid(c.name));
      vals.push(valueLiteral(liveRows[rowIdx][i]));
    });
    const sql = `INSERT INTO ${tableRef()} (${cols.join(', ')}) VALUES (${vals.join(', ')});`;
    openReview('Review INSERT (duplicate row)', sql);
  }

  function onEditKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitEdit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelEdit();
    }
  }

  // ── Review-SQL modal (shared by cell edits + row duplication) ────────────────
  // The textarea is the source of truth for what runs.
  let reviewSql = $state<{ title: string; sql: string } | null>(null);
  let runningReview = $state(false);

  function openReview(title: string, sql: string): void {
    reviewSql = { title, sql };
  }
  function closeReview(): void {
    if (runningReview) return;
    reviewSql = null;
  }
  async function runReview(): Promise<void> {
    if (!reviewSql || !connectionId) return;
    const sql = reviewSql.sql.trim();
    if (!sql) return;
    runningReview = true;
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
      reviewSql = null;
      // Refresh what's ON SCREEN: re-run the statement that produced this grid
      // (`statement` is the tab's ran_statement, not the live editor buffer —
      // which may have been rewritten since) and stay on the current page.
      await database.runQuery(statement ?? undefined, undefined, {
        transient: true,
        keepOffset: true,
      });
    } catch (e) {
      toasts.error('Statement failed', e instanceof Error ? e.message : String(e));
      // keep the modal open so the user can fix the SQL and retry
    } finally {
      runningReview = false;
    }
  }
  function onReviewKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      closeReview();
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void runReview();
    }
  }

  // ── Row selection & delete (review-gated) ────────────────────────────────────
  // Selection is tracked by stable liveRows index. It's only meaningful when the
  // result is editable (single table/collection with a resolved key). Deleting
  // builds a statement and opens the SAME review modal as edits — nothing runs
  // until the user confirms there.
  let selected = $state<Set<number>>(new Set());
  let lastClickedIdx = $state<number | null>(null);

  // Clear the selection whenever the upstream result changes (incl. the re-query
  // after a delete runs). Pending cell drafts are keyed by liveRows index, so a
  // result change invalidates them too — cleared together.
  $effect(() => {
    void result;
    selected = new Set();
    lastClickedIdx = null;
    pendingEdits = new Map();
    focusCell = null; // roving keyboard focus is positional — a new result invalidates it
  });

  const allInViewSelected = $derived(
    viewRows.length > 0 && viewRows.every((r) => selected.has(r.idx)),
  );

  function toggleRow(idx: number, e: MouseEvent): void {
    e.stopPropagation();
    const next = new Set(selected);
    if (e.shiftKey && lastClickedIdx !== null) {
      // Range over the CURRENT visible order, so it stays intuitive with a sort
      // or filter active.
      const order = viewRows.map((r) => r.idx);
      const a = order.indexOf(lastClickedIdx);
      const b = order.indexOf(idx);
      if (a !== -1 && b !== -1) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        for (let k = lo; k <= hi; k++) next.add(order[k]);
        selected = next;
        lastClickedIdx = idx;
        return;
      }
    }
    if (next.has(idx)) next.delete(idx);
    else next.add(idx);
    selected = next;
    lastClickedIdx = idx;
  }

  function toggleAllInView(): void {
    const next = new Set(selected);
    if (allInViewSelected) viewRows.forEach((r) => next.delete(r.idx));
    else viewRows.forEach((r) => next.add(r.idx));
    selected = next;
  }

  function clearSelection(): void {
    selected = new Set();
    lastClickedIdx = null;
  }

  /** `{"$oid": "hex"}` (or raw JSON) for a row's `_id` — Mongo delete targeting. */
  function mongoIdValue(rowIdx: number): string {
    const idIdx = result!.columns.findIndex((c) => c.name === '_id');
    const idVal = liveRows[rowIdx][idIdx];
    if (typeof idVal === 'string' && /^[a-f0-9]{24}$/i.test(idVal)) {
      return `{"$oid": ${JSON.stringify(idVal)}}`;
    }
    return JSON.stringify(idVal);
  }

  /** Build a DELETE / deleteMany targeting the given rows (by liveRows index). */
  function buildDelete(indices: number[]): { title: string; sql: string } | null {
    if (!result || !editable || indices.length === 0) return null;
    const n = indices.length;
    const noun = `${n} row${n === 1 ? '' : 's'}`;
    if (engine === 'mongodb') {
      const ids = indices.map(mongoIdValue).join(', ');
      return {
        title: `Review deleteMany (${noun})`,
        sql: `db.${editTable}.deleteMany({"_id": {"$in": [${ids}]}})`,
      };
    }
    let where: string;
    if (editPkCols.length === 1) {
      const pk = editPkCols[0];
      const ci = result.columns.findIndex((c) => c.name === pk);
      const list = indices.map((i) => valueLiteral(liveRows[i][ci])).join(', ');
      where = `${qid(pk)} IN (${list})`;
    } else {
      // Composite key: OR a per-row AND of every key column.
      where = indices.map((i) => `(${whereByPk(i)})`).join(' OR ');
    }
    const sql =
      engine === 'clickhouse'
        ? `ALTER TABLE ${tableRef()} DELETE WHERE ${where};`
        : `DELETE FROM ${tableRef()} WHERE ${where};`;
    return {
      title:
        engine === 'clickhouse' ? `Review ALTER … DELETE (${noun})` : `Review DELETE (${noun})`,
      sql,
    };
  }

  function deleteRows(indices: number[]): void {
    const built = buildDelete(indices);
    if (built) openReview(built.title, built.sql);
  }
  function deleteSelected(): void {
    deleteRows([...selected].filter((i) => i >= 0 && i < liveRows.length));
  }

  // ── Generate SQL from selected rows (0003b) ──────────────────────────────────
  // For an editable single-table result, turn the selection into reusable SQL
  // using the same escaping as inline edits/deletes (`valueLiteral`). One opens a
  // new tab (INSERTs); the other copies a `pk IN (…)` predicate to the clipboard.

  /** Selected liveRows indices, in the current visible order, bounds-checked. */
  function selectedIndices(): number[] {
    const order = viewRows.map((r) => r.idx).filter((i) => selected.has(i));
    return order.filter((i) => i >= 0 && i < liveRows.length);
  }

  /** Qualified `db.table` for a copy target, quoted for the active engine. */
  function copyTableRef(t: { db: string | null; table: string }): string {
    const q = qid(t.table);
    return t.db ? `${qid(t.db)}.${q}` : q;
  }

  /** JSON for a stored Mongo value inside a generated document. An `_id` that
   *  looks like an ObjectId hex is wrapped as `{"$oid": …}` — the same
   *  convention `mongoIdFilter` uses — so the document round-trips as a real
   *  ObjectId instead of degrading into a plain string. */
  function mongoValueLiteral(name: string, v: unknown): string {
    if (v === undefined || v === null) return 'null';
    if (name === '_id' && typeof v === 'string' && /^[a-f0-9]{24}$/i.test(v)) {
      return `{"$oid": ${JSON.stringify(v)}}`;
    }
    return JSON.stringify(v);
  }

  /** Insert statements for the given rows (all result columns), in the active
   *  engine's own syntax: `insertMany` for Mongo, `INSERT INTO … VALUES` for the
   *  SQL engines (ClickHouse included — it accepts the same backtick quoting).
   *  Returns null when there is no resolvable target. */
  function buildInsertStatements(idxs: number[]): string | null {
    const target = copyTarget;
    if (!target || !result || idxs.length === 0) return null;
    if (engine === 'mongodb') {
      const docs = idxs.map((i) => {
        const fields = result!.columns.map(
          (c, ci) => `${JSON.stringify(c.name)}: ${mongoValueLiteral(c.name, liveRows[i][ci])}`,
        );
        return `  { ${fields.join(', ')} }`;
      });
      return `db.${target.table}.insertMany([\n${docs.join(',\n')}\n])`;
    }
    const ref = copyTableRef(target);
    const cols = result.columns.map((c) => qid(c.name)).join(', ');
    return idxs
      .map((i) => {
        const vals = result!.columns.map((_, ci) => valueLiteral(liveRows[i][ci])).join(', ');
        return `INSERT INTO ${ref} (${cols}) VALUES (${vals});`;
      })
      .join('\n');
  }

  /** Open the given rows as insert statements in a new tab — NOT run. */
  function copyRowsAsInsert(idxs: number[]): void {
    const text = buildInsertStatements(idxs);
    if (!text || !copyTarget) return;
    void database.openInNewTab(text, {
      name: `INSERT ${copyTarget.table}`,
      node: database.activeDb ?? undefined,
    });
    const n = idxs.length;
    toasts.success(
      'Generated',
      engine === 'mongodb'
        ? `insertMany with ${n} document${n === 1 ? '' : 's'}`
        : `${n} INSERT statement${n === 1 ? '' : 's'}`,
    );
  }

  function copySelectedAsInsert(): void {
    copyRowsAsInsert(selectedIndices());
  }

  /** Build a `pk IN (…)` (single-PK) or OR-of-ANDs (composite) predicate for the
   *  selected rows and copy it to the clipboard. */
  function copySelectedWhere(): void {
    if (!editable || editPkCols.length === 0) return;
    const idxs = selectedIndices();
    if (idxs.length === 0) return;
    let where: string;
    if (editPkCols.length === 1) {
      const pk = editPkCols[0];
      const ci = result!.columns.findIndex((c) => c.name === pk);
      const list = idxs.map((i) => valueLiteral(liveRows[i][ci])).join(', ');
      where = `${qid(pk)} IN (${list})`;
    } else {
      where = idxs.map((i) => `(${whereByPk(i)})`).join(' OR ');
    }
    void copyText(where);
    toasts.success('Copied', `WHERE for ${idxs.length} row${idxs.length === 1 ? '' : 's'}`);
  }

  // ── Export / copy (reflect the current filtered + sorted view) ───────────────
  function exportText(v: unknown): string {
    if (v === null || v === undefined) return '';
    // Same rendering as the grid: a BSON sentinel exports as its typed form
    // (ObjectId("…")/ISODate("…")), never as "[object Object]".
    return cellStr(v);
  }
  // Quick-filter chips narrow the grid too — exports must honor them (viewRows
  // already carries chip + search filtering and the sort).
  const chipFiltering = $derived(activeChips.length > 0);
  function exportRows(): unknown[][] {
    return filtering || sorting || chipFiltering ? viewRows.map((r) => r.row) : liveRows;
  }
  function toTsv(): string {
    if (!result) return '';
    const header = result.columns.map((c) => c.name).join('\t');
    const body = exportRows()
      .map((r) => r.map((v) => exportText(v).replace(/\t/g, ' ').replace(/\n/g, ' ')).join('\t'))
      .join('\n');
    return `${header}\n${body}`;
  }
  function csvCell(v: unknown): string {
    let s = exportText(v);
    // Formula-injection guard: a STRING cell starting with = + - @ executes when
    // the CSV lands in a spreadsheet — neutralize with a leading apostrophe.
    // Non-string values (a bare -5 is data, not a formula) are left alone.
    if (typeof v === 'string' && /^[=+\-@]/.test(s)) s = `'${s}`;
    return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  }
  function toCsv(): string {
    if (!result) return '';
    const header = result.columns.map((c) => csvCell(c.name)).join(',');
    const body = exportRows().map((r) => r.map(csvCell).join(',')).join('\n');
    return `${header}\n${body}`;
  }
  function toJson(): string {
    if (!result) return '[]';
    const names = uniqueColNames;
    const objs = exportRows().map((r) => Object.fromEntries(names.map((n, i) => [n, r[i] ?? null])));
    return JSON.stringify(objs, null, 2);
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  const exportScope = $derived.by(() => {
    const parts: string[] = [];
    if (filtering || chipFiltering) parts.push('filtered');
    if (sorting) parts.push('sorted');
    return parts.length ? ` (${parts.join(' + ')} view)` : '';
  });

  async function copyTsv(): Promise<void> {
    try {
      await navigator.clipboard.writeText(toTsv());
      toasts.success('Copied', `Result copied as TSV${exportScope}`);
    } catch {
      toasts.error('Copy failed');
    }
  }
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
  function exportCsv(): void {
    download(toCsv(), 'result.csv', 'text/csv');
  }
  function exportJson(): void {
    download(toJson(), 'result.json', 'application/json');
  }

  // ── Large-batch streaming export to a local file ─────────────────────────────
  // Runs the statement uncapped on the daemon and STREAMS the result straight to
  // a file the user chooses on the daemon host — for result sets too big to pull
  // into the browser. Format is selectable; the destination directory is picked
  // via the shared FolderPicker (the same /fs/browse picker used elsewhere). Last
  // format + directory are remembered in localStorage.
  type ExportFmtOpt = { value: DbExportFormat; label: string };
  const EXPORT_FORMATS: ExportFmtOpt[] = [
    { value: 'csv', label: 'CSV' },
    { value: 'csv_with_names', label: 'CSV (with header)' },
    { value: 'tsv', label: 'TSV' },
    { value: 'tsv_with_names', label: 'TSV (with header)' },
    { value: 'json', label: 'JSON (array)' },
    { value: 'ndjson', label: 'NDJSON' },
  ];
  const EXT_BY_FORMAT: Record<DbExportFormat, string> = {
    csv: 'csv',
    csv_with_names: 'csv',
    tsv: 'tsv',
    tsv_with_names: 'tsv',
    json: 'json',
    ndjson: 'ndjson',
  };
  const LS_FORMAT = 'otto_db_export_format';
  const LS_DIR = 'otto_db_export_dir';

  function loadFormat(): DbExportFormat {
    const v = (typeof localStorage !== 'undefined' && localStorage.getItem(LS_FORMAT)) || 'csv';
    return EXPORT_FORMATS.some((f) => f.value === v) ? (v as DbExportFormat) : 'csv';
  }
  function loadDir(): string {
    return (typeof localStorage !== 'undefined' && localStorage.getItem(LS_DIR)) || '~/Downloads';
  }

  let showExportDialog = $state(false);
  let pickingDir = $state(false);
  let exportFormat = $state<DbExportFormat>(loadFormat());
  let exportDir = $state<string>(loadDir());
  let exportName = $state('');
  let exportLimit = $state('');
  let exportingPath = $state(false);
  // Live progress for the streaming export (bytes written so far). Null when no
  // export is running; drives the dialog's progress bar.
  let exportProgress = $state<{ bytes: number } | null>(null);
  // In-flight stream controller — the dialog's Cancel aborts it while exporting.
  let exportAbort: AbortController | null = null;

  // Default a filename from the statement (a leading table-ish token) or 'result'.
  function defaultExportName(): string {
    const fromStmt = statement?.match(/\bfrom\s+["'`]?([\w.]+)/i)?.[1];
    const base = (fromStmt || 'result').replace(/[^\w.-]+/g, '_').slice(0, 60) || 'result';
    return `${base}.${EXT_BY_FORMAT[exportFormat]}`;
  }

  function openExportDialog(): void {
    exportFormat = loadFormat();
    exportDir = loadDir();
    exportName = defaultExportName();
    exportLimit = '';
    showExportDialog = true;
  }

  // Keep the filename extension in sync when the format changes (only if the user
  // hasn't typed a custom, non-default-stem name).
  function onFormatChange(): void {
    const ext = EXT_BY_FORMAT[exportFormat];
    if (!exportName) {
      exportName = defaultExportName();
      return;
    }
    exportName = exportName.replace(/\.(csv|tsv|json|ndjson)$/i, '') + `.${ext}`;
  }

  function joinPath(dir: string, name: string): string {
    const d = dir.replace(/\/+$/, '');
    return `${d}/${name}`;
  }

  async function runPathExport(): Promise<void> {
    if (!connectionId || !statement || exportingPath) return;
    const name = exportName.trim() || defaultExportName();
    const dir = exportDir.trim() || '~/Downloads';
    const localPath = joinPath(dir, name);
    const maxRows = exportLimit.trim() ? Number(exportLimit.trim()) : undefined;
    if (maxRows !== undefined && (!Number.isFinite(maxRows) || maxRows <= 0)) {
      toasts.error('Invalid row limit', 'Leave blank for all rows, or enter a positive number.');
      return;
    }
    exportingPath = true;
    exportProgress = { bytes: 0 };
    exportAbort = new AbortController();
    let done: ExportToPathResp | null = null;
    let failed: string | null = null;
    try {
      // The endpoint streams NDJSON progress lines ({bytes:N}) and a final line
      // ({done,local_path,rows,bytes,duration_ms} or {error}); read them live so
      // the bar moves and a long export never idles out the browser fetch.
      await postNdjsonStream(
        `/connections/${connectionId}/db/export-to-path`,
        {
          statement,
          node: database.activeDb ?? undefined,
          format: exportFormat,
          local_path: localPath,
          max_rows: maxRows,
        },
        (msg) => {
          const m = msg as Record<string, unknown>;
          if (typeof m.error === 'string') failed = m.error;
          else if (m.done) done = m as unknown as ExportToPathResp;
          else if (typeof m.bytes === 'number') exportProgress = { bytes: m.bytes };
        },
        exportAbort.signal,
      );
      if (failed) throw new Error(failed);
      if (done) {
        const r: ExportToPathResp = done;
        if (typeof localStorage !== 'undefined') {
          localStorage.setItem(LS_FORMAT, exportFormat);
          localStorage.setItem(LS_DIR, dir);
        }
        showExportDialog = false;
        toasts.success(
          'Exported',
          `${r.rows.toLocaleString()} row${r.rows === 1 ? '' : 's'} · ${fmtBytes(r.bytes)} → ${r.local_path}`,
        );
      }
    } catch (e) {
      // A user-initiated cancel isn't a failure — the partial file stays where
      // the export was writing it.
      if (e instanceof DOMException && e.name === 'AbortError') {
        toasts.info('Export cancelled', 'The partially written file was left in place.');
      } else {
        toasts.error('Export failed', e instanceof Error ? e.message : String(e));
      }
    } finally {
      exportingPath = false;
      exportProgress = null;
      exportAbort = null;
    }
  }

  // Paste the query + result rows into the running agent's input (bracketed
  // paste, not auto-submitted) so it can act on the real DB state.
  // B2a: open the redacted-preview dialog instead of injecting raw text.
  // The dialog runs the payload through the server-side redaction pass so the
  // operator sees what the agent will receive before committing.
  function sendToRunningAgent(): void {
    if (!result || !ws.current) {
      toasts.error('No result to send', 'Run a query first');
      return;
    }
    const cols = result.columns.map((c) => c.name);
    const cap = 50;
    const rowsObj = viewRows.slice(0, cap).map(({ row }) => {
      const o: Record<string, unknown> = {};
      cols.forEach((c, i) => (o[c] = row[i]));
      return o;
    });
    const connName =
      (connectionId ? database.connections.find((c) => c.id === connectionId)?.name : null) ?? 'db';
    const more = viewRows.length > cap ? `, first ${cap} shown` : '';
    sendToAgentPayload = {
      connection: connName,
      statement: statement ?? null,
      rows: rowsObj,
      total_rows: viewRows.length,
      note: more ? `first ${cap} of ${viewRows.length} rows` : null,
    };
    sendToAgentOpen = true;
  }

  // ── Examine with AI (investigate) ────────────────────────────────────────────
  // Open the embedded DB Assistant in investigate mode, seeded with the current
  // statement + a small sample of the result columns/rows. The agent runs in its
  // own live shell beside the editor and can sample more data read-only.
  function examineWithAi(): void {
    if (!result) return;
    const cols = result.columns.map((c) => c.name).join(', ');
    const sample = (result.rows ?? []).slice(0, 8);
    const lines: string[] = [];
    if (statement) lines.push(`Statement:\n${statement}`);
    lines.push(`Columns: ${cols}`);
    lines.push(
      `Rows returned: ${result.stats.row_count} in ${result.stats.duration_ms} ms`,
    );
    if (sample.length) lines.push(`Sample rows:\n${JSON.stringify(sample, null, 2)}`);
    database.openAssist('investigate', lines.join('\n\n'));
  }

  // "Ask AI to fix" (from the error panel): open the DB Assistant in investigate
  // mode seeded with the failed statement + the engine error, so the agent can
  // diagnose and propose a corrected query. Same path as examineWithAi.
  function askAiToFix(): void {
    const parts: string[] = [];
    if (statement) parts.push(`Statement:\n${statement}`);
    if (error) parts.push(`Error:\n${error}`);
    parts.push('Explain what is wrong and propose a corrected query.');
    database.openAssist('investigate', parts.join('\n\n'));
  }

  // Autofocus + select the inline editor input on open. Svelte actions can't be
  // async, so defer the focus/select to a microtask after mount.
  function focusEditor(node: HTMLInputElement): void {
    void tick().then(() => {
      node.focus();
      node.select();
    });
  }

  // ── Keyboard grid navigation ─────────────────────────────────────────────────
  // Roving focus over the VISIBLE (filtered + sorted) rows: `r` indexes viewRows,
  // `c` the column. The scroll container owns focus + keydown; the focused cell
  // gets a ring. Arrows/Home/End/Page move, Enter edits (or expands a complex /
  // read-only cell), ⌘/Ctrl+C copies the cell, and ContextMenu / Shift+F10 opens
  // the row menu anchored to the cell.
  let focusCell = $state<{ r: number; c: number } | null>(null);
  /** Approx sticky-header height the top of a row must clear to be visible. */
  const HEAD_H = 27;

  function ensureRowVisible(r: number): void {
    if (!scrollEl) return;
    const top = r * ROW_H;
    if (top < scrollEl.scrollTop) scrollEl.scrollTop = top;
    else if (top + ROW_H > scrollEl.scrollTop + viewportH - HEAD_H)
      scrollEl.scrollTop = top + ROW_H - viewportH + HEAD_H;
  }

  /** Open the cell context menu for the focused cell, anchored to its element
   *  (a synthetic MouseEvent carries the coordinates ctxMenu positions by). */
  function openFocusMenu(): void {
    if (!focusCell || !result) return;
    const entry = viewRows[focusCell.r];
    if (!entry) return;
    const rect = scrollEl?.querySelector('td.kbd-focus')?.getBoundingClientRect();
    const ev = new MouseEvent('contextmenu', {
      clientX: rect ? rect.left + Math.min(rect.width, 160) / 2 : 80,
      clientY: rect ? rect.bottom - 2 : 80,
    });
    cellMenu(ev, focusCell.c, entry.row[focusCell.c], entry.idx);
  }

  function onGridKeydown(e: KeyboardEvent): void {
    if (mini || !result || editing || reviewSql || viewer || docEditor) return;
    const nRows = viewRows.length;
    const nCols = result.columns.length;
    if (nRows === 0 || nCols === 0) return;
    const cur = focusCell ?? { r: 0, c: 0 };
    const page = Math.max(1, Math.floor((viewportH - HEAD_H) / ROW_H) - 1);
    const setFocus = (r: number, c: number): void => {
      focusCell = {
        r: Math.max(0, Math.min(nRows - 1, r)),
        c: Math.max(0, Math.min(nCols - 1, c)),
      };
      ensureRowVisible(focusCell.r);
      e.preventDefault();
    };
    switch (e.key) {
      case 'ArrowDown': setFocus(focusCell ? cur.r + 1 : 0, cur.c); return;
      case 'ArrowUp': setFocus(cur.r - 1, cur.c); return;
      case 'ArrowRight': setFocus(cur.r, focusCell ? cur.c + 1 : 0); return;
      case 'ArrowLeft': setFocus(cur.r, cur.c - 1); return;
      case 'Home': setFocus(cur.r, 0); return;
      case 'End': setFocus(cur.r, nCols - 1); return;
      case 'PageDown': setFocus(cur.r + page, cur.c); return;
      case 'PageUp': setFocus(cur.r - page, cur.c); return;
    }
    if (!focusCell) return;
    const entry = viewRows[focusCell.r];
    if (!entry) return;
    const v = entry.row[focusCell.c];
    if (e.key === 'Enter') {
      e.preventDefault();
      if (isEditableCell(focusCell.c) && !isComplex(v)) beginEdit(entry.idx, focusCell.c);
      else openCell(v, entry.idx, focusCell.c);
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'c') {
      // A real text selection keeps the native copy.
      if (window.getSelection()?.toString()) return;
      e.preventDefault();
      void copyText(v === null || v === undefined ? '' : cellStr(v));
      toasts.success('Copied', 'Cell value copied');
      return;
    }
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault();
      openFocusMenu();
      return;
    }
    if (e.key === 'Escape') focusCell = null;
  }

  // ── Dialog keyboard behavior (cell viewer / doc editor / review modal) ───────
  /** Minimal dialog a11y as a Svelte action: focus moves into the dialog on
   *  open, Tab cycles inside it, Escape closes (a child that stopPropagation's
   *  Escape — the editor textareas — wins), and focus returns on close. */
  function dialogKeys(node: HTMLElement, onEscape: () => void) {
    const focusables = (): HTMLElement[] =>
      [
        ...node.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ].filter((el) => el.offsetParent !== null);
    const prev = document.activeElement as HTMLElement | null;
    void tick().then(() => {
      if (!node.contains(document.activeElement)) focusables()[0]?.focus();
    });
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onEscape();
        return;
      }
      if (e.key !== 'Tab') return;
      const els = focusables();
      if (els.length === 0) return;
      const first = els[0];
      const last = els[els.length - 1];
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !node.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    node.addEventListener('keydown', onKey);
    return {
      destroy() {
        node.removeEventListener('keydown', onKey);
        prev?.focus?.();
      },
    };
  }
</script>

{#snippet runningCard()}
  <!-- Inline running card for the no-result branches (a fresh tab's first query
       has no stale grid to dim — same selectors as the absolute overlay). -->
  <div class="rg-overlay rg-inline" role="status" aria-live="polite">
    <div class="rg-overlay-card">
      <span class="rg-spin"><Icon name="refresh" size={16} /></span>
      <span class="rg-overlay-text">Running… {elapsed}s</span>
      <button class="rg-cancel" onclick={() => database.abortQuery()} title="Cancel the running query">
        <Icon name="x" size={11} />Cancel
      </button>
    </div>
  </div>
{/snippet}

{#if !mini && resultSets.length > 1}
  <!-- Multi-statement batch: a segmented switcher over the result sets. Errored
       statements (execution stopped there) get a red dot; the tooltip previews
       the statement that produced each set. Sits above the active view. -->
  <div class="rg-switch" role="tablist" aria-label="Result sets">
    {#each resultSets as rs, i (i)}
      <button
        class="rg-seg"
        class:on={resultIdx === i}
        class:err={rs.errored}
        role="tab"
        aria-selected={resultIdx === i}
        title={rs.statement ?? `Result ${i + 1}`}
        onclick={() => (resultIdx = i)}
      >
        {#if rs.errored}<span class="rg-seg-dot" aria-hidden="true"></span>{/if}
        Result {i + 1}
      </button>
    {/each}
  </div>
{/if}
{#if error}
  {#if mini}
    <div class="grid-error mono">
      <Icon name="x" size={14} />
      <span>{error}</span>
    </div>
  {:else}
    <ErrorPanel {error} {engine} statement={statement ?? ''} onAskAi={askAiToFix} />
  {/if}
{:else if !resultProp}
  {#if !mini}
    {#if running}
      <div class="grid-empty">{@render runningCard()}</div>
    {:else}
      <div class="grid-empty">
        <Icon name="grid" size={mini ? 16 : 22} />
        <span>Run a query to see results.</span>
      </div>
    {/if}
  {/if}
{:else if !result || result.columns.length === 0}
  {#if running && !mini}
    <div class="grid-empty">{@render runningCard()}</div>
  {:else}
    <div class="grid-empty">
      <Icon name="check" size={mini ? 16 : 22} />
      <span>{emptyResultLabel}</span>
    </div>
  {/if}
{:else}
  <div class="grid-wrap" class:mini>
    {#if result.message && !mini}
      <div class="grid-notice mono" title={result.message}>{result.message}</div>
    {/if}
    {#if !mini}
      <div class="grid-toolbar">
        <div class="gt-search">
          <Icon name="search" size={11} />
          <input
            class="gt-search-input mono"
            type="text"
            placeholder="Search rows…"
            bind:value={search}
            spellcheck="false"
            autocomplete="off"
          />
          {#if filtering}
            <button class="gt-search-clear" title="Clear search" aria-label="Clear search" onclick={() => (search = '')}>
              <Icon name="x" size={10} />
            </button>
          {/if}
        </div>
        <span class="grow"></span>
        {#if editable}
          <span
            class="gt-edit-hint"
            title="Double-click a cell to edit (you review the SQL before it runs). Primary key {editPkCols.length > 1 ? 'columns' : 'column'} ({editPkCols.join(', ')}) {editPkCols.length > 1 ? 'are' : 'is'} read-only."
          >
            <Icon name="edit" size={10} />double-click to edit
          </span>
        {/if}
        <div class="view-seg" role="tablist" aria-label="Result view">
          <button class="vs" class:on={viewMode === 'grid'} role="tab" aria-selected={viewMode === 'grid'} onclick={() => (viewMode = 'grid')} title="Columnar grid">Grid</button>
          <button class="vs" class:on={viewMode === 'vertical'} role="tab" aria-selected={viewMode === 'vertical'} onclick={() => (viewMode = 'vertical')} title="One record per block (field: value)">Vertical</button>
          <button class="vs" class:on={viewMode === 'json'} role="tab" aria-selected={viewMode === 'json'} onclick={() => (viewMode = 'json')} title="One JSON object per row">JSON</button>
        </div>
        {#if viewMode === 'grid'}
          <button
            class="tb-btn"
            class:on={expandJson}
            onclick={() => (expandJson = !expandJson)}
            title="Expand all nested JSON cells inline (instead of clicking each)"
          ><Icon name={expandJson ? 'minimize' : 'maximize'} size={11} />{expandJson ? 'Collapse' : 'Expand'} JSON</button>
        {/if}
        {#if result?.masked}
          <span class="tb-masked" title="Server-side PII masking was applied — sensitive values were redacted before leaving the server">
            <Icon name="lock" size={11} />Masked
          </span>
        {/if}
        <button class="tb-btn" onclick={sendToRunningAgent} title="Paste this query + result into your running agent (so it sees the real DB state)"><Icon name="comment" size={11} />→ Agent</button>
        {#if connectionId}
          <button class="tb-btn" onclick={examineWithAi} title="Investigate this result with the DB Assistant agent (read-only, side-by-side)"><Icon name="zap" size={11} />Examine with AI</button>
        {/if}
        <button class="tb-btn" onclick={copyTsv} title="Copy as TSV{exportScope}"><Icon name="file" size={11} />Copy</button>
        <button class="tb-btn" onclick={exportCsv} title="Export CSV{exportScope}"><Icon name="arrowDown" size={11} />CSV</button>
        <button class="tb-btn" onclick={exportJson} title="Export JSON{exportScope}"><Icon name="arrowDown" size={11} />JSON</button>
        {#if connectionId && statement}
          <button
            class="tb-btn"
            class:accent={result?.truncated}
            onclick={openExportDialog}
            title="Export ALL rows — streams the full (uncapped) result to a file on the daemon host, in a selectable format, with live progress"
          ><Icon name="arrowDown" size={11} />Export all rows…</button>
        {/if}
        {#if connectionId && (database.capabilities?.sql || database.capabilities?.engine === 'mongodb')}
          <button
            class="tb-btn"
            onclick={() => database.openImportDialog()}
            title="Import a local file (CSV/TSV/NDJSON/JSON) into a table or collection — batched writes through the same write guard"
          ><Icon name="arrowDown" size={11} />Import file…</button>
        {/if}
      </div>
    {/if}

    {#if !mini && selected.size > 0}
      <div class="sel-bar">
        <span class="sel-count">{selected.size} selected</span>
        {#if copyTarget}
          <button
            class="sel-gen"
            onclick={copySelectedAsInsert}
            title={engine === 'mongodb'
              ? 'Open the selected rows as an insertMany(…) in a new tab (not run)'
              : 'Open the selected rows as INSERT statements in a new tab (not run)'}
          >
            <Icon name="file" size={11} />Copy as INSERT
          </button>
        {/if}
        {#if editable}
          <button
            class="sel-gen"
            onclick={copySelectedWhere}
            title="Copy a `pk IN (…)` predicate for the selected rows to the clipboard"
          >
            <Icon name="file" size={11} />WHERE pk IN (…)
          </button>
        {/if}
        <button class="sel-del" onclick={deleteSelected} title="Delete selected rows (you review before it runs)">
          <Icon name="trash" size={11} />Delete…
        </button>
        <button class="sel-clear" onclick={clearSelection}>Clear</button>
        <span class="sel-hint">you'll review the statement before it runs</span>
      </div>
    {/if}

    {#if !mini && database.filters.length > 0}
      <div class="filter-bar">
        <span class="fb-label"><Icon name="search" size={11} />Filters</span>
        {#each database.filters as cond, ci (ci)}
          {#if cond.kind === 'raw'}
            <span class="chip raw" title="Existing WHERE condition">
              <span class="chip-text mono">{cond.text}</span>
              <button class="chip-x" title="Remove" aria-label="Remove" onclick={() => database.removeFilterCond(ci)}><Icon name="x" size={9} /></button>
            </span>
          {:else}
            <span class="chip" class:exclude={cond.op === 'not_in'}>
              <button
                class="chip-op"
                title={cond.op === 'in' ? 'Include (click to exclude)' : 'Exclude (click to include)'}
                onclick={() => database.toggleFilterMode(ci)}
              >{cond.op === 'in' ? '=' : '≠'}</button>
              <span class="chip-col mono">{cond.column}</span>
              {#each cond.values as val, vi (vi)}
                <span class="chip-val mono">
                  {val.isNull ? 'NULL' : val.raw}
                  <button class="val-x" aria-label="Remove value" onclick={() => database.removeFilterValue(ci, vi)}>×</button>
                </span>
              {/each}
              <input
                class="chip-add mono"
                placeholder="+ value"
                bind:value={addValText[ci]}
                onkeydown={(e) => { if (e.key === 'Enter') submitFilterValue(ci); }}
              />
              <button class="chip-x" title="Remove filter" aria-label="Remove filter" onclick={() => database.removeFilterCond(ci)}><Icon name="x" size={9} /></button>
            </span>
          {/if}
        {/each}
        <button class="fb-clear" onclick={() => database.clearFilters()} title="Clear all filters">Clear all</button>
        <span class="fb-hint">filtering loaded rows — press Run to re-query the server</span>
      </div>
    {/if}

    {#if viewMode === 'json'}
      <!-- One JSON object PER ROW (not one big array) so row boundaries are
           unmistakable: each row is its own bordered, numbered, copyable block.
           Same data the server returned — only the rendering differs. -->
      <div class="alt-view">
        {#if viewTruncated}<div class="alt-note dim">Showing first {VIEW_CAP} of {viewRows.length} rows.</div>{/if}
        {#each objRows as { obj, idx }, ri (ri)}
          <div class="jrec">
            <div class="jrec-head mono">
              <span class="jrec-n">#{ri + 1}</span>
              {#if editable && !reviewSql}
                <button class="jrec-copy" title="Edit this document (opens a review before running)" aria-label="Edit document" onclick={() => openDocEditor(idx)}><Icon name="edit" size={10} /></button>
              {/if}
              <button class="jrec-copy" title="Copy this row as JSON" aria-label="Copy row JSON" onclick={() => copyText(prettyJson(obj))}><Icon name="file" size={10} /></button>
            </div>
            <!-- Collapsible tree, NOT a stringified blob: a closed branch renders
                 one summary line, so a 90KB document costs a handful of nodes. -->
            <div class="alt-json mono"><JsonTree value={obj} /></div>
          </div>
        {/each}
        {#if altRemaining > 0}
          <button class="alt-more" onclick={() => (altShown += ALT_BATCH)}>
            Show {Math.min(ALT_BATCH, altRemaining)} more · {altRemaining} not rendered
          </button>
        {/if}
      </div>
    {:else if viewMode === 'vertical'}
      <div class="alt-view">
        {#if viewTruncated}<div class="alt-note dim">Showing first {VIEW_CAP} of {viewRows.length} rows.</div>{/if}
        {#each objRows as { obj, idx }, ri (ri)}
          <div class="vrec">
            <div class="vrec-head mono">
              #{ri + 1}
              {#if editable && !reviewSql}
                <button class="jrec-copy" title="Edit this record (opens a review before running)" aria-label="Edit record" onclick={() => openDocEditor(idx)}><Icon name="edit" size={10} /></button>
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
          <button class="alt-more" onclick={() => (altShown += ALT_BATCH)}>
            Show {Math.min(ALT_BATCH, altRemaining)} more · {altRemaining} not rendered
          </button>
        {/if}
      </div>
    {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions a11y_no_static_element_interactions -->
    <div
      class="grid-scroll"
      bind:this={scrollEl}
      onscroll={onScroll}
      tabindex={mini ? undefined : 0}
      role={mini ? undefined : 'group'}
      aria-label={mini
        ? undefined
        : 'Results grid — arrow keys move, Enter edits or expands, ⌘C copies the cell, Shift+F10 opens the row menu'}
      onkeydown={onGridKeydown}
    >
      <table class="grid mono" class:expanded={expandJson} style="--last:{result.columns.length}; --row-h:{ROW_H}px">
        <thead>
          <tr>
            <th class="rownum">
              {#if editable}
                <input
                  class="sel-box"
                  type="checkbox"
                  checked={allInViewSelected}
                  onchange={toggleAllInView}
                  title="Select all rows in view"
                  aria-label="Select all rows"
                />
              {:else}#{/if}
            </th>
            {#each result.columns as c, ci (ci)}
              <th
                title={mini ? (c.type_hint ?? undefined) : `${c.name} — click to sort, right-click for filters`}
                class:pk={editable && editPkCols.includes(c.name)}
                class:sortable={!mini}
                class:sorted={sortCol === ci}
                aria-sort={sortCol === ci ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'}
                style="width:{widthFor(ci)}ch; max-width:{widthFor(ci)}ch;"
                oncontextmenu={(e) => headerMenu(e, ci)}
              >
                {#if mini}
                  <span class="th-inner">
                    <span class="th-name">{c.name}</span>
                    {#if c.type_hint}<span class="th-type">{c.type_hint}</span>{/if}
                  </span>
                {:else}
                  <button class="th-sort" type="button" onclick={() => cycleSort(ci)}>
                    <span class="th-inner">
                      <span class="th-name">{c.name}</span>
                      {#if editable && editPkCols.includes(c.name)}<span class="th-pk" title="Primary key (read-only)">PK</span>{/if}
                      {#if c.type_hint}<span class="th-type">{c.type_hint}</span>{/if}
                    </span>
                    <span class="th-sort-ind" class:on={sortCol === ci} aria-hidden="true"
                      >{sortCol === ci ? (sortDir === 'asc' ? '▲' : '▼') : '↕'}</span
                    >
                  </button>
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <span
                    class="th-resize"
                    class:active={dragName === c.name}
                    onpointerdown={(e) => startResize(e, ci)}
                    onpointermove={onResizeMove}
                    onpointerup={endResize}
                    onpointercancel={endResize}
                  ></span>
                {/if}
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#if padTop > 0}
            <tr class="spacer" aria-hidden="true"><td colspan={result.columns.length + 1} style="height:{padTop}px"></td></tr>
          {/if}
          {#each windowRows as { row, idx }, wi (idx)}
            {@const vpos = startIdx + wi}
            <tr class:odd={idx % 2 === 1} class:selected={selected.has(idx)}>
              <td class="rownum">
                {#if editable}
                  <input
                    class="sel-box"
                    type="checkbox"
                    checked={selected.has(idx)}
                    onclick={(e) => toggleRow(idx, e)}
                    title="Select row (shift-click for a range)"
                    aria-label="Select row {idx + 1}"
                  />
                {/if}
                <span class="rownum-n">{idx + 1}</span>
                {#if editable}
                  <button
                    class="row-dup"
                    title="Duplicate row (review INSERT before running)"
                    aria-label="Duplicate row"
                    onclick={() => duplicateRow(idx)}
                  >
                    <Icon name="plus" size={11} />
                  </button>
                {/if}
              </td>
              {#each result.columns as _c, ci (ci)}
                {@const v = row[ci]}
                {@const w = widthFor(ci)}
                {#if editing && editing.rowIdx === idx && editing.colIdx === ci}
                  <td class="cell editing" style="width:{w}ch; max-width:{w}ch;">
                    <!-- svelte-ignore a11y_autofocus -->
                    <input
                      class="cell-input mono"
                      bind:value={editing.value}
                      use:focusEditor
                      onkeydown={onEditKeydown}
                      onblur={commitEdit}
                    />
                  </td>
                {:else if pendingValue(idx, ci) !== undefined}
                  {@const pv = pendingValue(idx, ci) ?? ''}
                  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                  <td
                    class="cell dirty"
                    class:kbd-focus={focusCell?.r === vpos && focusCell?.c === ci}
                    title="Pending change — Review & apply (bar below) writes it; double-click to keep editing"
                    style="width:{w}ch; max-width:{w}ch;"
                    onclick={() => (focusCell = { r: vpos, c: ci })}
                    ondblclick={() => beginEdit(idx, ci)}
                    oncontextmenu={(e) => cellMenu(e, ci, v, idx)}
                  >{#if pv === '' || pv === SET_NULL}<span class="null-glyph">∅</span>{:else if pv === SET_EMPTY}<span class="null-glyph">''</span>{:else}{pv}{/if}</td>
                {:else if v === null || v === undefined}
                  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                  <td
                    class="cell null"
                    class:editable={isEditableCell(ci)}
                    class:kbd-focus={focusCell?.r === vpos && focusCell?.c === ci}
                    title="NULL"
                    style="width:{w}ch; max-width:{w}ch;"
                    onclick={() => (focusCell = { r: vpos, c: ci })}
                    ondblclick={() => beginEdit(idx, ci)}
                    oncontextmenu={(e) => cellMenu(e, ci, v, idx)}
                  ><span class="null-glyph">∅</span></td>
                {:else if isComplex(v)}
                  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                  <td
                    class="cell json"
                    class:wrap={expandJson}
                    class:kbd-focus={focusCell?.r === vpos && focusCell?.c === ci}
                    title="Click to expand"
                    style="width:{w}ch; max-width:{w}ch;"
                    onclick={() => {
                      focusCell = { r: vpos, c: ci };
                      openCell(v, idx, ci);
                    }}
                    ondblclick={() => { openCell(v, idx, ci); startViewerEdit(); }}
                    oncontextmenu={(e) => cellMenu(e, ci, v, idx)}
                  >{clip(expandJson ? prettyJson(v) : compactJson(v))}<button class="cell-expand" title="Expand value" aria-label="Expand value" onclick={(e) => { e.stopPropagation(); openCell(v, idx, ci); }}><Icon name="maximize" size={9} /></button></td>
                {:else}
                  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                  <td
                    class="cell"
                    class:editable={isEditableCell(ci)}
                    class:kbd-focus={focusCell?.r === vpos && focusCell?.c === ci}
                    style="width:{w}ch; max-width:{w}ch;"
                    onclick={() => (focusCell = { r: vpos, c: ci })}
                    ondblclick={() => beginEdit(idx, ci)}
                    oncontextmenu={(e) => cellMenu(e, ci, v, idx)}
                  >{#if filtering}{#each highlightParts(cellDisplay(v)) as part}{#if part.hit}<mark>{part.t}</mark>{:else}{part.t}{/if}{/each}{:else}{cellDisplay(v)}{/if}<button class="cell-expand" title="Expand value" aria-label="Expand value" onclick={(e) => { e.stopPropagation(); openCell(v, idx, ci); }}><Icon name="maximize" size={9} /></button></td>
                {/if}
              {/each}
            </tr>
          {/each}
          {#if padBottom > 0}
            <tr class="spacer" aria-hidden="true"><td colspan={result.columns.length + 1} style="height:{padBottom}px"></td></tr>
          {/if}
        </tbody>
      </table>
    </div>
    {/if}
    {#if pendingCells > 0}
      <div class="pending-bar" data-testid="pending-edits-bar">
        <Icon name="edit" size={12} />
        <span>
          <strong>{pendingCells}</strong> pending change{pendingCells === 1 ? '' : 's'} on
          <strong>{pendingEdits.size}</strong> row{pendingEdits.size === 1 ? '' : 's'} — nothing
          is written until you review &amp; run.
        </span>
        <span class="pending-spacer"></span>
        <button class="btn small ghost" onclick={discardPending}>Discard</button>
        <button class="btn small primary" onclick={reviewPending}>Review &amp; apply</button>
      </div>
    {/if}
    {#if !mini}
      <div class="grid-foot">
        {#if filtering || chipFiltering}
          <span><strong>{viewRows.length}</strong> of {liveRows.length} row{liveRows.length === 1 ? '' : 's'}</span>
        {:else}
          <span><strong>{result.stats.row_count}</strong> row{result.stats.row_count === 1 ? '' : 's'}</span>
        {/if}
        {#if sorting && sortCol !== null}
          <button class="sort-chip" title="Clear sort" onclick={() => { sortCol = null; sortDir = null; }}>
            {sortDir === 'asc' ? '▲' : '▼'} {result.columns[sortCol].name}
            <Icon name="x" size={9} />
          </button>
        {/if}
        <span class="dot">·</span>
        <span>{result.stats.duration_ms} ms</span>
        {#if result.stats.bytes_read != null}
          <span class="dot">·</span>
          <span>{fmtBytes(result.stats.bytes_read)} read</span>
        {/if}
        {#if result.rows_affected != null}
          <span class="dot">·</span>
          <span>{result.rows_affected} affected</span>
        {/if}
        {#if result.truncated}
          <span
            class="trunc-badge"
            title="Row cap reached — more rows exist. Raise the Limit or add an explicit LIMIT to fetch more."
            >capped at {result.stats.row_count.toLocaleString()}</span
          >
        {/if}
        {#if showPager}
          <span class="dot">·</span>
          <span class="pager">
            <button class="pg-btn" disabled={offset <= 0} onclick={() => database.runPage(-1)} title="Previous page" aria-label="Previous page">‹ Prev</button>
            <span class="pg-range mono">rows {pageFrom.toLocaleString()}–{pageTo.toLocaleString()}</span>
            <button class="pg-btn" disabled={!hasNextPage} onclick={() => database.runPage(1)} title="Next page" aria-label="Next page">Next ›</button>
            {#if !hasOrderBy}<span class="pg-unordered" title="Without an ORDER BY, row order can shift between pages">unordered</span>{/if}
          </span>
        {/if}
        {#if !editable && statement}
          <span class="grow"></span>
          <span class="edit-note" title={editReason ?? undefined}
            >{editReason ?? 'Editing needs a single-table result with a primary key'}</span
          >
        {:else if result.message}
          <span class="grow"></span>
          <span class="msg">{result.message}</span>
        {/if}
      </div>
    {/if}
    {#if running && !mini}
      <!-- Running overlay: dims the stale grid while the active tab's query is in
           flight, with an elapsed counter + inline Cancel (client + engine stop). -->
      <div class="rg-overlay" role="status" aria-live="polite">
        <div class="rg-overlay-card">
          <span class="rg-spin"><Icon name="refresh" size={16} /></span>
          <span class="rg-overlay-text">Running… {elapsed}s</span>
          <button class="rg-cancel" onclick={() => database.abortQuery()} title="Cancel the running query">
            <Icon name="x" size={11} />Cancel
          </button>
        </div>
      </div>
    {/if}
  </div>
{/if}

{#if viewer}
  <div
    class="cell-viewer-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) viewer = null;
    }}
  >
    <div
      class="cell-viewer"
      role="dialog"
      aria-modal="true"
      aria-label="Cell value"
      use:dialogKeys={() => (viewer = null)}
    >
      <div class="cv-head">
        <span>Cell value</span>
        <span class="grow"></span>
        {#if viewer.sql}
          <button
            class="tb-btn"
            class:active={viewer.formatted}
            onclick={() => (viewer && (viewer.formatted = !viewer.formatted))}
            title="Toggle SQL formatting"
          >
            <Icon name="grid" size={11} />{viewer.formatted ? 'Formatted' : 'Raw'}
          </button>
        {/if}
        {#if viewer.edit && !viewerEditing}
          <button class="tb-btn" onclick={startViewerEdit} title="Edit this cell value">
            <Icon name="edit" size={11} />Edit
          </button>
        {/if}
        <button class="tb-btn" onclick={copyViewer} title="Copy full value"><Icon name="file" size={11} />Copy</button>
        <button class="icon-btn" onclick={() => (viewer = null)} aria-label="Close">✕</button>
      </div>
      {#if viewerEditing}
        <!-- svelte-ignore a11y_autofocus -->
        <textarea
          class="cv-edit mono"
          bind:value={viewerDraft}
          spellcheck="false"
          autofocus
          onkeydown={(e) => {
            if (e.key === 'Escape') { e.stopPropagation(); viewerEditing = false; viewerErr = null; }
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) saveViewerEdit();
          }}
        ></textarea>
        <div class="cv-foot">
          {#if viewerErr}<span class="cv-err">{viewerErr}</span>{/if}
          <span class="grow"></span>
          <button class="btn small ghost" onclick={() => { viewerEditing = false; viewerErr = null; }}>Cancel</button>
          <button class="btn small primary" onclick={saveViewerEdit} title="Validate and review the update (⌘⏎)">Save…</button>
        </div>
      {:else}
        <pre class="cv-body mono">{viewerText}</pre>
      {/if}
    </div>
  </div>
{/if}

{#if docEditor}
  <div
    class="cell-viewer-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) docEditor = null;
    }}
  >
    <div
      class="cell-viewer"
      role="dialog"
      aria-modal="true"
      aria-label="Edit document"
      use:dialogKeys={() => (docEditor = null)}
    >
      <div class="cv-head">
        <span>Edit document <span class="dim">— row is replaced/updated after review</span></span>
        <span class="grow"></span>
        <button class="icon-btn" onclick={() => (docEditor = null)} aria-label="Close">✕</button>
      </div>
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        class="cv-edit mono"
        bind:value={docEditor.draft}
        spellcheck="false"
        autofocus
        onkeydown={(e) => {
          if (e.key === 'Escape') { e.stopPropagation(); docEditor = null; }
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) saveDocEdit();
        }}
      ></textarea>
      <div class="cv-foot">
        {#if docEditor.err}<span class="cv-err">{docEditor.err}</span>{/if}
        <span class="grow"></span>
        <button class="btn small ghost" onclick={() => (docEditor = null)}>Cancel</button>
        <button class="btn small primary" onclick={saveDocEdit} title="Validate and review the statement (⌘⏎)">Save…</button>
      </div>
    </div>
  </div>
{/if}

{#if showExportDialog}
  <Modal
    title="Export all rows"
    width={520}
    onclose={() => {
      exportAbort?.abort();
      showExportDialog = false;
    }}
  >
    <div class="exp-form">
      <p class="exp-hint">
        Runs the statement on the daemon host and <strong>streams</strong> the full result to a local
        file — for sets too large to pull into the browser. Choose the format, destination directory,
        and an optional row limit.
      </p>

      <label class="exp-row">
        <span class="exp-label">Format</span>
        <select class="exp-select" bind:value={exportFormat} onchange={onFormatChange}>
          {#each EXPORT_FORMATS as f (f.value)}
            <option value={f.value}>{f.label}</option>
          {/each}
        </select>
      </label>

      <div class="exp-row">
        <span class="exp-label">Folder</span>
        <div class="exp-dir">
          <input class="exp-input mono" bind:value={exportDir} spellcheck="false" placeholder="~/Downloads" />
          <button class="tb-btn" onclick={() => (pickingDir = true)} title="Browse the daemon host">
            <Icon name="folder" size={11} />Browse…
          </button>
        </div>
      </div>

      <label class="exp-row">
        <span class="exp-label">File name</span>
        <input class="exp-input mono" bind:value={exportName} spellcheck="false" placeholder="result.csv" />
      </label>

      <label class="exp-row">
        <span class="exp-label">Row limit</span>
        <input
          class="exp-input mono"
          bind:value={exportLimit}
          type="number"
          min="1"
          spellcheck="false"
          placeholder="all rows"
        />
      </label>

      <div class="exp-dest mono" title="Resolved destination on the daemon host">
        → {joinPath(exportDir.trim() || '~/Downloads', exportName.trim() || defaultExportName())}
      </div>

      {#if exportingPath}
        <div class="exp-progress" role="status" aria-live="polite">
          <div class="exp-bar"><div class="exp-bar-fill"></div></div>
          <div class="exp-prog-text mono">
            {exportProgress ? fmtBytes(exportProgress.bytes) : '0 B'} written…
          </div>
        </div>
      {/if}
    </div>

    {#snippet footer()}
      <button
        class="btn"
        onclick={() => (exportingPath ? exportAbort?.abort() : (showExportDialog = false))}
        title={exportingPath ? 'Stop the running export (the partial file is left in place)' : undefined}
      >
        {exportingPath ? 'Cancel export' : 'Cancel'}
      </button>
      <button class="btn primary" onclick={() => void runPathExport()} disabled={exportingPath}>
        {exportingPath ? 'Exporting…' : 'Export all'}
      </button>
    {/snippet}
  </Modal>
{/if}

{#if pickingDir}
  <FolderPicker
    title="Choose export folder (daemon host)"
    start={exportDir}
    onpick={(p) => {
      exportDir = p;
      pickingDir = false;
    }}
    onclose={() => (pickingDir = false)}
  />
{/if}

{#if reviewSql}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="cell-viewer-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeReview();
    }}
    onkeydown={onReviewKeydown}
  >
    <div
      class="review-modal"
      role="dialog"
      aria-modal="true"
      aria-label={reviewSql.title}
      use:dialogKeys={closeReview}
    >
      <div class="cv-head">
        <span>{reviewSql.title}</span>
        <button class="icon-btn" onclick={closeReview} disabled={runningReview} aria-label="Close">✕</button>
      </div>
      <div class="review-body">
        <p class="review-hint">Review and edit the statement before running. This will run against the connection.</p>
        <!-- svelte-ignore a11y_autofocus -->
        <textarea
          class="review-sql mono"
          bind:value={reviewSql.sql}
          disabled={runningReview}
          spellcheck="false"
          autofocus
          rows="5"
        ></textarea>
      </div>
      <div class="review-foot">
        <span class="review-kbd mono">⌘↵ to run · Esc to cancel</span>
        <span class="grow"></span>
        <button class="tb-btn" onclick={closeReview} disabled={runningReview}>Cancel</button>
        <button class="tb-btn primary" onclick={runReview} disabled={runningReview || !reviewSql.sql.trim()}>
          <Icon name="play" size={11} />{runningReview ? 'Running…' : 'Run'}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if sendToAgentOpen && ws.current && sendToAgentPayload !== null}
  <ContextPacketDialog
    workspaceId={ws.current.id}
    sessionId={ws.targetAgentId}
    kind="db"
    payload={sendToAgentPayload}
    onclose={() => (sendToAgentOpen = false)}
  />
{/if}

<style>
  .grid-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    height: 100%;
    /* Anchors the running overlay. */
    position: relative;
  }
  /* ── Multi-result switcher ── */
  .rg-switch {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-wrap: wrap;
    padding: 2px 2px 8px;
    flex-shrink: 0;
  }
  .rg-seg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
    white-space: nowrap;
  }
  .rg-seg:hover {
    color: var(--text);
  }
  .rg-seg.on {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .rg-seg.err {
    border-color: color-mix(in srgb, var(--status-exited) 45%, transparent);
  }
  .rg-seg-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-exited);
    flex-shrink: 0;
  }
  /* ── Running overlay ── */
  .rg-overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg) 55%, transparent);
    backdrop-filter: blur(1px);
    z-index: 5;
  }
  /* Inline variant for the no-result branches — same card, no dimmer. */
  .rg-overlay.rg-inline {
    position: static;
    inset: auto;
    background: none;
    backdrop-filter: none;
  }
  .rg-overlay-card {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow);
    font-size: 12.5px;
    color: var(--text);
  }
  .rg-spin {
    display: grid;
    place-items: center;
    color: var(--accent);
    animation: rg-spin 0.9s linear infinite;
  }
  @keyframes rg-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .rg-overlay-text {
    font-variant-numeric: tabular-nums;
  }
  .rg-cancel {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid color-mix(in srgb, var(--status-exited) 55%, transparent);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
    color: var(--status-exited);
    border-radius: var(--radius-s);
    font-size: 11.5px;
    font-weight: 600;
    padding: 3px 9px;
    cursor: pointer;
  }
  .rg-cancel:hover {
    background: color-mix(in srgb, var(--status-exited) 24%, transparent);
  }
  /* ── Footer pager ── */
  .pager {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .pg-btn {
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: 11px;
    padding: 1px 8px;
    cursor: pointer;
  }
  .pg-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .pg-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .pg-range {
    font-size: 11px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .pg-unordered {
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--status-warn);
    background: var(--status-warn-soft);
    border-radius: 999px;
    padding: 1px 6px;
  }
  .grid-empty,
  .grid-error {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 28px 16px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .grid-error {
    color: var(--status-exited);
    justify-content: flex-start;
    align-items: flex-start;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
  }
  .grid-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    row-gap: 6px;
    padding: 4px 2px 8px;
  }
  /* Notice shown above results (e.g. the Mongo command a SQL query translated to). */
  .grid-notice {
    font-size: 11px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 9%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 22%, transparent);
    border-radius: var(--radius-s);
    padding: 4px 8px;
    margin-bottom: 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* ── Quick-filter bar ── */
  .filter-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 8px;
    margin-bottom: 8px;
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 5%, var(--surface-2));
  }
  .fb-label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 4px 0 0;
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: 999px;
    background: var(--surface);
    font-size: 11px;
  }
  .chip.exclude {
    border-color: color-mix(in srgb, var(--status-exited) 45%, transparent);
  }
  .chip.raw {
    padding: 0 4px 0 9px;
    border-style: dashed;
  }
  .chip-op {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    margin: 0 0 0 1px;
    border: none;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    font-weight: 700;
    cursor: pointer;
  }
  .chip.exclude .chip-op {
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    color: var(--status-exited);
  }
  .chip-col {
    font-weight: 600;
    color: var(--text);
  }
  .chip-val {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 3px 0 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .val-x {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 13px;
    line-height: 1;
    padding: 0 1px;
  }
  .val-x:hover {
    color: var(--status-exited);
  }
  .chip-add {
    width: 64px;
    height: 18px;
    border: none;
    border-bottom: 1px dashed var(--border);
    background: transparent;
    color: var(--text);
    font-size: 11px;
    outline: none;
  }
  .chip-text {
    color: var(--text-dim);
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .chip-x:hover {
    background: color-mix(in srgb, var(--status-exited) 20%, transparent);
    color: var(--status-exited);
  }
  .fb-clear {
    height: 20px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    font-size: 10.5px;
    cursor: pointer;
  }
  .fb-clear:hover {
    color: var(--status-exited);
    border-color: color-mix(in srgb, var(--status-exited) 40%, transparent);
  }
  .fb-hint {
    font-size: 10.5px;
    color: var(--text-dim);
    font-style: italic;
    margin-inline-start: auto;
  }
  .gt-search {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 7px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    min-width: 180px;
  }
  .gt-search:focus-within {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    color: var(--accent);
  }
  .gt-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    outline: none;
    padding: 0;
  }
  .gt-search-input::placeholder {
    color: var(--text-dim);
  }
  .gt-search-clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 15px;
    height: 15px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .gt-search-clear:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
  }
  .gt-edit-hint {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 18px;
    padding: 0 7px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    cursor: help;
  }
  .view-seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .vs {
    height: 22px;
    padding: 0 9px;
    border: none;
    border-inline-end: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
  }
  .vs:last-child {
    border-inline-end: none;
  }
  .vs.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
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
  .tb-btn.on {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .tb-btn:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .tb-btn.active {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  /* Nudge the user toward the full export when the shown result is capped. */
  .tb-btn.accent {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    color: var(--accent);
  }
  /* Server-side masking badge — shown in toolbar when result.masked is true. */
  .tb-masked {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    font-size: 11.5px;
    font-weight: 600;
  }
  .grid-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .grid-scroll:focus {
    outline: none;
  }
  .grid-scroll:focus-visible {
    outline: 1px solid color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: -1px;
  }
  /* Roving keyboard cell cursor (see onGridKeydown). */
  .grid tbody td.kbd-focus {
    outline: 1.5px solid var(--accent);
    outline-offset: -1.5px;
  }
  .grid {
    border-collapse: collapse;
    table-layout: fixed;
    width: max-content;
    min-width: 100%;
    user-select: text;
  }
  .grid thead th {
    position: sticky;
    top: 0;
    z-index: 2;
    text-align: start;
    padding: 5px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    border-inline-end: 1px solid var(--border);
    font-size: 11px;
    white-space: nowrap;
    vertical-align: bottom;
    overflow: hidden;
  }
  /* When sortable, the header content lives in a button that fills the cell. */
  .grid thead th.sortable {
    padding: 0;
  }
  .grid thead th.sorted {
    background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
  }
  .th-sort {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 6px;
    width: 100%;
    /* leave a sliver on the right for the resize handle */
    padding: 5px 12px 5px 10px;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .th-sort:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .th-sort-ind {
    flex: 0 0 auto;
    font-size: 8.5px;
    line-height: 1;
    color: var(--text-dim);
    opacity: 0;
    transform: translateY(-1px);
    transition: opacity 0.12s;
  }
  .th-sort:hover .th-sort-ind {
    opacity: 0.55;
  }
  .th-sort-ind.on {
    opacity: 1;
    color: var(--accent);
  }
  .th-inner {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .th-name {
    font-weight: 700;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .th-pk {
    flex: 0 0 auto;
    font-size: 8.5px;
    font-weight: 800;
    letter-spacing: 0.04em;
    padding: 0 4px;
    border-radius: 3px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    transform: translateY(-1px);
  }
  .th-type {
    flex: 0 0 auto;
    font-weight: 400;
    font-size: 10px;
    color: var(--text-dim);
  }
  /* Drag handle on the header's right edge. */
  .th-resize {
    position: absolute;
    top: 0;
    right: -3px;
    width: 7px;
    height: 100%;
    cursor: col-resize;
    z-index: 4;
    touch-action: none;
  }
  .th-resize::after {
    content: '';
    position: absolute;
    top: 4px;
    bottom: 4px;
    left: 3px;
    width: 1px;
    background: transparent;
  }
  .th-resize:hover::after,
  .th-resize.active::after {
    background: var(--accent);
  }
  .grid td {
    padding: 4px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    border-inline-end: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    vertical-align: middle;
    color: var(--text);
  }
  /* Fixed row height keeps the virtualization math exact (ROW_H in script). */
  .grid tbody td {
    box-sizing: border-box;
    height: 26px;
  }
  /* Expand-JSON mode: taller uniform rows (matches ROW_H via --row-h) so the
     virtualization math stays exact; complex cells pretty-print + wrap. */
  .grid.expanded tbody tr:not(.spacer) td {
    height: var(--row-h);
    vertical-align: top;
  }
  .grid.expanded .cell.json.wrap {
    white-space: pre-wrap;
    overflow: auto;
    line-height: 1.4;
  }
  /* Stripe by data-row index (not :nth-child) so the pattern stays stable as
     the virtualized window scrolls. */
  .grid tbody tr.odd td {
    background: color-mix(in srgb, var(--text-dim) 4%, transparent);
  }
  .grid tbody tr:not(.spacer):hover td {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  /* Spacer rows reserve scroll height for the off-screen (un-rendered) rows. */
  .grid tbody tr.spacer td {
    padding: 0;
    border: none;
    background: transparent;
    height: auto;
  }
  .rownum {
    color: var(--text-dim);
    text-align: end;
    font-size: 10.5px;
    position: sticky;
    inset-inline-start: 0;
    background: var(--surface-2);
    z-index: 1;
    width: 4ch;
    max-width: 4ch;
  }
  .grid thead .rownum {
    z-index: 3;
  }
  .rownum-n {
    display: inline-block;
  }
  /* Per-row duplicate action: revealed on row hover, anchored to the RIGHT of the
   * # cell so it never covers the selection checkbox. */
  .row-dup {
    position: absolute;
    top: 0;
    bottom: 0;
    inset-inline-end: 0;
    width: 2.2ch;
    display: none;
    align-items: center;
    justify-content: center;
    border: none;
    background: color-mix(in srgb, var(--accent) 14%, var(--surface-2));
    color: var(--accent);
    cursor: pointer;
    padding: 0;
  }
  .grid tbody tr:hover .row-dup {
    display: flex;
  }
  .row-dup:hover {
    background: color-mix(in srgb, var(--accent) 26%, var(--surface-2));
  }
  /* Selection checkbox in the # column (only present for editable results). */
  .rownum:has(.sel-box) {
    width: 6ch;
    max-width: 6ch;
    text-align: start;
    padding-inline-start: 5px;
  }
  .sel-box {
    width: 12px;
    height: 12px;
    margin: 0 4px 0 0;
    vertical-align: middle;
    cursor: pointer;
    accent-color: var(--accent);
  }
  .grid tbody tr.selected td {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .grid tbody tr.selected:not(.spacer):hover td {
    background: color-mix(in srgb, var(--accent) 24%, transparent);
  }
  /* Selection action bar (shown when ≥1 row is selected). */
  .sel-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--accent) 6%, var(--surface-2));
    font-size: 11px;
  }
  .sel-count {
    font-weight: 600;
    color: var(--accent);
  }
  .sel-del {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 9px;
    border-radius: 5px;
    border: 1px solid color-mix(in srgb, var(--danger, #e5484d) 50%, transparent);
    background: color-mix(in srgb, var(--danger, #e5484d) 14%, transparent);
    color: var(--danger, #e5484d);
    cursor: pointer;
  }
  .sel-del:hover {
    background: color-mix(in srgb, var(--danger, #e5484d) 24%, transparent);
  }
  /* Generate-SQL-from-selection actions (0003b) — neutral chips next to Delete. */
  .sel-gen {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 9px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .sel-gen:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .sel-clear {
    padding: 3px 8px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .sel-clear:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .sel-hint {
    color: var(--text-dim);
    font-size: 10.5px;
  }
  .cell.null {
    text-align: center;
  }
  .null-glyph {
    color: color-mix(in srgb, var(--text-dim) 75%, transparent);
    font-style: normal;
  }
  .cell.json {
    color: var(--accent);
    cursor: pointer;
  }
  .cell.json:hover {
    text-decoration: underline;
  }
  /* Expand-to-viewer affordance, revealed on cell hover (top-right corner). */
  .grid td.cell {
    position: relative;
  }
  .cell-expand {
    position: absolute;
    top: 1px;
    inset-inline-end: 1px;
    display: none;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
    box-shadow: -3px 0 5px var(--surface);
  }
  .grid td.cell:hover .cell-expand {
    display: inline-flex;
  }
  .cell-expand:hover {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .cell.editable {
    cursor: text;
  }
  .cell.editable:hover {
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .cell.editing {
    padding: 0;
    background: var(--surface) !important;
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  /* A parked (pending) cell draft: visibly different until reviewed & applied. */
  .cell.dirty {
    background: color-mix(in srgb, var(--status-warn) 14%, transparent) !important;
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--status-warn) 55%, transparent);
    font-style: italic;
    cursor: default;
  }
  .pending-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 7px;
    padding: 6px 10px;
    border: 1px solid color-mix(in srgb, var(--status-warn) 45%, transparent);
    background: color-mix(in srgb, var(--status-warn) 10%, transparent);
    border-radius: var(--radius-s);
    font-size: 11.5px;
    color: var(--text);
    flex-shrink: 0;
  }
  .pending-bar strong {
    font-variant-numeric: tabular-nums;
  }
  .pending-spacer {
    flex: 1;
  }
  .cell-input {
    width: 100%;
    height: 100%;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    padding: 4px 10px;
  }
  .cell-input:disabled {
    opacity: 0.6;
  }
  .grid td mark {
    background: color-mix(in srgb, var(--accent) 35%, transparent);
    color: var(--text);
    border-radius: 2px;
  }
  .grid-foot {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 7px 2px 0;
    font-size: 11px;
    color: var(--text-dim);
    flex-wrap: wrap;
  }
  .grid-foot strong {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .dot {
    opacity: 0.5;
  }
  .sort-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 16px;
    padding: 0 6px;
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    cursor: pointer;
  }
  .sort-chip:hover {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .trunc-badge {
    padding: 0 7px;
    height: 16px;
    line-height: 16px;
    border-radius: 999px;
    font-size: 9.5px;
    font-weight: 700;
    color: #d2691e;
    background: color-mix(in srgb, #d2691e 16%, transparent);
  }
  .msg {
    color: var(--text-dim);
    font-style: italic;
  }
  .edit-note {
    color: var(--text-dim);
    font-style: italic;
    opacity: 0.85;
  }
  .grow {
    flex: 1;
  }
  .cell-viewer-backdrop {
    position: fixed;
    inset: 0;
    z-index: 250;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
  }
  .cell-viewer {
    width: min(720px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow);
    overflow: hidden;
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
  .cv-body {
    margin: 0;
    padding: 14px;
    overflow: auto;
    font-size: 12px;
    line-height: 1.55;
    user-select: text;
    white-space: pre-wrap;
    word-break: break-word;
  }
  /* In-viewer editor (JSON/long text): fills the same band as .cv-body. */
  .cv-edit {
    flex: 1;
    min-height: 220px;
    margin: 10px 14px 0;
    padding: 10px;
    font-size: 12px;
    line-height: 1.55;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    resize: none;
    white-space: pre;
  }
  .cv-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
  }
  .cv-err {
    color: var(--status-exited);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  /* ── Local-file export dialog ─────────────────────────────────────────────── */
  .exp-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .exp-hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .exp-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .exp-label {
    flex: 0 0 76px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .exp-select,
  .exp-input {
    flex: 1;
    min-width: 0;
    padding: 6px 9px;
    font-size: 12.5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
  }
  .exp-select:focus,
  .exp-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .exp-dir {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .exp-dir .exp-input {
    flex: 1;
  }
  .exp-dest {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 6px 9px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Streaming-export progress: total size is unknown up front, so the bar is an
     indeterminate sweep + a live bytes-written readout. */
  .exp-progress {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .exp-bar {
    position: relative;
    height: 6px;
    border-radius: 999px;
    background: var(--surface-3, var(--surface-2));
    overflow: hidden;
  }
  .exp-bar-fill {
    position: absolute;
    top: 0;
    left: 0;
    height: 100%;
    width: 35%;
    border-radius: 999px;
    background: var(--accent);
    animation: exp-sweep 1.1s ease-in-out infinite;
  }
  @keyframes exp-sweep {
    0% { left: -35%; }
    100% { left: 100%; }
  }
  .exp-prog-text {
    font-size: 11px;
    color: var(--text-dim);
  }
  @media (prefers-reduced-motion: reduce) {
    .exp-bar-fill {
      animation: none;
      left: 0;
      width: 100%;
      opacity: 0.5;
    }
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     The results toolbar (search + view-segment + Copy/CSV/JSON/Agent) is dense
     — let it wrap rather than run off the edge, and make sure the grid itself
     fills its bounded block and scrolls in BOTH directions on touch. */
  /* Tablet (641–1024px): the narrowed results column can't fit the toolbar
     (Copy/CSV/JSON/Download/→Agent) on one line, so it overflows and gets
     clipped by the (overflow:hidden) ancestor. Wrap it and let the search take
     the first row — same as the phone layout, but WITHOUT the phone-only grid
     height overrides. */
  @media (min-width: 641px) and (max-width: 1024px) {
    .grid-toolbar {
      flex-wrap: wrap;
      row-gap: 6px;
    }
    .grid-toolbar .grow {
      display: none;
    }
    .gt-search {
      flex: 1 1 100%;
    }
  }

  @media (max-width: 640px) {
    .grid-toolbar {
      flex-wrap: wrap;
      row-gap: 6px;
    }
    .grid-toolbar .grow {
      display: none;
    }
    .gt-search {
      flex: 1 1 100%;
    }
    /* The grid block must have a definite height so the table scrolls inside it
       (its parent .qe-results gives it min-height on mobile). */
    .grid-wrap {
      height: 100%;
      min-height: 320px;
    }
    .grid-scroll {
      -webkit-overflow-scrolling: touch;
    }
    /* Bump tiny grid text up a notch for phone legibility. Row height is fixed
       (virtualization) so we keep cell font modest; headers can grow freely. */
    .grid thead th {
      font-size: 12.5px;
    }
    .grid td {
      font-size: 12.5px;
    }
    .gt-search-input {
      font-size: 13px;
    }
    .grid-empty,
    .grid-error {
      font-size: 13.5px;
    }
    /* Vertical / JSON views are the comfiest on a narrow phone — bump them too. */
    .alt-json,
    .vk,
    .vv {
      font-size: 13px;
    }
  }
</style>
