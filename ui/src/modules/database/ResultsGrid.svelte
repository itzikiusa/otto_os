<script lang="ts">
  // Result view orchestrator: sticky-header grid (GridView), a JSON array view
  // (JsonView) or a vertical row-per-record layout (VerticalView) over the same
  // rows, plus everything around them — result-set switcher, running overlay,
  // toolbar search · view switch · Copy (TSV) · Export CSV/JSON · streaming
  // export, quick-filter chips, selection bar, pending-edits bar and the footer
  // (rows · ms · pager). Client-side filter + sort live here and feed the views;
  // the edit flow (cell drafts, selection, viewer, doc editor, review modal) is
  // one `EditFlow` instance shared with the views through their `flow` prop.
  // When the result comes from a simple single-table SELECT with a known
  // primary key, cells become double-click editable (issues an UPDATE via the
  // connection's query API after a review).
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import {
    database,
    effectiveViewMode,
    viewModeReason,
    type ViewMode,
  } from '../../lib/stores/database.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { buildFilteredQuery, type FilterMode } from './query-filter';
  import { databaseAccessChild } from '../../lib/access-options';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import type { QueryResult, DbForeignKey } from '../../lib/api/types';
  import ContextPacketDialog from '../../lib/components/ContextPacketDialog.svelte';
  import ErrorPanel from './ErrorPanel.svelte';
  import GridView from './GridView.svelte';
  import VerticalView from './VerticalView.svelte';
  import JsonView from './JsonView.svelte';
  import CellViewer from './CellViewer.svelte';
  import DocEditor from './DocEditor.svelte';
  import ReviewModal from './ReviewModal.svelte';
  import ExportDialog from './ExportDialog.svelte';
  import { EditFlow, SET_EMPTY, SET_NULL } from './EditFlow.svelte';
  import { qid, valueLiteral } from './edit-sql';
  import { ALT_BATCH, cellStr, copyText, fmtBytes, isComplex } from './results-format';

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
    /**
     * The result view to render, resolved by the owner (QueryEditor →
     * `effectiveViewMode`: tab pick → column threshold → connection memory →
     * engine default). Without it the component keeps a local grid/vertical/json
     * state — dashboard mini widgets and the Athena view have no query tab.
     */
    viewMode?: ViewMode;
    /** Why `viewMode` is what it is (the view switch's tooltip). */
    viewReason?: string;
    /** The tab's explicit pick, if any — shows the "Auto" chip that clears it. */
    tabPick?: ViewMode | null;
    /**
     * User picked a view (`null` = back to automatic). The store write happens in
     * the owner — this component is also mounted where there is no tab to write.
     */
    onviewmode?: (m: ViewMode | null) => void;
  }
  let {
    result: resultProp,
    error = null,
    mini = false,
    statement,
    connectionId,
    running = false,
    offset = 0,
    viewMode,
    viewReason,
    tabPick = null,
    onviewmode,
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
  // Column-name signature of the shown result. GridView takes it as its
  // `resetToken` (widths + scroll reset only when it changes).
  const colKey = $derived((result?.columns ?? []).map((c) => c.name).join(''));
  // Signature of the last rendered result (non-reactive — used only to decide
  // whether the view state should reset).
  let prevColKey: string | null = null;
  $effect(() => {
    // Rows always re-seed (edits re-query, not patch in place).
    liveRows = result ? (mini ? result.rows.slice(0, MINI_MAX) : result.rows) : [];
    // Preserve sort / search (and, in GridView, column widths / scroll) when the
    // new result has the SAME columns (a re-run of the same query), so the grid
    // doesn't jump; reset them only when the shape actually changes.
    if (colKey !== prevColKey) {
      search = '';
      sortCol = null;
      sortDir = null;
      // WP2: expansion
      prevColKey = colKey;
    }
    // Clear the selection whenever the upstream result changes (incl. the
    // re-query after a delete runs). Pending cell drafts are keyed by liveRows
    // index, so a result change invalidates them too — cleared together.
    flow.resetForResult();
  });

  // Engine behind this result (drives dialect for inline edits).
  const engine = $derived(database.capabilities?.engine ?? null);

  // "Expand JSON" mode pretty-prints complex grid cells inline (GridView grows
  // its rows to a fixed taller height so the virtualization math stays exact).
  let expandJson = $state(false);

  // Result view mode: columnar grid, one JSON object per row, or a vertical
  // row-per-record layout (like Postgres `\x` / ClickHouse FORMAT Vertical).
  // Controlled by the `viewMode` prop when the owner has a query tab to keep it
  // on; otherwise (mini widgets, Athena) a local pick that starts at Grid.
  let localMode = $state<ViewMode>('grid');
  const mode = $derived<ViewMode>(viewMode ?? localMode);
  function pickView(m: ViewMode | null): void {
    if (onviewmode) onviewmode(m);
    else localMode = m ?? 'grid';
  }
  // The "Auto" chip only makes sense where a pick can be cleared back to the
  // automatic resolution — i.e. a tab-owned grid with an explicit pick on it.
  const showAutoChip = $derived(!!onviewmode && tabPick !== null);
  // What clearing the pick would land on (same resolution minus the tab pick).
  const VIEW_LABEL: Record<ViewMode, string> = { grid: 'Grid', vertical: 'Vertical', json: 'JSON' };
  const autoTitle = $derived.by(() => {
    const a = {
      tabPick: null,
      connPick: database.connView,
      columnCount: result?.columns.length ?? 0,
      autoVerticalCols: ui.dbAutoVerticalCols,
      engine: database.capabilities?.engine ?? null,
    };
    return `Back to automatic: ${VIEW_LABEL[effectiveViewMode(a)]} (${viewModeReason(a)})`;
  });
  // Non-grid views aren't virtualized, and one document can be enormous on its
  // own (a `lobby_format_history` doc is ~88KB, so 100 rows ≈ 9MB). A flat 500-row
  // cap is therefore no protection at all — rendering is BATCHED instead: draw
  // ALT_BATCH records, grow on demand. VIEW_CAP stays the hard ceiling.
  const VIEW_CAP = 500;
  let altShown = $state(ALT_BATCH);
  // Collapse the window back whenever the result or the view mode changes —
  // otherwise a big window opened on one result silently applies to the next.
  $effect(() => {
    void result;
    void mode;
    altShown = ALT_BATCH;
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
    if (!result || mode === 'grid') return [];
    const names = uniqueColNames;
    return viewRows.slice(0, altCap).map(({ row, idx }) => {
      const o: Record<string, unknown> = {};
      names.forEach((n, i) => (o[n] = row[i]));
      return { obj: o, idx };
    });
  });
  /** Records still drawable below the current batch (excludes the hard-capped tail). */
  const altRemaining = $derived(
    mode === 'grid' ? 0 : Math.max(0, Math.min(viewRows.length, VIEW_CAP) - altCap),
  );
  const viewTruncated = $derived(mode !== 'grid' && viewRows.length > VIEW_CAP);

  // ── Edit flow ────────────────────────────────────────────────────────────────
  // Editability, cell drafts, selection, viewer / doc editor and the review
  // modal live on ONE EditFlow instance. Its inputs are pushed from here (a
  // `.svelte.ts` class must not create effects at construction); the views get
  // `flow` as a prop and read/call it directly.
  const accessChild = $derived(databaseAccessChild(connectionId === database.selectedConnId ? database.activeDb : undefined));
  const canModify = $derived(!!connectionId && resourceAccess.can('connection',connectionId,'db_data','database','edit',accessChild));
  const canExport = $derived(!!connectionId && resourceAccess.can('connection',connectionId,'db_export','database','view',accessChild));
  $effect(()=>{if(connectionId)void resourceAccess.load('connection',connectionId,accessChild);});

  const flow = new EditFlow();
  $effect(() => {
    flow.update({
      result,
      liveRows,
      statement,
      connectionId,
      engine,
      canModify,
      uniqueColNames,
      mini,
      viewOrder: viewRows.map((r) => r.idx),
      resultCount: resultSets.length,
    });
  });
  // Resolve the primary key whenever statement/connection/result changes. The
  // store fields are read here too (the old inline effect read them mid-body
  // and therefore tracked them) so the target re-resolves when the active DB or
  // the capabilities change — `resolveTarget` reads its inputs synchronously
  // before the first await, so they're tracked as well.
  $effect(() => {
    void statement;
    void connectionId;
    void result?.columns;
    void database.activeDb;
    void database.capabilities?.sql;
    void database.schemaRoot;
    void flow.resolveTarget();
  });

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
    return flow.editFks.find((fk) => fk.columns.includes(col)) ?? null;
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
      conds.push(`${qid(engine, refCol)} = ${valueLiteral(engine, v)}`);
    }
    if (conds.length === 0) return null;
    const ref = fk.ref_schema
      ? `${qid(engine, fk.ref_schema)}.${qid(engine, fk.ref_table)}`
      : qid(engine, fk.ref_table);
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
      { label: 'Expand value', icon: 'maximize', action: () => flow.openCell(v, rowIdx, ci) },
      { label: 'Copy value', icon: 'file', action: () => copyText(v === null || v === undefined ? '' : cellStr(v)) },
    );
    // Explicit NULL / '' — the typed editor can't express the difference (an
    // empty draft parks as NULL). Both park a pending change, review-gated.
    if (flow.isEditableCell(ci)) {
      items.push(
        { separator: true },
        { label: 'Set NULL', icon: 'x', disabled: v === null || v === undefined, action: () => flow.parkValue(rowIdx, ci, SET_NULL) },
        { label: 'Set empty string', icon: 'edit', action: () => flow.parkValue(rowIdx, ci, SET_EMPTY) },
      );
    }
    // Copy as INSERT — acts on the whole selection when this row is part of it,
    // otherwise on just this row (so a single row needs no checkbox first).
    if (flow.copyTarget) {
      const rows = flow.selected.has(rowIdx) ? flow.selectedIndices() : [rowIdx];
      items.push({
        label: rows.length > 1 ? `Copy ${rows.length} rows as INSERT` : 'Copy row as INSERT',
        icon: 'file',
        action: () => flow.copyRowsAsInsert(rows),
      });
    }
    // Delete actions — only for editable results (single table/collection with a
    // resolved key). Builds a statement and opens the review modal; never runs
    // immediately.
    if (flow.editable) {
      items.push({ separator: true });
      if (flow.selected.size > 0) {
        items.push({ label: `Delete selected (${flow.selected.size})…`, icon: 'trash', danger: true, action: () => flow.deleteSelected() });
      }
      if (!flow.selected.has(rowIdx)) {
        items.push({ label: 'Delete this row…', icon: 'trash', danger: true, action: () => flow.deleteRows([rowIdx]) });
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
    if (!canExport) return;
    download(toCsv(), 'result.csv', 'text/csv');
  }
  function exportJson(): void {
    if (!canExport) return;
    download(toJson(), 'result.json', 'application/json');
  }

  // ── Large-batch streaming export to a local file ─────────────────────────────
  // The dialog itself (format / folder / row limit / live progress) is
  // ExportDialog.svelte; it mounts fresh on every open.
  let showExportDialog = $state(false);

  function openExportDialog(): void {
    if (!canExport) return;
    showExportDialog = true;
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
        <!-- WP2/WP4: toolbar mounts -->
        {#if flow.editable}
          <span
            class="gt-edit-hint"
            title="Double-click a cell to edit (you review the SQL before it runs). Primary key {flow.editPkCols.length > 1 ? 'columns' : 'column'} ({flow.editPkCols.join(', ')}) {flow.editPkCols.length > 1 ? 'are' : 'is'} read-only."
          >
            <Icon name="edit" size={10} />double-click to edit
          </span>
        {/if}
        <div class="view-seg" title={viewReason}>
          <div class="view-tabs" role="tablist" aria-label="Result view">
            <button class="vs" class:on={mode === 'grid'} role="tab" aria-selected={mode === 'grid'} onclick={() => pickView('grid')} title="Columnar grid">Grid</button>
            <button class="vs" class:on={mode === 'vertical'} role="tab" aria-selected={mode === 'vertical'} onclick={() => pickView('vertical')} title="One record per block (field: value)">Vertical</button>
            <button class="vs" class:on={mode === 'json'} role="tab" aria-selected={mode === 'json'} onclick={() => pickView('json')} title="One JSON object per row">JSON</button>
          </div>
          {#if showAutoChip}
            <!-- Not a fourth tab: clears THIS tab's pick only (the connection memory stays). -->
            <button class="vs auto" onclick={() => pickView(null)} title={autoTitle}>Auto</button>
          {/if}
        </div>
        {#if mode === 'grid'}
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
        <button class="tb-btn" disabled={!canExport} onclick={exportCsv} title="Export CSV{exportScope}"><Icon name="arrowDown" size={11} />CSV</button>
        <button class="tb-btn" disabled={!canExport} onclick={exportJson} title="Export JSON{exportScope}"><Icon name="arrowDown" size={11} />JSON</button>
        {#if connectionId && statement}
          <button
            class="tb-btn"
            class:accent={result?.truncated}
            disabled={!canExport} onclick={openExportDialog}
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

    {#if !mini && flow.selected.size > 0}
      <div class="sel-bar">
        <span class="sel-count">{flow.selected.size} selected</span>
        {#if flow.copyTarget}
          <button
            class="sel-gen"
            onclick={() => flow.copySelectedAsInsert()}
            title={engine === 'mongodb'
              ? 'Open the selected rows as an insertMany(…) in a new tab (not run)'
              : 'Open the selected rows as INSERT statements in a new tab (not run)'}
          >
            <Icon name="file" size={11} />Copy as INSERT
          </button>
        {/if}
        {#if flow.editable}
          <button
            class="sel-gen"
            onclick={() => flow.copySelectedWhere()}
            title="Copy a `pk IN (…)` predicate for the selected rows to the clipboard"
          >
            <Icon name="file" size={11} />WHERE pk IN (…)
          </button>
        {/if}
        <button class="sel-del" onclick={() => flow.deleteSelected()} title="Delete selected rows (you review before it runs)">
          <Icon name="trash" size={11} />Delete…
        </button>
        <button class="sel-clear" onclick={() => flow.clearSelection()}>Clear</button>
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

    <!-- WP4: filter bar -->
    {#if mode === 'json'}
      <JsonView
        {result}
        {objRows}
        {uniqueColNames}
        {engine}
        {flow}
        {mini}
        {altRemaining}
        {viewTruncated}
        viewCap={VIEW_CAP}
        totalRows={viewRows.length}
        onshowmore={() => (altShown += ALT_BATCH)}
      />
    {:else if mode === 'vertical'}
      <VerticalView
        {result}
        {objRows}
        {uniqueColNames}
        {engine}
        {flow}
        {mini}
        {altRemaining}
        {viewTruncated}
        viewCap={VIEW_CAP}
        totalRows={viewRows.length}
        onshowmore={() => (altShown += ALT_BATCH)}
      />
    {:else}
      <GridView
        {result}
        {liveRows}
        {viewRows}
        {mini}
        {expandJson}
        {flow}
        {filtering}
        {searchLc}
        {sortCol}
        {sortDir}
        resetToken={colKey}
        oncellmenu={cellMenu}
        onheadermenu={headerMenu}
        oncyclesort={cycleSort}
      />
    {/if}
    {#if flow.pendingCells > 0}
      <div class="pending-bar" data-testid="pending-edits-bar">
        <Icon name="edit" size={12} />
        <span>
          <strong>{flow.pendingCells}</strong> pending change{flow.pendingCells === 1 ? '' : 's'} on
          <strong>{flow.pendingEdits.size}</strong> row{flow.pendingEdits.size === 1 ? '' : 's'} — nothing
          is written until you review &amp; run.
        </span>
        <span class="pending-spacer"></span>
        <button class="btn small ghost" onclick={() => flow.discardPending()}>Discard</button>
        <button class="btn small primary" onclick={() => flow.reviewPending()}>Review &amp; apply</button>
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
        {#if !flow.editable && statement}
          <span class="grow"></span>
          <span class="edit-note" title={flow.editReason ?? undefined}
            >{flow.editReason ?? 'Editing needs a single-table result with a primary key'}</span
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

{#if flow.viewer}
  <CellViewer {flow} />
{/if}

{#if flow.docEditor}
  <DocEditor {flow} />
{/if}

{#if showExportDialog && connectionId && statement}
  <ExportDialog {statement} {connectionId} {canExport} onclose={() => (showExportDialog = false)} />
{/if}

{#if flow.reviewSql}
  <ReviewModal
    title={flow.reviewSql.title}
    sql={flow.reviewSql.sql}
    diff={flow.reviewSql.diff}
    running={flow.runningReview}
    onsql={(s) => flow.setReviewSql(s)}
    onrun={() => void flow.runReview()}
    onclose={() => flow.closeReview()}
  />
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
  /* The tablist wraps only the three real views (a11y: the Auto chip is a
     button, not a tab); `contents` keeps the chips in one flex row. */
  .view-tabs {
    display: contents;
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
  /* "Auto" is an escape hatch, not a fourth view — dimmed and italic so it
     reads as "clear my pick" next to the three real modes. */
  .vs.auto {
    border-inline-start: 1px solid var(--border);
    font-style: italic;
    opacity: 0.7;
  }
  .vs.auto:hover {
    opacity: 1;
    color: var(--accent);
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
  .tb-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     The results toolbar (search + view-segment + Copy/CSV/JSON/Agent) is dense
     — let it wrap rather than run off the edge, and make sure the grid itself
     fills its bounded block (GridView handles the two-way touch scroll). */
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
    .gt-search-input {
      font-size: 13px;
    }
    .grid-empty,
    .grid-error {
      font-size: 13.5px;
    }
  }
</style>
