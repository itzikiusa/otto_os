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
  import type { QueryResult } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import { downloadText } from '../../lib/components/exporters';

  interface Props {
    result: QueryResult | null;
    error?: string | null;
    /** Compact mode for dashboard widget mini-grids (no toolbar/footer). */
    mini?: boolean;
    /** Active statement — enables editability detection when set. */
    statement?: string;
    /** Connection id the result came from — required for inline editing. */
    connectionId?: string | null;
  }
  let { result, error = null, mini = false, statement, connectionId }: Props = $props();

  // Mini widget grids are previews — cap their rendering. The main grid renders
  // ALL fetched rows via windowed virtualization (only the visible slice is in
  // the DOM), so there's no row cap there.
  const MINI_MAX = 200;

  // The rows we render/filter/sort over. Re-seeded whenever the upstream result
  // changes (edits run against the DB and refresh via re-query, not in place).
  let liveRows = $state<unknown[][]>([]);
  $effect(() => {
    // Track the result identity; reset local state on any new result.
    liveRows = result ? (mini ? result.rows.slice(0, MINI_MAX) : result.rows) : [];
    search = '';
    colWidths = {};
    sortCol = null;
    sortDir = null;
    // Jump back to the top on a fresh result.
    scrollTop = 0;
    if (scrollEl) scrollEl.scrollTop = 0;
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
  const VIEW_CAP = 500; // non-grid views aren't virtualized — cap for responsiveness
  let scrollEl = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let viewportH = $state(0);
  const virtualize = $derived(!mini);

  // ── Search / filter ─────────────────────────────────────────────────────────
  let search = $state('');
  const searchLc = $derived(search.trim().toLowerCase());
  const filtering = $derived(searchLc.length > 0);

  function rowMatches(row: unknown[]): boolean {
    for (const v of row) {
      if (v === null || v === undefined) continue;
      const s = (isComplex(v) ? compactJson(v) : String(v)).toLowerCase();
      if (s.includes(searchLc)) return true;
    }
    return false;
  }

  // Rows passing the filter, carrying their original index so edits target the
  // right entry in `liveRows`. Purely client-side over the fetched rows.
  const filteredRows = $derived.by<{ row: unknown[]; idx: number }[]>(() => {
    if (!filtering) return liveRows.map((row, idx) => ({ row, idx }));
    const out: { row: unknown[]; idx: number }[] = [];
    for (let idx = 0; idx < liveRows.length; idx++) {
      if (rowMatches(liveRows[idx])) out.push({ row: liveRows[idx], idx });
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

  // Filtered/sorted rows as plain objects (for the JSON / vertical views), capped.
  const objRows = $derived.by<Record<string, unknown>[]>(() => {
    if (!result || viewMode === 'grid') return [];
    const cols = result.columns;
    return viewRows.slice(0, VIEW_CAP).map(({ row }) => {
      const o: Record<string, unknown> = {};
      cols.forEach((c, i) => (o[c.name] = row[i]));
      return o;
    });
  });
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
  let viewer = $state<{ raw: string; sql: boolean; formatted: boolean } | null>(null);
  const viewerText = $derived(
    viewer ? (viewer.formatted ? (viewer.sql ? formatSql(viewer.raw) : viewer.raw) : viewer.raw) : '',
  );

  function isComplex(v: unknown): boolean {
    return v !== null && typeof v === 'object';
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
    if (isComplex(v)) return compactJson(v);
    return String(v);
  }
  function openCell(v: unknown): void {
    if (typeof v === 'string') {
      viewer = { raw: v, sql: looksLikeSql(v), formatted: looksLikeSql(v) };
    } else if (v === null || v === undefined) {
      viewer = { raw: 'NULL', sql: false, formatted: false };
    } else {
      viewer = { raw: prettyJson(v), sql: false, formatted: false };
    }
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

  // ── Quick-filter context menus (cell + header) ───────────────────────────────
  function shortLabel(v: unknown): string {
    const s = v === null || v === undefined ? 'NULL' : isComplex(v) ? compactJson(v) : String(v);
    return s.length > 28 ? s.slice(0, 28) + '…' : s;
  }
  function cellMenu(e: MouseEvent, ci: number, v: unknown, rowIdx: number): void {
    if (mini) return;
    const col = result?.columns[ci]?.name;
    if (!col) return;
    const short = shortLabel(v);
    const items: import('../../lib/contextmenu.svelte').MenuItem[] = [
      { label: `Filter:  ${col} = ${short}`, icon: 'search', action: () => database.addQuickFilter(col, v, 'include') },
      { label: `Exclude:  ${col} ≠ ${short}`, icon: 'x', action: () => database.addQuickFilter(col, v, 'exclude') },
      { separator: true },
      { label: 'Expand value', icon: 'maximize', action: () => openCell(v) },
      { label: 'Copy value', icon: 'file', action: () => copyText(v === null || v === undefined ? '' : isComplex(v) ? compactJson(v) : String(v)) },
    ];
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
      const len = (isComplex(v) ? compactJson(v) : String(v)).length;
      if (len > max) max = len;
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

  const editable = $derived(editPkCols.length > 0 && editTable !== null);

  /** Parse a simple SELECT … FROM <table>. Returns {db, table} or null. */
  function parseSimpleSelect(sql: string): { db: string | null; table: string } | null {
    const s = sql.trim().replace(/;\s*$/, '');
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
    const m = t.match(/^db\.([A-Za-z0-9_$.-]+)\.find\s*\(/i);
    if (m) return m[1];
    return parseSimpleSelect(t)?.table ?? null;
  }

  /** JSON-encode a value typed into a Mongo cell editor: keep numbers/bools when
   * the prior value was one; valid JSON when editing a nested object/array;
   * empty → null; else a quoted string. */
  function mongoLiteral(raw: string, prev: unknown): string {
    if (raw === '') return 'null';
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
    if (!sql || !conn || !cols || cols.length === 0) return;

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
      editReason = null;
    })();
    return () => {
      cancelled = true;
    };
  });

  // ── Inline editing ───────────────────────────────────────────────────────────
  // Edits are NOT applied directly. Committing a cell (or duplicating a row)
  // builds SQL and opens the "Review SQL" modal; the SQL only runs when the
  // user confirms there. After a successful run the grid refreshes by re-running
  // the active query, so values reflect the database (no optimistic patching).
  let editing = $state<{ rowIdx: number; colIdx: number; value: string } | null>(null);

  function isEditableCell(colIdx: number): boolean {
    if (!editable) return false;
    const name = result?.columns[colIdx]?.name;
    return !!name && !editPkCols.includes(name); // PK column(s) read-only
  }

  /** `\`pk1\` = v1 AND \`pk2\` = v2` targeting one row by its primary key. */
  function whereByPk(rowIdx: number): string {
    if (!result) return '';
    return editPkCols
      .map((pk) => {
        const ci = result!.columns.findIndex((c) => c.name === pk);
        return `\`${pk}\` = ${valueLiteral(liveRows[rowIdx][ci])}`;
      })
      .join(' AND ');
  }

  function beginEdit(rowIdx: number, colIdx: number): void {
    if (!isEditableCell(colIdx) || reviewSql) return;
    const v = liveRows[rowIdx]?.[colIdx];
    editing = { rowIdx, colIdx, value: v === null || v === undefined ? '' : isComplex(v) ? compactJson(v) : String(v) };
  }
  function cancelEdit(): void {
    editing = null;
  }

  /** SQL-quote a scalar value typed into the cell editor: numbers bare (when
   * the previous value was numeric), empty → NULL, else 'escaped'. */
  function sqlLiteral(raw: string, asNumber: boolean): string {
    if (raw === '') return 'NULL';
    if (asNumber && /^-?\d+(\.\d+)?$/.test(raw)) return raw;
    return `'${raw.replace(/'/g, "''")}'`;
  }
  /** SQL-quote an existing typed value (for WHERE / INSERT values). */
  function valueLiteral(v: unknown): string {
    if (v === null || v === undefined) return 'NULL';
    if (typeof v === 'number' || typeof v === 'bigint') return String(v);
    if (typeof v === 'boolean') return v ? '1' : '0';
    if (isComplex(v)) return `'${compactJson(v).replace(/'/g, "''")}'`;
    return `'${String(v).replace(/'/g, "''")}'`;
  }
  /** `\`db\`.\`table\`` (db optional). */
  function tableRef(): string {
    return editDb ? `\`${editDb}\`.\`${editTable}\`` : `\`${editTable}\``;
  }

  /** Build the UPDATE for the in-progress cell edit and open the review modal.
   * ClickHouse uses `ALTER TABLE … UPDATE` (a mutation); other SQL engines use
   * a plain `UPDATE`. */
  function commitEdit(): void {
    if (!editing || !result || !editTable || editPkCols.length === 0) {
      editing = null;
      return;
    }
    const { rowIdx, colIdx, value } = editing;
    const colName = result.columns[colIdx].name;
    const prev = liveRows[rowIdx][colIdx];
    const prevStr = prev === null || prev === undefined ? '' : isComplex(prev) ? compactJson(prev) : String(prev);
    if (value === prevStr) {
      editing = null; // no change → nothing to review
      return;
    }
    // Mongo: build an updateOne targeting `_id` and open the review modal.
    if (engine === 'mongodb') {
      const cmd = `db.${editTable}.updateOne(${mongoIdFilter(rowIdx)}, {"$set": {${JSON.stringify(colName)}: ${mongoLiteral(value, prev)}}})`;
      editing = null;
      openReview('Review updateOne', cmd);
      return;
    }
    const asNumber = typeof prev === 'number';
    const setExpr = `\`${colName}\` = ${sqlLiteral(value, asNumber)}`;
    const where = whereByPk(rowIdx);
    const sql =
      engine === 'clickhouse'
        ? `ALTER TABLE ${tableRef()} UPDATE ${setExpr} WHERE ${where};`
        : `UPDATE ${tableRef()} SET ${setExpr} WHERE ${where};`;
    editing = null;
    openReview(engine === 'clickhouse' ? 'Review ALTER … UPDATE (mutation)' : 'Review UPDATE', sql);
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
      cols.push(`\`${c.name}\``);
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
      // Refresh from the DB by re-running the active tab's query.
      await database.runQuery();
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
  // after a delete runs).
  $effect(() => {
    void result;
    selected = new Set();
    lastClickedIdx = null;
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
      where = `\`${pk}\` IN (${list})`;
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

  // ── Export / copy (reflect the current filtered + sorted view) ───────────────
  function exportText(v: unknown): string {
    if (v === null || v === undefined) return '';
    if (isComplex(v)) return compactJson(v);
    return String(v);
  }
  function exportRows(): unknown[][] {
    return filtering || sorting ? viewRows.map((r) => r.row) : liveRows;
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
    const s = exportText(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  }
  function toCsv(): string {
    if (!result) return '';
    const header = result.columns.map((c) => csvCell(c.name)).join(',');
    const body = exportRows().map((r) => r.map(csvCell).join(',')).join('\n');
    return `${header}\n${body}`;
  }
  function toJson(): string {
    if (!result) return '[]';
    const names = result.columns.map((c) => c.name);
    const objs = exportRows().map((r) => Object.fromEntries(names.map((n, i) => [n, r[i] ?? null])));
    return JSON.stringify(objs, null, 2);
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  const exportScope = $derived(
    filtering && sorting
      ? ' (filtered + sorted view)'
      : filtering
        ? ' (filtered rows only)'
        : sorting
          ? ' (sorted view)'
          : '',
  );

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

  // Server-side full export: runs the statement again without a row cap.
  // The server returns the file with `Content-Disposition: attachment` so the
  // browser auto-downloads. We proxy it through `api.post` to carry the auth
  // header, then trigger a download from the returned text.
  let exporting = $state(false);

  async function exportFullCsv(): Promise<void> {
    if (!connectionId || !statement || exporting) return;
    exporting = true;
    try {
      const text = await api.post<string>(
        `/connections/${connectionId}/db/export`,
        { statement, format: 'csv', node: database.activeDb ?? undefined },
      );
      downloadText(typeof text === 'string' ? text : JSON.stringify(text), 'export.csv', 'text/csv');
    } catch (e) {
      toasts.error('Export failed', e instanceof Error ? e.message : String(e));
    } finally {
      exporting = false;
    }
  }

  // Paste the query + result rows into the running agent's input (bracketed
  // paste, not auto-submitted) so it can act on the real DB state.
  function sendToRunningAgent(): void {
    const agentId = ws.targetAgentId;
    if (!agentId) {
      toasts.error('No running agent', 'Open a claude/codex session in this workspace first');
      return;
    }
    if (!result) return;
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
    const text =
      `Here is the current database state from "${connName}":\n\n` +
      (statement ? `Query:\n${statement}\n\n` : '') +
      `Result (${viewRows.length} row${viewRows.length === 1 ? '' : 's'}${more}):\n` +
      `${JSON.stringify(rowsObj, null, 2)}\n`;
    ws.injectInput(agentId, text);
    toasts.success('Sent to running agent', 'Pasted into the agent — press Enter to send');
  }

  // Autofocus + select the inline editor input on open. Svelte actions can't be
  // async, so defer the focus/select to a microtask after mount.
  function focusEditor(node: HTMLInputElement): void {
    void tick().then(() => {
      node.focus();
      node.select();
    });
  }
</script>

{#if error}
  <div class="grid-error mono">
    <Icon name="x" size={14} />
    <span>{error}</span>
  </div>
{:else if !result}
  {#if !mini}
    <div class="grid-empty">
      <Icon name="grid" size={mini ? 16 : 22} />
      <span>Run a query to see results.</span>
    </div>
  {/if}
{:else if result.columns.length === 0}
  <div class="grid-empty">
    <Icon name="check" size={mini ? 16 : 22} />
    <span>
      {result.message ??
        (result.rows_affected != null ? `${result.rows_affected} row(s) affected` : 'Statement OK')}
    </span>
  </div>
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
          <button class="vs" class:on={viewMode === 'json'} role="tab" aria-selected={viewMode === 'json'} onclick={() => (viewMode = 'json')} title="JSON array">JSON</button>
        </div>
        {#if viewMode === 'grid'}
          <button
            class="tb-btn"
            class:on={expandJson}
            onclick={() => (expandJson = !expandJson)}
            title="Expand all nested JSON cells inline (instead of clicking each)"
          ><Icon name={expandJson ? 'minimize' : 'maximize'} size={11} />{expandJson ? 'Collapse' : 'Expand'} JSON</button>
        {/if}
        <button class="tb-btn" onclick={sendToRunningAgent} title="Paste this query + result into your running agent (so it sees the real DB state)"><Icon name="comment" size={11} />→ Agent</button>
        <button class="tb-btn" onclick={copyTsv} title="Copy as TSV{exportScope}"><Icon name="file" size={11} />Copy</button>
        <button class="tb-btn" onclick={exportCsv} title="Export CSV{exportScope}"><Icon name="arrowDown" size={11} />CSV</button>
        <button class="tb-btn" onclick={exportJson} title="Export JSON{exportScope}"><Icon name="arrowDown" size={11} />JSON</button>
        {#if connectionId && statement && result?.truncated}
          <button
            class="tb-btn"
            disabled={exporting}
            onclick={() => void exportFullCsv()}
            title="Server-side full export — re-runs the query without the row cap"
          ><Icon name={exporting ? 'refresh' : 'fetch'} size={11} />{exporting ? 'Exporting…' : 'Full Export'}</button>
        {/if}
      </div>
    {/if}

    {#if !mini && selected.size > 0}
      <div class="sel-bar">
        <span class="sel-count">{selected.size} selected</span>
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
        <span class="fb-hint">filters update the query — press Run to apply</span>
      </div>
    {/if}

    {#if viewMode === 'json'}
      <div class="alt-view">
        {#if viewTruncated}<div class="alt-note dim">Showing first {VIEW_CAP} of {viewRows.length} rows.</div>{/if}
        <pre class="alt-json mono">{JSON.stringify(objRows, null, 2)}</pre>
      </div>
    {:else if viewMode === 'vertical'}
      <div class="alt-view">
        {#if viewTruncated}<div class="alt-note dim">Showing first {VIEW_CAP} of {viewRows.length} rows.</div>{/if}
        {#each objRows as obj, ri (ri)}
          <div class="vrec">
            <div class="vrec-head mono">#{ri + 1}</div>
            {#each result.columns as c (c.name)}
              <div class="vrow">
                <span class="vk mono">{c.name}</span>
                <span class="vv mono">{obj[c.name] === null || obj[c.name] === undefined ? '∅' : isComplex(obj[c.name]) ? compactJson(obj[c.name]) : String(obj[c.name])}</span>
              </div>
            {/each}
          </div>
        {/each}
      </div>
    {:else}
    <div class="grid-scroll" bind:this={scrollEl} bind:clientHeight={viewportH} onscroll={onScroll}>
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
          {#each windowRows as { row, idx } (idx)}
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
                {:else if v === null || v === undefined}
                  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                  <td
                    class="cell null"
                    class:editable={isEditableCell(ci)}
                    title="NULL"
                    style="width:{w}ch; max-width:{w}ch;"
                    ondblclick={() => beginEdit(idx, ci)}
                    oncontextmenu={(e) => cellMenu(e, ci, v, idx)}
                  ><span class="null-glyph">∅</span></td>
                {:else if isComplex(v)}
                  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                  <td
                    class="cell json"
                    class:wrap={expandJson}
                    title="Click to expand"
                    style="width:{w}ch; max-width:{w}ch;"
                    onclick={() => openCell(v)}
                    oncontextmenu={(e) => cellMenu(e, ci, v, idx)}
                  >{expandJson ? prettyJson(v) : compactJson(v)}<button class="cell-expand" title="Expand value" aria-label="Expand value" onclick={(e) => { e.stopPropagation(); openCell(v); }}><Icon name="maximize" size={9} /></button></td>
                {:else}
                  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                  <td
                    class="cell"
                    class:editable={isEditableCell(ci)}
                    style="width:{w}ch; max-width:{w}ch;"
                    ondblclick={() => beginEdit(idx, ci)}
                    oncontextmenu={(e) => cellMenu(e, ci, v, idx)}
                  >{#if filtering}{#each highlightParts(cellText(v)) as part}{#if part.hit}<mark>{part.t}</mark>{:else}{part.t}{/if}{/each}{:else}{cellText(v)}{/if}<button class="cell-expand" title="Expand value" aria-label="Expand value" onclick={(e) => { e.stopPropagation(); openCell(v); }}><Icon name="maximize" size={9} /></button></td>
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
    {#if !mini}
      <div class="grid-foot">
        {#if filtering}
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
    <div class="cell-viewer" role="dialog" aria-modal="true" aria-label="Cell value">
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
        <button class="tb-btn" onclick={copyViewer} title="Copy full value"><Icon name="file" size={11} />Copy</button>
        <button class="icon-btn" onclick={() => (viewer = null)} aria-label="Close">✕</button>
      </div>
      <pre class="cv-body mono">{viewerText}</pre>
    </div>
  </div>
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
    <div class="review-modal" role="dialog" aria-modal="true" aria-label={reviewSql.title}>
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

<style>
  .grid-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
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
    align-items: center;
    gap: 6px;
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
    margin-left: auto;
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
    border-right: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
  }
  .vs:last-child {
    border-right: none;
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
  .alt-json {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    color: var(--text);
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
  .grid-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
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
    text-align: left;
    padding: 5px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    border-right: 1px solid var(--border);
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
    text-align: left;
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
    border-right: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
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
    text-align: right;
    font-size: 10.5px;
    position: sticky;
    left: 0;
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
    right: 0;
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
    text-align: left;
    padding-left: 5px;
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
    right: 1px;
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
