///<reference types="svelte" />
;
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  // The grid's dialogs load on first open. Module-level: ONE loader per
  // dialog, so a later open renders in the same frame, and a failed chunk
  // surfaces through LazyMount (toast + the open flag reset, so the next
  // click retries) instead of a silent, sticky no-op.
  const DocEditorLazy = lazyComponent(() => import('./DocEditor.svelte'));
  const RawDocViewerLazy = lazyComponent(() => import('./RawDocViewer.svelte'));
  const ExportDialogLazy = lazyComponent(() => import('./ExportDialog.svelte'));
  const RecordDiffLazy = lazyComponent(() => import('./RecordDiff.svelte'));
  const AggregateBuilderLazy = lazyComponent(() => import('./AggregateBuilder.svelte'));
  const ReviewModalLazy = lazyComponent(() => import('./ReviewModal.svelte'));
  const ContextPacketDialogLazy = lazyComponent(() => import('../../lib/components/ContextPacketDialog.svelte'));
;;

import LazyMount from '../../lib/components/LazyMount.svelte';
import { formatBytes } from '../../lib/metric-format';
import { pluralNoun } from '../../lib/plural';
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
import { onDestroy, untrack } from 'svelte';
import { stashGridState, takeGridState, type GridTabState } from '../../lib/database/grid-tab-state';
import Icon from '../../lib/components/Icon.svelte';
import { onTabKey } from '../../lib/tabKeys';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { isReleased, releasedRows, resultBudget } from '../../lib/stores/db-result-budget';
import { findInPage } from '../../lib/findinpage.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import {
    database,
    effectiveViewMode,
    viewModeReason,
    type ViewMode,
  } from '../../lib/stores/database.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { obviousWriteVerb } from '../../lib/database/sql-dialect';
import { buildFilteredQuery, type FilterMode } from './query-filter';
import { filterValMatches, type FilterVal } from '../../lib/database/filter-chips';
import { databaseAccessChild } from '../../lib/access-options';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import type { QueryResult, DbForeignKey } from '../../lib/api/types';
import ErrorPanel from './ErrorPanel.svelte';
import GridView from './GridView.svelte';
import VerticalView from './VerticalView.svelte';
import JsonView from './JsonView.svelte';
import CellViewer from './CellViewer.svelte';
// Modal-only children (DocEditor / RawDocViewer reach CodeEditor, the rest
// are dialogs) load on first open through the module-level `lazyComponent`s
// above, so the grid — and Home's DB widget that embeds it — doesn't carry
// them up front.
import MongoFilterBar from './MongoFilterBar.svelte';
import { EditFlow, SET_EMPTY, SET_NULL, type RowPatch } from './EditFlow.svelte';
import { qid, valueLiteral } from './edit-sql';
import { ALT_BATCH, cellStr, copyWithToast, isComplex } from './results-format';
import { newExpansionState } from '../../lib/jsonTree/expansion-plan';
import { cellMatchesFilter } from './grid-format';
import {
    colScanFor as cachedColScan,
    dropColScans,
    dropScan,
    memoFilter,
    memoSort,
    prebuild,
    scanFor,
  } from './grid-view-cache';
import RowDetail from './RowDetail.svelte';
import type { MenuItem } from '../../lib/contextmenu.svelte';
;

  interface Props {
    result: QueryResult | null;
    error?: string | null;
    /** Compact mode for dashboard widget mini-grids (no toolbar/footer). */
    mini?: boolean;
    /** Active statement — enables editability detection when set. */
    statement?: string;
    /** Connection id the result came from — required for inline editing. */
    connectionId?: string | null;
    /**
     * The scope node the result RAN with (the tab's `ran_node`; `null` = no
     * database selected at run time). Edits, "Copy as INSERT" and the
     * post-apply refresh target it — never whatever the selector shows now.
     * Omitted where there is no query tab (widgets, Athena).
     */
    ranNode?: string | null;
    /** True while the active tab's query is in flight — drives the running overlay. */
    running?: boolean;
    /**
     * When the in-flight run started (epoch ms, the tab's `pending.startedAt`).
     * The overlay counts from it, so switching tab / connection / view or
     * reloading doesn't restart the clock at 0s. Absent → counts from mount.
     */
    startedAt?: number | null;
    /**
     * Stable key of the query tab this grid currently shows (its uid). Sort,
     * search, column filters and un-applied edits are kept per tab under it
     * (grid-tab-state.ts) instead of leaking across tabs / being discarded.
     */
    tabKey?: string | null;
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
    /**
     * Cancel the in-flight query behind `running`. A DB Explorer tab (a
     * `connectionId` is set) defaults to `database.abortQuery()`; a host with no
     * tab (Athena) passes its own — without one the running card has no Cancel,
     * rather than a button that would stop some OTHER query in the explorer.
     */
    oncancel?: () => void;
  };function $$render() {
  const runningCard/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  
   { svelteHTML.createElement("div", { "class":`rg-overlay rg-inline`,});
     { svelteHTML.createElement("div", { "class":`rg-overlay-card`,});
       { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 16px`,"aria-hidden":`true`,}); }
       { svelteHTML.createElement("span", { "class":`rg-overlay-text`,}); elapsed;  }
      if(cancelQuery){
         { svelteHTML.createElement("button", {     "class":`rg-cancel`,"onclick":cancelQuery,"title":`Cancel the running query`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
      }
     }
   }
};return __sveltets_2_any(0)};  const loadingFrame/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  
   { svelteHTML.createElement("div", {   "class":`grid-wrap`,...__sveltets_2_empty({"data-state":`loading`}),});
     { svelteHTML.createElement("div", {   "class":`grid-toolbar`,"aria-hidden":`true`,});
       { svelteHTML.createElement("div", { "class":`gt-search skel-box`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});} }
     }
     { svelteHTML.createElement("div", { "class":`grid-body`,});
       { svelteHTML.createElement("div", {       "class":`grid-skeleton`,...__sveltets_2_empty({"data-grid-frame":true}),"role":`status`,"aria-live":`polite`,"aria-label":`Running query`,});
         { svelteHTML.createElement("div", { "class":`skel-head`,}); }
            for(let _ of __sveltets_2_ensureArray({ length: 8 })){let i = 1;i;
           { svelteHTML.createElement("div", { "class":`skel-row`,}); { svelteHTML.createElement("span", {   "class":`skel-bar`,"style":`width:${38 + ((i * 17) % 40)}%`,}); } }
        }
       }
     }
     { svelteHTML.createElement("div", { "class":`grid-foot`,});
       { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 11px`,"aria-hidden":`true`,}); }
       { svelteHTML.createElement("span", { "class":`rg-overlay-text`,}); elapsed;  }
      if(cancelQuery){
         { svelteHTML.createElement("button", {     "class":`pg-btn`,"onclick":cancelQuery,"title":`Cancel the running query`,});  }
      }
     }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Menu actions capture cell values. Destroying this result (source switch,
  // closure or access revocation) must release them without closing a newer
  // menu opened elsewhere in the app.
  let menuSeq: number | null = null;
  function showMenu(event: MouseEvent | KeyboardEvent, items: MenuItem[]): void {
    ctxMenu.show(event, items);
    menuSeq = ctxMenu.seq;
  }
  onDestroy(() => {
    if (ctxMenu.open && ctxMenu.seq === menuSeq) ctxMenu.close();
  });

  // ── Send-to-agent dialog (B2a) ────────────────────────────────────────────────
  let sendToAgentOpen = $state(false);
  let sendToAgentPayload = $state<unknown>(null);
  // ── WP4: aggregate pipeline builder + compare-two-records ───────────────────
  let pipelineOpen = $state(false);
  // AggregateBuilder stays mounted once opened (it keeps its draft pipeline
  // across close/reopen) but isn't loaded until the first open.
  let pipelineMounted = $state(false);
  $effect(() => {
    if (pipelineOpen) pipelineMounted = true;
  });
  /** liveRows indices of the two records being compared (null = closed). */
  let compare = $state<[number, number] | null>(null);
  /** A record exactly as the Vertical/JSON views build it (uniqueColNames-keyed). */
  function objRowAt(idx: number): Record<string, unknown> {
    const o: Record<string, unknown> = {};
    uniqueColNames.forEach((n, i) => (o[n] = liveRows[idx]?.[i]));
    return o;
  }

  let {
    result: resultProp,
    error = null,
    mini = false,
    statement,
    connectionId,
    ranNode,
    running = false,
    startedAt = null,
    tabKey = null,
    offset = 0,
    viewMode,
    viewReason,
    tabPick = null,
    onviewmode,
    oncancel,
  }: Props = $props();

  // A result from a DB Explorer query tab (vs a dashboard mini widget or the
  // Athena view). Everything that reads or writes the explorer's store — its
  // engine, quick-filter chips, pager, statement splicing, the DB Assistant —
  // is gated on this, so another page's grid can't show the explorer's filter
  // chips, label an Athena error "MYSQL", or rewrite the explorer's editor.
  const hosted = $derived(!!connectionId);
  /** The DB Assistant spawns an agent — the editor's Ask AI buttons need this too. */
  const canAssist = $derived(auth.can('agents', 'edit'));
  const cancelQuery = $derived.by<(() => void) | null>(() => {
    const cancel = oncancel ?? (hosted ? () => void database.abortQuery() : null);
    return cancel ? () => { cancelAsked = true; cancel(); } : null;
  });

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
        ? `${result.rows_affected.toLocaleString()} ${pluralNoun(result.rows_affected, 'row')} affected`
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
    const start = startedAt ?? Date.now();
    const tick = (): void => {
      elapsed = Math.max(0, Math.floor((Date.now() - start) / 1000));
    };
    // Immediately: no "0s" flash for a run that has been going for minutes.
    tick();
    // The label shows whole seconds: tick once per second, aligned to the next
    // second boundary of the run (not 4×/s).
    let iv: ReturnType<typeof setInterval> | undefined;
    const align = setTimeout(() => {
      tick();
      iv = setInterval(tick, 1000);
    }, 1000 - ((Date.now() - start) % 1000) + 5);
    return () => {
      clearTimeout(align);
      if (iv !== undefined) clearInterval(iv);
    };
  });

  // ── Run announcements (screen readers) ───────────────────────────────────────
  // The ticking "Running… Ns" text is visual only: a live region holding it was
  // re-read every second for the whole run. One always-mounted status says the
  // EDGES instead — "Running query" once, then finished / failed / canceled.
  let runAnnounce = $state('');
  let wasRunning = false;
  let cancelAsked = false;
  $effect(() => {
    const now = running;
    untrack(() => {
      if (now && !wasRunning) {
        cancelAsked = false;
        runAnnounce = 'Running query';
      } else if (!now && wasRunning) {
        runAnnounce = cancelAsked
          ? 'Query canceled'
          : error
            ? 'Query failed'
            : elapsed >= 1 ? `Query finished in ${elapsed} s` : 'Query finished';
      }
      wasRunning = now;
    });
  });

  // ── Footer pager (single auto-limited result only) ───────────────────────────
  // `auto_limited` (the server's applied LIMIT) lives on the top-level result and
  // is present only for a single paginatable SELECT / Mongo find — never batches.
  const pageSize = $derived(resultSets.length === 1 ? (resultProp?.auto_limited ?? 0) : 0);
  const showPager = $derived(!mini && hosted && pageSize > 0 && !!result);
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
  // `$state.raw`: only ever REASSIGNED. A deep proxy made every filter / sort /
  // search pass over 100k rows walk proxies (the first search key 0.7–0.9 s).
  let liveRows = $state.raw<unknown[][]>([]);
  // Column-name signature of the shown result. GridView takes it as its
  // `resetToken` (widths + scroll reset only when it changes).
  const colKey = $derived((result?.columns ?? []).map((c) => c.name).join('\u0001'));
  // Signature of the last rendered result (non-reactive — used only to decide
  // whether the view state should reset).
  let prevColKey: string | null = null;
  // The tab whose view state is live in this instance (non-reactive).
  let prevTabKey: string | null = null;
  // The result the live view state was built on (non-reactive): on a tab
  // switch `resultProp` is already the INCOMING tab's when the effect runs.
  let shownResult: unknown = null;
  /** The live view state, to park under the tab it belongs to. */
  function currentGridState(): GridTabState<RowPatch> {
    return {
      result: shownResult,
      colKey: prevColKey ?? '',
      search: searchInput,
      sortCol,
      sortDir,
      colFilters: colFiltersInput,
      pending: flow.pending,
    };
  }
  // Park the shown tab's state when the grid goes away (view switch, page
  // navigation) so coming back restores it.
  onDestroy(() => {
    if (prevTabKey) stashGridState(prevTabKey, untrack(currentGridState));
  });
  $effect(() => {
    // Rows always re-seed (edits re-query, not patch in place).
    liveRows = result ? (mini ? result.rows.slice(0, MINI_MAX) : result.rows) : [];
    // A different query tab: park the outgoing tab's sort/search/filters and
    // un-applied edits, and bring back the incoming tab's (when its result is
    // still the one they were made on) — never carry one tab's onto another.
    const key = tabKey ?? null;
    // Viewing a tab keeps its result resident (least-recently-VIEWED goes first).
    if (key) resultBudget.touch(key);
    if (key !== prevTabKey) {
      if (prevTabKey) stashGridState(prevTabKey, untrack(currentGridState));
      prevTabKey = key;
      const saved = key ? takeGridState<RowPatch>(key) : null;
      if (saved && saved.colKey === colKey) {
        setSearch(saved.search);
        sortCol = saved.sortCol;
        sortDir = saved.sortDir;
        setColFilters(saved.colFilters);
        detailIdx = null;
        expansion = newExpansionState();
        prevColKey = colKey;
        flow.resetForResult();
        if (saved.result === resultProp && saved.pending.size > 0) flow.pending = saved.pending;
        compare = null;
        shownResult = resultProp;
        return;
      }
      // No parked state for this tab: start it clean (the shape check below
      // would otherwise keep the previous tab's sort when columns match).
      prevColKey = null;
    }
    // Preserve sort / search (and, in GridView, column widths / scroll) when the
    // new result has the SAME columns (a re-run of the same query), so the grid
    // doesn't jump; reset them only when the shape actually changes.
    if (colKey !== prevColKey) {
      setSearch('');
      sortCol = null;
      sortDir = null;
      setColFilters({});
      detailIdx = null;
      // A different result shape invalidates every per-path toggle.
      expansion = newExpansionState();
      prevColKey = colKey;
    }
    // Clear the selection whenever the upstream result changes (incl. the
    // re-query after a delete runs). Pending cell drafts are keyed by liveRows
    // index, so a result change invalidates them too — cleared together.
    flow.resetForResult();
    compare = null;
    shownResult = resultProp;
  });

  // Tell the store how many un-applied edits the active tab holds (a tab
  // close asks before discarding them). Explorer grids only.
  $effect(() => {
    if (!hosted || mini) return;
    database.livePendingEdits = flow.pendingCells;
    return () => {
      database.livePendingEdits = 0;
    };
  });

  // Engine behind this result (drives dialect for inline edits).
  const engine = $derived(hosted ? (database.capabilities?.engine ?? null) : null);

  // "Expand JSON" mode pretty-prints complex grid cells inline (GridView grows
  // its rows to a fixed taller height so the virtualization math stays exact).
  let expandJson = $state(false);
  // ── WP2: nested-document expansion (Vertical + JSON views) ──────────────────
  // Node budget + sticky per-path toggles + Expand/Collapse all, shared by both
  // alt views so a branch opened in one is open in the other. The views mutate
  // it through `expansion-plan.ts`'s helpers (`setOverride`/`setExpansionMode`),
  // which bump its `version` — that is what their derived plans key on.
  let expansion = $state(newExpansionState());

  // Result view mode: columnar grid, one JSON object per row, or a vertical
  // row-per-record layout (like Postgres `\x` / ClickHouse FORMAT Vertical).
  // Controlled by the `viewMode` prop when the owner has a query tab to keep it
  // on; otherwise (mini widgets, Athena) a local pick that starts at Grid.
  let localMode = $state<ViewMode>('grid');
  // The owner resolves the view against the tab's FIRST result set, but a
  // multi-statement batch shows one set at a time (the switcher above), each with
  // its own width. Without a tab pick, re-resolve against the SHOWN set so the
  // Settings → Appearance "Auto-Vertical by engine" threshold applies to Result 2
  // as it does to Result 1 — and the tooltip says why for the set on screen.
  const autoInputs = $derived({
    tabPick: null,
    connPick: database.connView,
    columnCount: result?.columns.length ?? 0,
    autoVerticalCols: ui.dbAutoVerticalFor(database.capabilities?.engine),
    engine: database.capabilities?.engine ?? null,
  });
  const ownerAuto = $derived(!!onviewmode && viewMode !== undefined && tabPick === null);
  const mode = $derived<ViewMode>(
    ownerAuto ? effectiveViewMode(autoInputs) : (viewMode ?? localMode),
  );
  const modeReason = $derived(ownerAuto ? viewModeReason(autoInputs) : viewReason);
  function pickView(m: ViewMode | null): void {
    if (onviewmode) onviewmode(m);
    else localMode = m ?? 'grid';
  }
  // The "Auto" chip only makes sense where a pick can be cleared back to the
  // automatic resolution — i.e. a tab-owned grid with an explicit pick on it.
  const showAutoChip = $derived(!!onviewmode && tabPick !== null);
  // What clearing the pick would land on (same resolution minus the tab pick).
  const VIEW_LABEL: Record<ViewMode, string> = { grid: 'Grid', vertical: 'Vertical', json: 'JSON' };
  const autoTitle = $derived(
    `Back to automatic: ${VIEW_LABEL[effectiveViewMode(autoInputs)]} (${viewModeReason(autoInputs)})`,
  );
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
  // `searchInput` is what the box shows; `search` is what filters. Above
  // SEARCH_DEBOUNCE_ROWS loaded rows the filter pass (every row, plus the
  // highlight of every visible cell) runs once typing pauses, not per key —
  // the box itself stays instant.
  let searchInput = $state('');
  let search = $state('');
  const SEARCH_DEBOUNCE_ROWS = 20_000;
  const SEARCH_DEBOUNCE_MS = 120;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  function setSearch(value: string, debounce = false): void {
    searchInput = value;
    if (searchTimer !== null) clearTimeout(searchTimer);
    searchTimer = null;
    if (!debounce || value === '' || liveRows.length <= SEARCH_DEBOUNCE_ROWS) {
      search = value;
      return;
    }
    searchTimer = setTimeout(() => {
      searchTimer = null;
      search = value;
    }, SEARCH_DEBOUNCE_MS);
  }
  $effect(() => () => {
    if (searchTimer !== null) clearTimeout(searchTimer);
  });
  const searchLc = $derived(search.trim().toLowerCase());
  const filtering = $derived(searchLc.length > 0);

  // Per-row scan text, built ONCE per result rather than per keystroke (the
  // old code re-serialized every cell on every character typed — megabytes of
  // `JSON.stringify` per key on fat Mongo documents). It lives in
  // grid-view-cache.ts, keyed by the rows array: it survives a tab switch while
  // the tab's search is active, is pre-built in idle slices once the box is
  // focused on a big result (so the first key finds it done), and is freed
  // when the search is cleared or the box loses focus empty.
  /** Big results pre-build their search / column text while the user aims at
   *  the box (smaller ones build in one go on the first key, in < 20 ms). */
  let cancelPrebuild: (() => void) | null = null;
  function startPrebuild(target: 'search' | number): void {
    cancelPrebuild?.();
    cancelPrebuild = liveRows.length > SEARCH_DEBOUNCE_ROWS ? prebuild(liveRows, target, cellStr) : null;
  }
  function stopPrebuild(): void {
    cancelPrebuild?.();
    cancelPrebuild = null;
  }
  $effect(() => () => stopPrebuild());
  function onSearchFocus(): void {
    if (!filtering) startPrebuild('search');
  }
  function onSearchBlur(): void {
    stopPrebuild();
    if (!filtering && searchInput === '') dropScan(liveRows);
  }
  // A cleared search frees its text (a re-seed with no search does too).
  $effect(() => {
    const rows = liveRows;
    if (!filtering) untrack(() => dropScan(rows));
  });

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
    (hosted ? database.filters : []).filter((c) => c.kind === 'col' && c.values.length > 0),
  );
  // Typed match (a boolean chip compares by value — see filterValMatches).
  const cellMatchesVal = (cell: unknown, val: FilterVal): boolean => filterValMatches(cell, val, cellStr);
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

  // ── Header filter row (per column, client-side) ──────────────────────────────
  // Toggled from the toolbar; each column gets a "contains" box (plus =exact,
  // >n / <n, NULL, !NULL). Keyed by ORIGINAL column index; cleared when the
  // result's shape changes.
  let filterRow = $state(false);
  // `colFiltersInput` is what the boxes show; `colFilters` is what filters —
  // the same split (and the same SEARCH_DEBOUNCE_ROWS rule) as the toolbar
  // search, so typing into a header box over 100k rows doesn't run a full
  // filter pass per key.
  let colFiltersInput = $state.raw<Record<number, string>>({});
  let colFilters = $state.raw<Record<number, string>>({});
  let colFilterTimer: ReturnType<typeof setTimeout> | null = null;
  /** Replace BOTH maps at once (reset / tab restore) — never debounced. */
  function setColFilters(next: Record<number, string>): void {
    if (colFilterTimer !== null) clearTimeout(colFilterTimer);
    colFilterTimer = null;
    colFiltersInput = next;
    colFilters = next;
  }
  function setColFilter(ci: number, text: string): void {
    colFiltersInput = { ...colFiltersInput, [ci]: text };
    if (colFilterTimer !== null) clearTimeout(colFilterTimer);
    colFilterTimer = null;
    // Clearing a box (or a small result) applies at once.
    if (text.trim() === '' || liveRows.length <= SEARCH_DEBOUNCE_ROWS) {
      colFilters = colFiltersInput;
      return;
    }
    colFilterTimer = setTimeout(() => {
      colFilterTimer = null;
      colFilters = colFiltersInput;
    }, SEARCH_DEBOUNCE_MS);
  }
  $effect(() => () => {
    if (colFilterTimer !== null) clearTimeout(colFilterTimer);
  });
  const activeColFilters = $derived(
    Object.entries(colFilters)
      .filter(([, t]) => t.trim() !== '')
      .map(([ci, t]) => [Number(ci), t] as const),
  );
  // Per-column display text, built on a column's FIRST filtered key (or in
  // idle slices once its box is focused on a big result) and reused for every
  // following key and every other box until the rows change — for a JSON
  // column `cellStr` is a full `JSON.stringify` per document. Cached per rows
  // array in grid-view-cache.ts; columns whose filter is cleared are freed.
  function colScanFor(ci: number): (string | null)[] {
    return cachedColScan(liveRows, ci, cellStr);
  }
  /** The filter box that has focus (its column's text is being pre-built). */
  let focusedFilterCol: number | null = null;
  function onColFilterFocus(ci: number | null): void {
    stopPrebuild();
    focusedFilterCol = ci;
    if (ci !== null && !activeColFilters.some(([c]) => c === ci)) startPrebuild(ci);
    else if (ci === null) freeColScans(liveRows);
  }
  function freeColScans(rows: unknown[][]): void {
    const keep = new Set(activeColFilters.map(([c]) => c));
    if (focusedFilterCol !== null) keep.add(focusedFilterCol);
    dropColScans(rows, keep);
  }
  $effect(() => {
    const rows = liveRows;
    void activeColFilters;
    untrack(() => freeColScans(rows));
  });
  function colFilterMatches(idx: number): boolean {
    for (const [ci, text] of activeColFilters) {
      const s = colScanFor(ci)[idx];
      const isNull = s === null || s === undefined;
      if (!cellMatchesFilter(isNull ? '' : s, isNull, text)) return false;
    }
    return true;
  }
  function toggleFilterRow(): void {
    filterRow = !filterRow;
    if (!filterRow) setColFilters({});
  }

  // The unfiltered `{row, idx}` wrappers, memoised per rows array: switching
  // back to a 100k-row tab used to re-allocate 100k wrappers every time.
  const unfilteredMemo = new WeakMap<unknown[][], { row: unknown[]; idx: number }[]>();
  function unfilteredView(rows: unknown[][]): { row: unknown[]; idx: number }[] {
    let v = unfilteredMemo.get(rows);
    if (!v) {
      v = rows.map((row, idx) => ({ row, idx }));
      unfilteredMemo.set(rows, v);
    }
    return v;
  }

  // Rows passing the filter, carrying their original index so edits target the
  // right entry in `liveRows`. Purely client-side over the fetched rows.
  // The last filtered view is kept per rows array (grid-view-cache.ts), so a
  // tab switch back to a filtered 100k-row result is O(1).
  const filteredRows = $derived.by<{ row: unknown[]; idx: number }[]>(() => {
    const hasChips = activeChips.length > 0;
    const hasCols = activeColFilters.length > 0;
    const rows = liveRows;
    if (!filtering && !hasChips && !hasCols) return unfilteredView(rows);
    const key = [
      filtering ? searchLc : '',
      JSON.stringify(activeColFilters),
      JSON.stringify(activeChips.map((c) => (c.kind === 'col' ? [c.column, c.op, c.values] : null))),
    ].join('\u0001');
    return memoFilter(rows, key, () => {
      const all = unfilteredView(rows);
      const scan = filtering ? scanFor(rows, cellStr) : null;
      const needle = searchLc;
      const out: { row: unknown[]; idx: number }[] = [];
      for (let idx = 0; idx < rows.length; idx++) {
        const row = rows[idx];
        if (hasChips && !chipMatches(row)) continue;
        if (hasCols && !colFilterMatches(idx)) continue;
        if (scan && !(scan[idx] ?? '').includes(needle)) continue;
        out.push(all[idx]);
      }
      return out;
    });
  });

  // ── Row detail side panel (grid view) ────────────────────────────────────────
  // The record under the grid's cursor, one field per line — TablePlus's
  // "row detail". Remembered per device; the panel is user-toggled, so it never
  // appears on its own and shifts the grid.
  const DETAIL_KEY = 'db.rowDetail';
  let showDetail = $state(
    ((): boolean => {
      try {
        return localStorage.getItem(DETAIL_KEY) === '1';
      } catch {
        return false;
      }
    })(),
  );
  function toggleDetail(): void {
    showDetail = !showDetail;
    try {
      localStorage.setItem(DETAIL_KEY, showDetail ? '1' : '0');
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  /** liveRows index shown in the panel: the cursor row, else the first
   *  selected row, else the first row in view. */
  let detailIdx = $state<number | null>(null);
  const detailRow = $derived.by<number | null>(() => {
    if (detailIdx !== null && detailIdx < liveRows.length) return detailIdx;
    const sel = flow.selected.size > 0 ? [...flow.selected][0] : null;
    if (sel !== null && sel !== undefined) return sel;
    return viewRows[0]?.idx ?? null;
  });
  function stepDetail(delta: number): void {
    const cur = detailRow;
    if (cur === null) return;
    const pos = viewRows.findIndex((r) => r.idx === cur);
    const next = viewRows[Math.max(0, Math.min(viewRows.length - 1, pos + delta))];
    if (next) detailIdx = next.idx;
  }

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

  // ONE collator for every comparison: `localeCompare(…, { sensitivity })`
  // built a fresh collator per call in JSC — ~1.5 s to sort 100k strings.
  const SORT_COLLATOR = new Intl.Collator(undefined, { sensitivity: 'base' });

  // Final displayed rows: filter first, then sort (stable). Both in-memory.
  // Each value's sort key (empty? / number / string) is computed ONCE per sort
  // instead of per comparison (~1.7M comparisons at 100k rows).
  // The last sorted view is kept per rows array: switching back to a sorted
  // tab skips the 100k-string collator sort (150–400 ms in JSC).
  const viewRows = $derived.by<{ row: unknown[]; idx: number }[]>(() => {
    const base = filteredRows;
    if (!sorting || sortCol === null || sortDir === null) return base;
    const col = sortCol;
    const factor = sortDir === 'asc' ? 1 : -1;
    return memoSort(liveRows, base, `${col}|${sortDir}`, () => sortView(base, col, factor));
  });
  function sortView(base: { row: unknown[]; idx: number }[], col: number, factor: number): { row: unknown[]; idx: number }[] {
    const n = base.length;
    const empty = new Uint8Array(n);
    const nums = new Float64Array(n);
    const isNum = new Uint8Array(n);
    const strs: string[] = new Array(n);
    for (let i = 0; i < n; i++) {
      const v = base[i].row[col];
      if (isEmptyVal(v)) {
        empty[i] = 1;
        strs[i] = '';
        continue;
      }
      const num = numericVal(v);
      if (num !== null) {
        isNum[i] = 1;
        nums[i] = num;
      }
      strs[i] = String(v);
    }
    const order = new Array<number>(n);
    for (let i = 0; i < n; i++) order[i] = i;
    order.sort((a, b) => {
      // Empty values pinned to the bottom in BOTH directions.
      if (empty[a] || empty[b]) {
        if (empty[a] && empty[b]) return a - b;
        return empty[a] ? 1 : -1;
      }
      const cmp = isNum[a] && isNum[b] ? nums[a] - nums[b] : SORT_COLLATOR.compare(strs[a], strs[b]);
      if (cmp !== 0) return cmp * factor;
      return a - b; // stable tiebreak
    });
    const out = new Array<{ row: unknown[]; idx: number }>(n);
    for (let i = 0; i < n; i++) out[i] = base[order[i]];
    return out;
  }

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
  // Every record the JSON / Vertical views may draw, for ⌘F to search past the
  // drawn batch (json-find.ts). Lazy: only a find reads it.
  const findRows = $derived.by<Record<string, unknown>[]>(() => {
    if (!result || mode === 'grid') return [];
    const names = uniqueColNames;
    return viewRows.slice(0, VIEW_CAP).map(({ row }) => {
      const o: Record<string, unknown> = {};
      names.forEach((n, i) => (o[n] = row[i]));
      return o;
    });
  });

  // ── Edit flow ────────────────────────────────────────────────────────────────
  // Editability, cell drafts, selection, viewer / doc editor and the review
  // modal live on ONE EditFlow instance. Its inputs are pushed from here (a
  // `.svelte.ts` class must not create effects at construction); the views get
  // `flow` as a prop and read/call it directly.
  const accessChild = $derived(databaseAccessChild(connectionId === database.selectedConnId ? database.activeDb : undefined));
  // Edits run in the scope the result RAN with (`ranNode`), so authorize them
  // against that scope rather than the selector's current value.
  const editAccessChild = $derived(
    ranNode === undefined
      ? accessChild
      : databaseAccessChild(connectionId === database.selectedConnId ? ranNode : undefined),
  );
  const canModify = $derived(!!connectionId && resourceAccess.can('connection',connectionId,'db_data','database','edit',editAccessChild));
  const canExport = $derived(!!connectionId && resourceAccess.can('connection',connectionId,'db_export','database','view',accessChild));
  $effect(()=>{if(connectionId)void resourceAccess.load('connection',connectionId,accessChild);});
  $effect(()=>{if(connectionId&&editAccessChild!==accessChild)void resourceAccess.load('connection',connectionId,editAccessChild);});

  // `viewOrder` per view array (memoised like `unfilteredView`): a tab switch
  // back to an unchanged 100k-row view reuses it instead of re-mapping.
  const viewOrderMemo = new WeakMap<{ row: unknown[]; idx: number }[], number[]>();
  function viewOrderOf(view: { row: unknown[]; idx: number }[]): number[] {
    let o = viewOrderMemo.get(view);
    if (!o) {
      o = view.map((r) => r.idx);
      viewOrderMemo.set(view, o);
    }
    return o;
  }
  const flow = new EditFlow();
  // While an error is shown, `result` is still the PREVIOUS run's while
  // `statement` is the failed one — never pair them (BUG-7: the edit flow and
  // the PK lookup ran the old rows against the failed statement's table). The
  // ErrorPanel / agent paths keep the failed `statement`.
  const gridStatement = $derived(error ? undefined : statement);
  $effect(() => {
    flow.update({
      result,
      liveRows,
      statement: gridStatement,
      connectionId,
      ranNode,
      engine,
      canModify,
      uniqueColNames,
      mini,
      viewOrder: viewOrderOf(viewRows),
      resultCount: resultSets.length,
    });
  });
  // Resolve the primary key whenever statement/connection/result changes. The
  // store fields are read here too (the old inline effect read them mid-body
  // and therefore tracked them) so the target re-resolves when the active DB or
  // the capabilities change — `resolveTarget` reads its inputs synchronously
  // before the first await, so they're tracked as well.
  $effect(() => {
    void gridStatement;
    void connectionId;
    void ranNode;
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
    // Clipboard parity with the editor; a failed copy toasts its own error, so
    // the confirmation only claims "copied" when the copy landed.
    const title = mode === 'set' ? 'Query by value' : 'Added to query';
    void copyWithToast(query).then((ok) =>
      toasts.success(title, ok ? 'Query updated & copied — press Run to execute' : 'Query updated — press Run to execute'),
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
      conds.push(`${qid(engine, refCol)} = ${valueLiteral(engine, v, result.columns[ci]?.type_hint)}`);
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
    const items: import('../../lib/contextmenu.svelte').MenuItem[] = hosted
      ? [
          { label: `Filter:  ${col} = ${short}`, icon: 'search', action: () => database.addQuickFilter(col, v, 'include') },
          { label: `Exclude:  ${col} ≠ ${short}`, icon: 'x', action: () => database.addQuickFilter(col, v, 'exclude') },
        ]
      : [];
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
              // The scope the SOURCE rows ran in, not the selector's value now.
              node: flow.scopeNode() ?? undefined,
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
      // The ACTIVE query is the editor's — what applyFilterQuery rewrites — not
      // the ran `statement`: "Add to query" after an un-run "Query by value"
      // must AND onto that WHERE. Read on right-click only (not reactive).
      const base = (connectionId ? database.tab?.statement : undefined) || statement || '';
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
    if (items.length > 0) items.push({ separator: true });
    items.push(
      { label: 'Expand value', icon: 'maximize', action: () => flow.openCell(v, rowIdx, ci) },
      { label: 'Copy value', icon: 'file', action: () => copyWithToast(v === null || v === undefined ? '' : cellStr(v)) },
      { label: flow.rawDocLabel, icon: 'eye', action: () => flow.openRawDoc(rowIdx) },
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
    showMenu(e, items);
  }
  function headerMenu(e: MouseEvent, ci: number): void {
    if (mini) return;
    const col = result?.columns[ci]?.name;
    if (!col) return;
    showMenu(e, [
      { label: 'Sort ascending', icon: 'arrowUp', action: () => { sortCol = ci; sortDir = 'asc'; } },
      { label: 'Sort descending', icon: 'arrowDown', action: () => { sortCol = ci; sortDir = 'desc'; } },
      { label: 'Clear sort', disabled: sortCol !== ci, action: () => { sortCol = null; sortDir = null; } },
      { separator: true },
      ...(hosted ? [{ label: `Filter by ${col}…`, icon: 'search', action: () => database.addColumnFilter(col) }] : []),
      { label: 'Copy column name', icon: 'file', action: () => copyWithToast(col) },
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
  // (Header filter-row boxes count as chips here: they narrow the view too.)
  const chipFiltering = $derived(activeChips.length > 0 || activeColFilters.length > 0);
  function exportRows(): unknown[][] {
    return filtering || sorting || chipFiltering ? viewRows.map((r) => r.row) : liveRows;
  }
  // ── Chunked export text ─────────────────────────────────────────────────────
  // Copy / Download of a 100k-row result used to build the whole string in ONE
  // task (hundreds of ms of frozen UI, no feedback). Past CHUNKED_FROM rows the
  // text is built CHUNK_ROWS at a time, yielding between chunks, behind a
  // "Preparing…" toast; downloads hand the parts straight to a Blob (no single
  // giant string). Small results stay synchronous.
  const CHUNKED_FROM = 20_000;
  const CHUNK_ROWS = 5_000;
  const yieldTask = (): Promise<void> => new Promise((res) => setTimeout(res, 0));
  /** `header` + one line per row (joined by `sep`), as string parts. */
  async function buildParts(header: string, line: (r: unknown[]) => string, sep = '\n', tail = ''): Promise<string[]> {
    const rows = exportRows();
    const parts: string[] = [header];
    for (let i = 0; i < rows.length; i += CHUNK_ROWS) {
      if (i > 0) await yieldTask();
      const chunk = rows.slice(i, i + CHUNK_ROWS).map(line).join(sep);
      parts.push(i > 0 ? sep + chunk : chunk);
    }
    if (tail) parts.push(tail);
    return parts;
  }
  /** Run `work` behind a "Preparing…" toast when the export is large. */
  async function withProgress<T>(what: string, work: () => Promise<T>): Promise<T> {
    const big = exportRows().length > CHUNKED_FROM;
    const id = big ? toasts.info(`Preparing ${what}…`, `${exportRows().length.toLocaleString()} rows`) : null;
    try {
      return await work();
    } finally {
      if (id !== null) toasts.dismiss(id);
    }
  }
  function tsvLine(r: unknown[]): string {
    return r.map((v) => exportText(v).replace(/\t/g, ' ').replace(/\n/g, ' ')).join('\t');
  }
  function tsvParts(): Promise<string[]> {
    if (!result) return Promise.resolve(['']);
    return buildParts(`${result.columns.map((c) => c.name).join('\t')}\n`, tsvLine);
  }
  function csvParts(): Promise<string[]> {
    if (!result) return Promise.resolve(['']);
    return buildParts(`${result.columns.map((c) => csvCell(c.name)).join(',')}\n`, (r) => r.map(csvCell).join(','));
  }
  function jsonParts(): Promise<string[]> {
    if (!result || exportRows().length === 0) return Promise.resolve(['[]']);
    const names = uniqueColNames;
    // Each element pretty-printed and indented one level: byte-identical to
    // JSON.stringify(array, null, 2).
    return buildParts(
      '[\n',
      (r) => '  ' + JSON.stringify(Object.fromEntries(names.map((n, i) => [n, r[i] ?? null])), null, 2).replace(/\n/g, '\n  '),
      ',\n',
      '\n]',
    );
  }
  /**
   * Copy text that may take a while to build. WebKit only honours a clipboard
   * write inside the click's activation, so the write starts NOW with a
   * promised ClipboardItem; browsers without it fall back to writeText.
   */
  async function copyBuilt(build: () => Promise<string[]>): Promise<void> {
    const text = build().then((p) => p.join(''));
    const CI = (globalThis as { ClipboardItem?: typeof ClipboardItem }).ClipboardItem;
    if (CI && navigator.clipboard?.write) {
      try {
        await navigator.clipboard.write([new CI({ 'text/plain': text.then((t) => new Blob([t], { type: 'text/plain' })) })]);
        return;
      } catch {
        /* fall through to the shared copy (legacy path on insecure origins) */
      }
    }
    await copyTextOrThrow(await text);
  }
  function csvCell(v: unknown): string {
    let s = exportText(v);
    // Formula-injection guard: a STRING cell starting with = + - @ executes when
    // the CSV lands in a spreadsheet — neutralize with a leading apostrophe.
    // Non-string values (a bare -5 is data, not a formula) are left alone.
    if (typeof v === 'string' && /^[=+\-@]/.test(s)) s = `'${s}`;
    return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s; // not-sql: CSV field quoting
  }

  const exportScope = $derived.by(() => {
    const parts: string[] = [];
    if (filtering || chipFiltering) parts.push('filtered');
    if (sorting) parts.push('sorted');
    return parts.length ? ` (${parts.join(' + ')} view)` : '';
  });

  async function copyTsv(): Promise<void> {
    try {
      await withProgress('TSV', () => copyBuilt(tsvParts));
      toasts.success('Copied', `Result copied as TSV${exportScope}`);
    } catch {
      toasts.error('Couldn’t copy', 'Select the text and copy it manually.');
    }
  }
  function download(parts: string[], name: string, mime: string): void {
    const blob = new Blob(parts, { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1500);
  }
  async function exportCsv(): Promise<void> {
    if (!canExport) return;
    download(await withProgress('CSV', csvParts), 'result.csv', 'text/csv');
  }
  async function exportJson(): Promise<void> {
    if (!canExport) return;
    download(await withProgress('JSON', jsonParts), 'result.json', 'application/json');
  }
  async function copyAs(kind: 'csv' | 'json' | 'columns'): Promise<void> {
    try {
      if (kind === 'columns') await copyTextOrThrow((result?.columns ?? []).map((c) => c.name).join(', '));
      else await withProgress(kind.toUpperCase(), () => copyBuilt(kind === 'csv' ? csvParts : jsonParts));
      const what = kind === 'columns' ? 'Column names copied' : `Result copied as ${kind.toUpperCase()}${exportScope}`;
      toasts.success('Copied', what);
    } catch {
      toasts.error('Couldn’t copy', 'Select the text and copy it manually.');
    }
  }

  // ── Toolbar menus ────────────────────────────────────────────────────────────
  // One row, fixed height: the rarely-used verbs live in three menus instead of
  // a wrapping strip of a dozen buttons (which also re-flowed — and pushed the
  // grid down — when the editability probe added its items after first paint).
  function copyMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Copy as TSV', icon: 'copy', action: () => void copyTsv() },
      { label: 'Copy as CSV', icon: 'copy', action: () => void copyAs('csv') },
      { label: 'Copy as JSON', icon: 'copy', action: () => void copyAs('json') },
      { label: 'Copy column names', icon: 'file', action: () => void copyAs('columns') },
    ];
    if (flow.copyTarget) {
      const rows = flow.selected.size > 0 ? flow.selectedIndices() : viewRows.map((r) => r.idx);
      items.push(
        { separator: true },
        {
          label: engine === 'mongodb'
            ? `Open ${flow.selected.size > 0 ? 'selected' : 'all'} as insertMany(…)`
            : `Open ${flow.selected.size > 0 ? 'selected' : 'all'} as INSERT statements`,
          icon: 'external',
          action: () => flow.copyRowsAsInsert(rows),
        },
      );
    }
    showMenu(e, items);
  }
  function exportMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: `Download CSV${exportScope}`, icon: 'download', disabled: !canExport, action: exportCsv },
      { label: `Download JSON${exportScope}`, icon: 'download', disabled: !canExport, action: exportJson },
    ];
    if (connectionId && statement) {
      // Export RE-RUNS the statement uncapped — for an obvious write
      // (`UPDATE … RETURNING *`) that would apply it twice, so it's disabled
      // with the reason (the daemon runs exports read-only regardless).
      const writeVerb = obviousWriteVerb(statement);
      items.push(
        { separator: true },
        {
          label: result?.truncated ? 'Export all rows… (result is capped)' : 'Export all rows…',
          icon: 'arrowDown',
          disabled: !canExport || !!writeVerb,
          title: writeVerb ? `Export re-runs the statement — not available for a write (${writeVerb})` : undefined,
          action: openExportDialog,
        },
      );
    }
    if (connectionId && (database.capabilities?.sql || database.capabilities?.engine === 'mongodb')) {
      items.push({ separator: true }, { label: 'Import file…', icon: 'arrowUp', action: () => database.openImportDialog() });
    }
    showMenu(e, items);
  }
  function moreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [];
    if (engine === 'mongodb' && connectionId) {
      items.push({ label: 'Aggregate pipeline…', icon: 'layers', action: () => (pipelineOpen = true) });
    }
    if (mode !== 'grid') {
      // Row checkboxes live in the Grid view only — here the per-record ⋯ menu
      // ("Compare with…") is the way in, so a disabled item names it.
      items.push({
        label: flow.selected.size === 2 ? 'Compare two records…' : 'Compare two records… (use a record’s ⋯ → Compare with…)',
        icon: 'columns',
        disabled: flow.selected.size !== 2,
        action: () => (compare = [...flow.selected] as [number, number]),
      });
    }
    if (flow.editable) {
      items.push({
        label: `Insert ${engine === 'mongodb' ? 'document' : 'row'} from JSON…`,
        icon: 'plus',
        action: () => flow.openDocEditor(-1),
      });
    }
    if (mode === 'grid') {
      items.push({
        label: expandJson ? 'Collapse JSON cells' : 'Expand JSON cells',
        icon: expandJson ? 'minimize' : 'maximize',
        action: () => (expandJson = !expandJson),
      });
    }
    if (items.length) items.push({ separator: true });
    items.push({ label: 'Send to running agent…', icon: 'send', action: sendToRunningAgent });
    // Same gate as the editor's Ask AI buttons: the DB Assistant runs an agent.
    if (connectionId && canAssist) items.push({ label: 'Examine with Otto', icon: 'sparkle', action: examineWithAi });
    showMenu(e, items);
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
;
async () => {







 { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});runAnnounce; }
if(!mini && resultSets.length > 1){
  
   { svelteHTML.createElement("div", {     "class":`rg-switch`,"role":`tablist`,"aria-label":`Result sets`,});
        for(let rs of __sveltets_2_ensureArray(resultSets)){let i = 1;i;
       { svelteHTML.createElement("button", {                 "class":`rg-seg`,"role":`tab`,"aria-selected":resultIdx === i,"tabindex":resultIdx === i ? 0 : -1,"onkeydown":onTabKey,"title":rs.statement ?? `Result ${i + 1}`,"onclick":() => (resultIdx = i),});resultIdx === i;rs.errored;
        if(rs.errored){ { svelteHTML.createElement("span", {   "class":`rg-seg-dot`,"aria-hidden":`true`,}); }}
         i + 1;if(rs.errored){ { svelteHTML.createElement("span", { "class":`sr-only`,});  }}
       }
    }
   }
}
if(error){
  if(mini){
     { svelteHTML.createElement("div", { "class":`grid-error mono`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
       { svelteHTML.createElement("span", {});error; }
     }
  }else{
     { const $$_lenaProrrE0C = __sveltets_2_ensureComponent(ErrorPanel); new $$_lenaProrrE0C({ target: __sveltets_2_any(), props: {      error,engine,"statement":statement ?? '',"onAskAi":hosted && canAssist ? askAiToFix : undefined,}});}
  }
} else if (!resultProp){
  if(!mini){
    if(running){
      ;__sveltets_2_ensureSnippet(loadingFrame());
    }else{
       { svelteHTML.createElement("div", { "class":`grid-empty idle`,});
         { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`grid`,"title":`No results yet`,"body":`Run a query with ⌘↵ — the statement under the cursor, or your selection.`,}});}
       }
    }
  }
} else if (isReleased(resultProp)){
  
  if(running){
    if(!mini){;__sveltets_2_ensureSnippet(loadingFrame());}
  } else if (!mini){
     { svelteHTML.createElement("div", {   "class":`grid-released`,...__sveltets_2_empty({"data-testid":`db-result-released`}),});
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`refresh`,"title":`Result released to save memory`,"body":`${releasedRows(resultProp).toLocaleString()} rows were dropped while this tab was in the background. Re-run the query to bring them back.`,"actionLabel":hosted && statement ? 'Re-run' : undefined,"actionIcon":`play`,"onaction":() => void database.rerunReleased(),}});}
     }
  }
} else if (!result || result.columns.length === 0){
  if(running && !mini){
    ;__sveltets_2_ensureSnippet(loadingFrame());
  } else if (running){
     { svelteHTML.createElement("div", { "class":`grid-empty`,});;__sveltets_2_ensureSnippet(runningCard()); }
  }else{
     { svelteHTML.createElement("div", { "class":`grid-empty`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":mini ? 16 : 24,}});}
       { svelteHTML.createElement("span", {});emptyResultLabel; }
      if(!mini && result){
         { svelteHTML.createElement("span", { "class":`ge-meta`,});result.stats.duration_ms; if(result.rows_affected != null){ result.rows_affected; } }
      }
     }
  }
}else{
   { svelteHTML.createElement("div", {   "class":`grid-wrap`,...__sveltets_2_empty({"data-state":`ready`}),});mini;
    if(result.message && !mini){
       { svelteHTML.createElement("div", {   "class":`grid-notice mono`,"title":result.message,});result.message; }
    }
    if(!mini){
       { svelteHTML.createElement("div", { "class":`grid-toolbar`,});
         { svelteHTML.createElement("div", { "class":`gt-search`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
           { svelteHTML.createElement("input", {                      "dir":`ltr`,"class":`gt-search-input mono`,"type":`text`,"placeholder":`Filter rows…`,"aria-label":`Filter rows`,"value":searchInput,"oninput":(e) => setSearch(e.currentTarget.value, true),"onfocus":onSearchFocus,"onblur":onSearchBlur,"spellcheck":`false`,"autocomplete":`off`,});}
          if(filtering || searchInput){
             { svelteHTML.createElement("button", {       "class":`gt-search-clear`,"title":`Clear filter`,"aria-label":`Clear filter`,"onclick":() => setSearch(''),});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
          }
         }
         { svelteHTML.createElement("div", {   "class":`view-seg`,"title":modeReason,});
           { svelteHTML.createElement("div", {     "class":`view-tabs`,"role":`group`,"aria-label":`Result view`,});
             { svelteHTML.createElement("button", {        "class":`vs`,"aria-pressed":mode === 'grid',"onclick":() => pickView('grid'),"title":`Columnar grid`,});mode === 'grid';  }
             { svelteHTML.createElement("button", {        "class":`vs`,"aria-pressed":mode === 'vertical',"onclick":() => pickView('vertical'),"title":`One record per block (field: value)`,});mode === 'vertical';  }
             { svelteHTML.createElement("button", {        "class":`vs`,"aria-pressed":mode === 'json',"onclick":() => pickView('json'),"title":`One JSON object per row`,});mode === 'json';  }
           }
          if(showAutoChip){
            
             { svelteHTML.createElement("button", {     "class":`vs auto`,"onclick":() => pickView(null),"title":autoTitle,});  }
          }
         }
        if(mode === 'grid'){
           { svelteHTML.createElement("button", {            "class":`icon-btn tb-toggle`,"aria-pressed":filterRow,"onclick":toggleFilterRow,"aria-label":`Filter row`,"title":`Filter row — a filter box under every column header`,});filterRow; { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`filter`,"size":13,}});} }
           { svelteHTML.createElement("button", {            "class":`icon-btn tb-toggle`,"aria-pressed":showDetail,"onclick":toggleDetail,"aria-label":`Row detail`,"title":`Row detail — the selected record, one field per line`,});showDetail; { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`panel`,"size":13,}});} }
        }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => findInPage.show(),"aria-label":`Find in results`,"title":`Find (⌘F)`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
         }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":copyMenu,"title":`Copy the result${exportScope}`,"aria-haspopup":`menu`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} { svelteHTML.createElement("span", { "class":`tb-label`,});  } { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {        "class":`btn small`,"onclick":exportMenu,"title":`Download, export all rows, or import a file`,"aria-haspopup":`menu`,});result?.truncated;
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} { svelteHTML.createElement("span", { "class":`tb-label`,});  } { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":moreMenu,"aria-label":`More result actions`,"title":`More — pipeline, compare, insert, expand JSON, send to agent, examine with AI`,"aria-haspopup":`menu`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
       }
    }

    if(!mini && flow.selected.size > 0){
       { svelteHTML.createElement("div", { "class":`sel-bar`,});
         { svelteHTML.createElement("span", { "class":`sel-count`,});flow.selected.size;  }
        if(flow.copyTarget){
           { svelteHTML.createElement("button", {       "class":`sel-gen`,"onclick":() => flow.copySelectedAsInsert(),"title":engine === 'mongodb'
              ? 'Open the selected rows as an insertMany(…) in a new tab (not run)'
              : 'Open the selected rows as INSERT statements in a new tab (not run)',});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}  
           }
        }
        if(flow.editable && engine !== 'redis'){
           { svelteHTML.createElement("button", {       "class":`sel-gen`,"onclick":() => flow.copySelectedWhere(),"title":"Copy a `pk IN (…)` predicate for the selected rows to the clipboard",});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}   
           }
        }
        if(flow.selected.size === 2){
           { svelteHTML.createElement("button", {     "class":`sel-gen`,"onclick":() => (compare = [...flow.selected] as [number, number]),"title":`Compare the two selected records side by side`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":12,}});}  }
        }
        if(flow.editable){
         { svelteHTML.createElement("button", {     "class":`sel-del`,"onclick":() => flow.deleteSelected(),"title":`Delete selected rows (you review before it runs)`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
         }
        }
         { svelteHTML.createElement("button", {   "class":`sel-clear`,"onclick":() => flow.clearSelection(),});  }
        if(flow.editable){ { svelteHTML.createElement("span", { "class":`sel-hint`,});       }}
       }
    }

    if(!mini && hosted && database.filters.length > 0){
       { svelteHTML.createElement("div", { "class":`filter-bar`,});
         { svelteHTML.createElement("span", { "class":`fb-label`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}  }
            for(let cond of __sveltets_2_ensureArray(database.filters)){let ci = 1;ci;
          if(cond.kind === 'raw'){
             { svelteHTML.createElement("span", {   "class":`cond raw`,"title":`Existing WHERE condition`,});
               { svelteHTML.createElement("span", { "class":`cond-text mono`,});cond.text; }
               { svelteHTML.createElement("button", {       "class":`cond-x`,"title":`Remove`,"aria-label":`Remove`,"onclick":() => database.removeFilterCond(ci),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
          }else{
             { svelteHTML.createElement("span", {  "class":`cond`,});cond.op === 'not_in';
               { svelteHTML.createElement("button", {       "class":`cond-op`,"title":cond.op === 'in' ? 'Include (click to exclude)' : 'Exclude (click to include)',"onclick":() => database.toggleFilterMode(ci),});cond.op === 'in' ? '=' : '≠'; }
               { svelteHTML.createElement("span", { "class":`cond-col mono`,});cond.column; }
                  for(let val of __sveltets_2_ensureArray(cond.values)){let vi = 1;vi;
                 { svelteHTML.createElement("span", { "class":`cond-val mono`,});
                  val.isNull ? 'NULL' : val.raw;
                   { svelteHTML.createElement("button", {       "class":`val-x`,"aria-label":`Remove value`,"title":`Remove value`,"onclick":() => database.removeFilterValue(ci, vi),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                 }
              }
               { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`cond-add mono`,"placeholder":`+ value`,"aria-label":`Add a value to the ${cond.column} filter`,"bind:value":addValText[ci],"onkeydown":(e) => { if (e.key === 'Enter') submitFilterValue(ci); },});/*Ωignore_startΩ*/() => addValText[ci] = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               { svelteHTML.createElement("button", {       "class":`cond-x`,"title":`Remove filter`,"aria-label":`Remove filter`,"onclick":() => database.removeFilterCond(ci),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
          }
        }
         { svelteHTML.createElement("button", {     "class":`fb-clear`,"onclick":() => database.clearFilters(),"title":`Clear all filters`,});  }
         { svelteHTML.createElement("span", { "class":`fb-hint`,});          }
       }
    }

    
    if(engine === 'mongodb' && !mini && statement){
       { const $$_raBretliFognoM1C = __sveltets_2_ensureComponent(MongoFilterBar); new $$_raBretliFognoM1C({ target: __sveltets_2_any(), props: {      statement,"columns":result.columns.map((c) => c.name),"onrun":(q) => { database.setStatementFromCellFilter(q); void database.runQuery(); },}});}
    }
    if(mode === 'json'){
       { const $$_weiVnosJ1C = __sveltets_2_ensureComponent(JsonView); new $$_weiVnosJ1C({ target: __sveltets_2_any(), props: {                   result,objRows,uniqueColNames,engine,flow,mini,altRemaining,viewTruncated,"viewCap":VIEW_CAP,"totalRows":viewRows.length,"onshowmore":() => (altShown += ALT_BATCH),expansion,"oncompare":(l, r) => (compare = [l, r]),findRows,}});}
    } else if (mode === 'vertical'){
       { const $$_weiVlacitreV1C = __sveltets_2_ensureComponent(VerticalView); new $$_weiVlacitreV1C({ target: __sveltets_2_any(), props: {                   result,objRows,uniqueColNames,engine,flow,mini,altRemaining,viewTruncated,"viewCap":VIEW_CAP,"totalRows":viewRows.length,"onshowmore":() => (altShown += ALT_BATCH),expansion,"oncompare":(l, r) => (compare = [l, r]),findRows,}});}
    }else{
       { svelteHTML.createElement("div", {  "class":`grid-body`,});mini;
         { const $$_weiVdirG2C = __sveltets_2_ensureComponent(GridView); new $$_weiVdirG2C({ target: __sveltets_2_any(), props: {                             result,liveRows,viewRows,mini,expandJson,flow,filtering,searchLc,sortCol,sortDir,"resetToken":colKey,"filterRow":filterRow && !mini,"colFilters":colFiltersInput,"oncolfilter":setColFilter,"oncolfocus":onColFilterFocus,"onfocusrow":(i) => { if (i !== null) detailIdx = i; },"oncellmenu":cellMenu,"onheadermenu":headerMenu,"oncyclesort":cycleSort,}});}
        if(showDetail && !mini){
           { const $$_liateDwoR2C = __sveltets_2_ensureComponent(RowDetail); new $$_liateDwoR2C({ target: __sveltets_2_any(), props: {                    result,"row":detailRow === null ? null : (liveRows[detailRow] ?? null),"rowIdx":detailRow,"position":detailRow === null ? -1 : viewRows.findIndex((r) => r.idx === detailRow),"total":viewRows.length,"onstep":stepDetail,"onopen":(ci) => { if (detailRow !== null) flow.openCell(liveRows[detailRow]?.[ci], detailRow, ci); },"rawLabel":flow.rawDocLabel,"onraw":() => { if (detailRow !== null) flow.openRawDoc(detailRow); },"onclose":toggleDetail,}});}
        }
       }
    }
    if(flow.pendingCells > 0){
       { svelteHTML.createElement("div", {   "class":`pending-bar`,...__sveltets_2_empty({"data-testid":`pending-edits-bar`}),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
         { svelteHTML.createElement("span", {});
           { svelteHTML.createElement("strong", {});flow.pendingCells; } pluralNoun(flow.pendingCells, 'pending change'); 
           { svelteHTML.createElement("strong", {});flow.pendingEdits.size; } pluralNoun(flow.pendingEdits.size, 'row');  
                
         }
         { svelteHTML.createElement("span", { "class":`pending-spacer`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => flow.discardPending(),});  }
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => flow.reviewPending(),});   }
       }
    }
    if(!mini){
      
       { svelteHTML.createElement("div", { "class":`grid-foot`,});
        if(filtering || chipFiltering){
           { svelteHTML.createElement("span", {}); { svelteHTML.createElement("strong", {});viewRows.length.toLocaleString(); }  liveRows.length.toLocaleString(); pluralNoun(liveRows.length, 'row'); }
        }else{
           { svelteHTML.createElement("span", {}); { svelteHTML.createElement("strong", {});result.stats.row_count.toLocaleString(); } pluralNoun(result.stats.row_count, 'row'); }
        }
        if(sorting && sortCol !== null){
           { svelteHTML.createElement("button", {     "class":`sort-chip`,"title":`Clear sort on ${result.columns[sortCol].name}`,"onclick":() => { sortCol = null; sortDir = null; },});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":sortDir === 'asc' ? 'arrowUp' : 'arrowDown',"size":12,}});}
             { svelteHTML.createElement("span", { "class":`sort-chip-name`,});result.columns[sortCol].name; }
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
           }
        }
         { svelteHTML.createElement("span", { "class":`dot`,});  }
         { svelteHTML.createElement("span", {});result.stats.duration_ms.toLocaleString();  }
        if(result.stats.bytes_read != null){
           { svelteHTML.createElement("span", { "class":`dot`,});  }
           { svelteHTML.createElement("span", {});formatBytes(result.stats.bytes_read);  }
        }
        if(result.rows_affected != null){
           { svelteHTML.createElement("span", { "class":`dot`,});  }
           { svelteHTML.createElement("span", {});result.rows_affected.toLocaleString();  }
        }
        if(result.truncated && result.truncated_reason === 'bytes'){
           { svelteHTML.createElement("span", {     "class":`trunc-badge`,"title":`The response reached the 32 MB size budget. Narrow the query to retrieve omitted data.`,}); result.stats.row_count.toLocaleString();     }
        } else if (result.truncated){
           { svelteHTML.createElement("span", {     "class":`trunc-badge`,"title":`Row cap reached — more rows exist. Raise the row limit or narrow the query to fetch more.`,});  result.stats.row_count.toLocaleString(); }
        }
        if(showPager){
           { svelteHTML.createElement("span", { "class":`dot`,});  }
           { svelteHTML.createElement("span", { "class":`pager`,});
             { svelteHTML.createElement("button", {         "class":`pg-btn`,"disabled":offset <= 0,"onclick":() => database.runPage(-1),"title":`Previous page`,"aria-label":`Previous page`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});}  }
             { svelteHTML.createElement("span", { "class":`pg-range mono`,}); pageFrom.toLocaleString(); pageTo.toLocaleString(); }
             { svelteHTML.createElement("button", {         "class":`pg-btn`,"disabled":!hasNextPage,"onclick":() => database.runPage(1),"title":`Next page`,"aria-label":`Next page`,});  { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});} }
            if(!hasOrderBy){ { svelteHTML.createElement("span", {   "class":`pg-unordered`,"title":`Without an ORDER BY, row order can shift between pages`,});  }}
           }
        }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(result?.masked){
           { svelteHTML.createElement("span", {   "class":`tb-masked`,"title":`Server-side PII masking was applied — sensitive values were redacted before leaving the server`,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}
           }
        }
        if(flow.editable){
           { svelteHTML.createElement("span", {     "class":`gt-edit-hint`,"title":`Double-click a cell to edit (you review the SQL before it runs). Primary key ${flow.editPkCols.length > 1 ? 'columns' : 'column'} (${flow.editPkCols.join(', ')}) ${flow.editPkCols.length > 1 ? 'are' : 'is'} read-only.`,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}    
           }
        } else if (result.message){
           { svelteHTML.createElement("span", {   "class":`msg`,"title":result.message,});result.message; }
        } else if (statement && flow.editReason){
           { svelteHTML.createElement("span", {   "class":`edit-note`,"title":flow.editReason,});flow.editReason; }
        }
       }
    }
    if(running && !mini){
      
       { svelteHTML.createElement("div", { "class":`rg-overlay`,});
         { svelteHTML.createElement("div", { "class":`rg-overlay-card`,});
           { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 16px`,"aria-hidden":`true`,}); }
           { svelteHTML.createElement("span", { "class":`rg-overlay-text`,}); elapsed;  }
          if(cancelQuery){
             { svelteHTML.createElement("button", {     "class":`rg-cancel`,"onclick":cancelQuery,"title":`Cancel the running query`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
          }
         }
       }
    }
   }
}

if(flow.viewer){
   { const $$_reweiVlleC0C = __sveltets_2_ensureComponent(CellViewer); new $$_reweiVlleC0C({ target: __sveltets_2_any(), props: { flow,}});}
}

if(flow.docEditor){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":DocEditorLazy,"what":`the document editor`,"quiet":true,"props":{ flow },"onerror":() => (flow.docEditor = null),}});}
}

if(flow.rawDoc){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":RawDocViewerLazy,"what":`the document viewer`,"quiet":true,"props":{ flow },"onerror":() => (flow.rawDoc = null),}});}
}

if(showExportDialog && connectionId && statement){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {          "lazy":ExportDialogLazy,"what":`the export dialog`,"quiet":true,"props":{ statement, connectionId, node: ranNode, canExport, onclose: () => (showExportDialog = false) },"onerror":() => (showExportDialog = false),}});}
}

if(compare && result){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {          "lazy":RecordDiffLazy,"what":`the row comparison`,"quiet":true,"props":{
      left: objRowAt(compare[0]),
      right: objRowAt(compare[1]),
      leftLabel: `#${compare[0] + 1}`,
      rightLabel: `#${compare[1] + 1}`,
      onclose: () => (compare = null),
    },"onerror":() => (compare = null),}});}
}

if(connectionId && pipelineMounted){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {          "lazy":AggregateBuilderLazy,"what":`the pipeline builder`,"quiet":true,"props":{
      collection: engine === 'mongodb' ? flow.editTable : null,
      open: pipelineOpen,
      connId: connectionId,
      onclose: () => (pipelineOpen = false),
      oninsert: (stmt: string) => database.setStatement(stmt),
      onrun: (stmt: string) => { database.setStatement(stmt); void database.runQuery(stmt); },
    },"onerror":() => { pipelineOpen = false; pipelineMounted = false; },}});}
}

if(flow.reviewSql){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {          "lazy":ReviewModalLazy,"what":`the write review`,"quiet":true,"props":{
      title: flow.reviewSql.title,
      sql: flow.reviewSql.sql,
      diff: flow.reviewSql.diff,
      error: flow.reviewError,
      running: flow.runningReview,
      onsql: (s: string) => flow.setReviewSql(s),
      onrun: () => void flow.runReview(),
      onclose: () => flow.closeReview(),
    },"onerror":() => flow.closeReview(),}});}
}

if(sendToAgentOpen && ws.current && sendToAgentPayload !== null){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {          "lazy":ContextPacketDialogLazy,"what":`the send-to-agent dialog`,"quiet":true,"props":{
      workspaceId: ws.current.id,
      sessionId: ws.targetAgentId,
      kind: 'db',
      payload: sendToAgentPayload,
      onclose: () => (sendToAgentOpen = false),
    },"onerror":() => (sendToAgentOpen = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ResultsGrid__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ResultsGrid__SvelteComponent_ = ReturnType<typeof ResultsGrid__SvelteComponent_>;
/*Ωignore_endΩ*/export default ResultsGrid__SvelteComponent_;