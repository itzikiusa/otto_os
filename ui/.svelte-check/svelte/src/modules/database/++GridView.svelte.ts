///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
// Virtualized results table: sticky two-line header (name over type),
// monospace cells, numbers right-aligned in tabular figures, NULL as a dim
// italic `NULL`, objects/arrays as compact JSON with a click-to-expand cell
// viewer. Columns auto-size to their content, are drag-resizable and
// drag-reorderable, and an optional filter row sits under the header. Sort /
// search / quick-filter state is owned by ResultsGrid and arrives as props;
// the header + cell context menus are callbacks; every editing concern
// (drafts, selection, viewer) lives on `flow`.
//
// Layout stability: nothing that resolves AFTER the rows paint (the
// editability probe, the PK lookup) may change a column's width. The
// row-number column reserves the selection-checkbox slot whether or not the
// result turns out editable, so the grid never shifts sideways when the
// probe lands (the "table jumps when I open it" bug).
import { onDestroy, tick, untrack } from 'svelte';
import type { Attachment } from 'svelte/attachments';
import Icon from '../../lib/components/Icon.svelte';
import { bsonScalar } from '../../lib/jsonTree/bson';
import type { QueryResult } from '../../lib/api/types';
import type { EditFlow } from './EditFlow.svelte';
import { CELL_MAX, cellDisplay, cellStr, clip, copyWithToast, isComplex, previewJson } from './results-format';
import { registerFindProvider } from '../../lib/findProviders';
import { columnKind, moveColumn, rowNumberWidthCh, type ColumnKind } from './grid-format';
import { buildCell, widthStyle } from './grid-cells';
;

  interface Props {
    result: QueryResult;
    liveRows: unknown[][];
    /** Filtered + sorted rows, each carrying its ORIGINAL liveRows index. */
    viewRows: { row: unknown[]; idx: number }[];
    mini: boolean;
    expandJson: boolean;
    flow: EditFlow;
    filtering: boolean;
    searchLc: string;
    sortCol: number | null;
    sortDir: 'asc' | 'desc' | null;
    /** Column-name signature of the result — widths, order + scroll reset when it changes. */
    resetToken: string;
    /** Show the per-column filter row under the header. */
    filterRow?: boolean;
    /** Per-column filter text, keyed by ORIGINAL column index. */
    colFilters?: Record<number, string>;
    oncolfilter?: (ci: number, text: string) => void;
    /** A filter box gained (column index) or lost (null) focus. */
    oncolfocus?: (ci: number | null) => void;
    /** The keyboard/click cursor moved onto a row (its liveRows index). */
    onfocusrow?: (idx: number | null) => void;
    oncellmenu: (e: MouseEvent, ci: number, v: unknown, rowIdx: number) => void;
    onheadermenu: (e: MouseEvent, ci: number) => void;
    oncyclesort: (ci: number) => void;
  };;

  // ── Delegated cell events ───────────────────────────────────────────────
  // One click / dblclick / contextmenu listener on <tbody> instead of three
  // closures (plus an Icon component) per rendered cell: a scroll step that
  // swaps in 40 rows × 30 columns no longer allocates ~3,600 handlers. Each
  // cell carries data-r (its liveRows index), data-c (original column index),
  // data-p (display position) and data-k (kind: d = parked draft, u = nested
  // change under it, n = NULL, j = JSON, p = plain). The virtual row position
  // the keyboard cursor uses is resolved from the window on click, so a
  // scroll step never rewrites the rows that stay mounted.
  type CellHit = { idx: number; ci: number; pos: number; vpos: number; kind: string; expand: boolean };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    result,
    liveRows,
    viewRows,
    mini,
    expandJson,
    flow,
    filtering,
    searchLc,
    sortCol,
    sortDir,
    resetToken,
    filterRow = false,
    colFilters = {},
    oncolfilter,
    oncolfocus,
    onfocusrow,
    oncellmenu,
    onheadermenu,
    oncyclesort,
  }: Props = $props();

  // ── Windowed virtualization (main grid only) ─────────────────────────────────
  // Render only the rows in (or near) the viewport, with spacer rows preserving
  // the full scroll height. Row height is fixed in CSS (see ROW_H), so the math
  // is exact and we can scroll smoothly through 100k+ rows.
  // "Expand JSON" mode pretty-prints complex cells inline; rows grow to a fixed
  // taller height so the virtualization math stays exact.
  const ROW_H = $derived(expandJson ? 168 : 26); // must match `.grid tbody td` height
  const OVERSCAN = 12;
  let scrollEl = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let viewportH = $state(0);
  const virtualize = $derived(!mini);
  // ── Column (horizontal) virtualisation ──────────────────────────────────────
  // A 200–300-column ClickHouse result used to mount every column of every
  // windowed row (10–20k cells, 12×N rebuilt per scroll step). Past HVIRT_MIN
  // columns the body renders only the columns near the viewport: a leading
  // spacer cell spanning (colspan) the columns before the window, the window's
  // real cells, a trailing spacer for the rest. The header and filter rows
  // are windowed the same way: a `<colgroup>` carries every column's width
  // and the table switches to `table-layout: fixed` with a definite width, so
  // the grid no longer depends on 300 header cells. Those header cells (a
  // button + four spans each) used to be re-laid out on EVERY vertical step —
  // measured in WebKit at 15 of the 33 ms a 300 px step cost on a 300-column
  // result, vs 5 ms with only the window's headers mounted. Column edges are
  // arithmetic (prefix sums of the widths in ch × a measured px-per-ch), never
  // a read of N header cells. Cells keep their real `data-p` (display
  // position), so delegated clicks, the keyboard cursor and find are
  // unaffected. RTL works the same way: the edges are measured from the
  // table's inline START, and RTL's scrollLeft runs 0 → negative, so the
  // window reads |scrollLeft|. Off for mini grids.
  const HVIRT_MIN = 40;
  /** Columns rendered beyond each side of the viewport. */
  const COL_OVERSCAN = 3;
  /** Window bounds snap to multiples of this, so a horizontal scroll rebuilds
   *  the mounted rows every few columns rather than on every column edge. */
  const COL_CHUNK = 4;
  let scrollLeft = $state(0);
  let viewportW = $state(0);
  let rtl = $state(false);
  /** Rendered px per `ch` of a column width and the row-number column's px
   *  width — measured once per width change (see the effect below); the
   *  defaults only place the very first window. */
  let pxPerCh = $state(7.2);
  let rnPx = $state(60);

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
    viewportW = el.clientWidth;
    rtl = getComputedStyle(el).direction === 'rtl';
    const ro = new ResizeObserver(() => {
      viewportH = el.clientHeight;
      viewportW = el.clientWidth;
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Preserve column widths / order / scroll when the new result has the SAME
  // columns (a re-run of the same query), so the grid doesn't jump; reset them
  // only when the shape actually changes — `resetToken` is that signature.
  $effect(() => {
    void resetToken;
    scrollTop = 0;
    if (scrollEl) scrollEl.scrollTop = 0;
    colWidths = {};
    dragName = null;
    order = untrack(() => result.columns.map((_c, i) => i));
  });

  // ── Column order (drag a header onto another to move it) ─────────────────────
  // Display position → ORIGINAL column index. Everything row-side (values,
  // edits, menus, filters) keeps speaking original indices.
  let order = $state<number[]>([]);
  const cols = $derived(
    order.length === result.columns.length ? order : result.columns.map((_c, i) => i),
  );
  let dragFrom = $state<number | null>(null);
  let dropAt = $state<number | null>(null);
  function onHeadDragStart(e: DragEvent, pos: number): void {
    dragFrom = pos;
    e.dataTransfer?.setData('text/plain', String(pos));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onHeadDragOver(e: DragEvent, pos: number): void {
    if (dragFrom === null) return;
    e.preventDefault();
    dropAt = pos;
  }
  function onHeadDrop(e: DragEvent, pos: number): void {
    e.preventDefault();
    if (dragFrom !== null) order = moveColumn(cols, dragFrom, pos);
    dragFrom = null;
    dropAt = null;
  }
  function onHeadDragEnd(): void {
    dragFrom = null;
    dropAt = null;
  }

  // Per-column layout kind (numbers right-aligned, JSON link-styled…).
  const kinds = $derived.by<ColumnKind[]>(() =>
    result.columns.map((c, i) =>
      columnKind(
        c.type_hint,
        liveRows.slice(0, 50).map((r) => r[i]),
      ),
    ),
  );

  // Row-number column width: a function of the ROW COUNT only (see header note).
  const rnCh = $derived(rowNumberWidthCh(liveRows.length));

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

  // Patch the virtual window in the rendering phase, not in a scroll-event
  // microtask. WebKit can reset the native offset while committing that row /
  // spacer swap during scroll dispatch. One queued frame also coalesces bursts
  // without forcing a synchronous layout read after every DOM patch.
  let scrollFrame: number | undefined;
  function onScroll(): void {
    if (!scrollEl || scrollFrame !== undefined) return;
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = undefined;
      if (!scrollEl) return;
      scrollTop = scrollEl.scrollTop;
      if (hvirt) scrollLeft = scrollEl.scrollLeft;
    });
  }
  onDestroy(() => {
    if (scrollFrame !== undefined) cancelAnimationFrame(scrollFrame);
  });

  const hvirt = $derived(virtualize && result.columns.length > HVIRT_MIN);
  /** Horizontal scroll offset from the inline start (RTL scrolls negative). */
  const inlineScroll = $derived(Math.abs(scrollLeft));
  /** Right edge (px, from the table's start) of each display position:
   *  row-number width + prefix sum of the column widths. Pure arithmetic —
   *  recomputed when widths / order change, never per scroll step. */
  const colEdges = $derived.by<number[]>(() => {
    if (!hvirt) return [];
    const edges = new Array<number>(cols.length);
    let x = rnPx;
    for (let p = 0; p < cols.length; p++) {
      x += widthsCh[cols[p]] * pxPerCh;
      edges[p] = x;
    }
    return edges;
  });
  // Calibrate px-per-ch from the laid-out table (two layout reads, after the
  // DOM settles) whenever the widths or the font can change.
  $effect(() => {
    if (!hvirt) return;
    void widthsCh;
    void filterRow;
    const el = scrollEl;
    let live = true;
    void tick().then(() => {
      if (!live || !el) return;
      const table = el.querySelector<HTMLElement>('table');
      const rn = el.querySelector<HTMLElement>('thead th.rownum');
      if (!table || !rn) return;
      let sum = 0;
      for (const w of widthsCh) sum += w;
      const rnW = rn.offsetWidth;
      const per = sum > 0 ? (table.offsetWidth - rnW) / sum : 0;
      if (rnW > 0 && Math.abs(rnW - rnPx) >= 0.5) rnPx = rnW;
      if (per > 0 && Math.abs(per - pxPerCh) > 0.01) pxPerCh = per;
    });
    return () => {
      live = false;
    };
  });
  /** First display position whose right edge is past `x` (binary search). */
  function colAt(x: number): number {
    let lo = 0;
    let hi = colEdges.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (colEdges[mid] < x) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }
  /** [colStart, colEnd) — the display positions the body renders. */
  const colWindow = $derived.by<[number, number]>(() => {
    const n = cols.length;
    if (!hvirt || colEdges.length !== n || n === 0) return [0, n];
    const first = colAt(inlineScroll);
    const last = colAt(inlineScroll + Math.max(viewportW, 1));
    const start = Math.max(0, Math.floor((first - COL_OVERSCAN) / COL_CHUNK) * COL_CHUNK);
    const end = Math.min(n, Math.ceil((last + 1 + COL_OVERSCAN) / COL_CHUNK) * COL_CHUNK);
    return [start, end];
  });
  const colStart = $derived(colWindow[0]);
  const colEnd = $derived(colWindow[1]);
  /** Bring display column `pos` into the horizontal viewport (keyboard cursor). */
  function ensureColVisible(pos: number): void {
    if (!scrollEl || !hvirt) return;
    const right = colEdges[pos];
    if (right === undefined) return;
    const left = pos > 0 ? colEdges[pos - 1] : 0;
    const rn = (scrollEl.querySelector<HTMLElement>('thead th.rownum')?.offsetWidth ?? 0);
    const cur = Math.abs(scrollEl.scrollLeft);
    const sign = rtl ? -1 : 1;
    if (left - rn < cur) scrollEl.scrollLeft = sign * Math.max(0, left - rn);
    else if (right > cur + viewportW) scrollEl.scrollLeft = sign * (right - viewportW);
    scrollLeft = scrollEl.scrollLeft;
  }

  // ⌘F over every row (lib/findProviders.ts): the body mounts ~40 of 100k
  // rows, so find-in-page used to see ~0.05 % of a result. The provider
  // searches each row's DISPLAYED cell text (the same clip/preview the cells
  // render, NULL glyphs included) in column display order; the row-number
  // cell is `data-find-skip`. Mini grids mount every row — the DOM walk is
  // exact there, so the provider sits out.
  function findCellText(v: unknown): string {
    if (v === null || v === undefined) return 'NULL';
    return isComplex(v) ? clip(previewJson(v, CELL_MAX, expandJson)) : cellDisplay(v);
  }
  $effect(() =>
    registerFindProvider({
      root: () => scrollEl,
      active: () => virtualize,
      count: () => viewRows.length,
      text: (i) => {
        const r = viewRows[i];
        return r ? cols.map((ci) => findCellText(r.row[ci])).join('\t') : '';
      },
      reveal: async (i) => {
        if (!scrollEl) return;
        scrollEl.scrollTop = Math.max(0, i * ROW_H - (viewportH - HEAD_H) / 2);
        scrollTop = scrollEl.scrollTop;
        await tick();
      },
      rowElement: (i) => {
        const r = viewRows[i];
        return r ? (scrollEl?.querySelector(`tbody td[data-r="${r.idx}"]`)?.parentElement ?? null) : null;
      },
    }),
  );

  // ── Column widths ────────────────────────────────────────────────────────────
  // Auto-size each column from header (name OR type — they sit on two lines) +
  // cell content (sampling up to 200 rows), clamped to [MIN, MAX]. NULLs
  // contribute their 4-char label at most so they never widen a column.
  const MIN_CH = 6;
  const MAX_CH = 48;
  const WIDTH_SAMPLE = 200;

  /** Drag-overridden widths, keyed by column name; seeded from auto widths. */
  let colWidths = $state<Record<string, number>>({});

  function autoWidthCh(colIndex: number): number {
    if (!result) return MIN_CH;
    const col = result.columns[colIndex];
    // Name + sort indicator on line one; the type hint on line two (mini grids
    // show both on one line). PK badge room is reserved for EVERY column so the
    // badge landing after the editability probe can't reflow the header.
    let max = mini
      ? col.name.length + (col.type_hint ? col.type_hint.length + 2 : 0)
      : Math.max(col.name.length + 5, (col.type_hint ?? '').length + 1);
    const n = Math.min(liveRows.length, WIDTH_SAMPLE);
    for (let r = 0; r < n; r++) {
      const v = liveRows[r][colIndex];
      if (v === null || v === undefined) continue; // NULL must not widen
      // Never serialize a complex value just to MEASURE it — a Mongo document can
      // be ~90KB and the result is clamped to MAX_CH regardless. Sentinels measure
      // by their rendered form; scalars measure exactly.
      const b = bsonScalar(v);
      const len = b !== null ? b.length : isComplex(v) ? MAX_CH : String(v).length;
      if (len > max) max = len;
      if (max >= MAX_CH) break; // already clamped — nothing longer can change it
    }
    // +4 ch: the cell's 20px of horizontal padding (~3ch at 12px mono) plus a
    // little air, so a value that fits isn't ellipsized by its own padding.
    return Math.max(MIN_CH, Math.min(MAX_CH, max + 4));
  }

  const autoWidths = $derived.by<number[]>(() =>
    result ? result.columns.map((_c, i) => autoWidthCh(i)) : [],
  );

  function widthFor(colIndex: number): number {
    const name = result?.columns[colIndex]?.name ?? '';
    return colWidths[name] ?? autoWidths[colIndex] ?? MIN_CH;
  }
  /** Width (ch) per ORIGINAL column index, and the cell style per DISPLAY
   *  position — built once per width / order change, so a scroll step's new
   *  rows read an array instead of re-deriving 300 widths through the
   *  `colWidths` proxy each. */
  const widthsCh = $derived(result.columns.map((_c, ci) => widthFor(ci)));
  const widthStyles = $derived(cols.map((ci) => widthStyle(widthsCh[ci])));
  /** The display positions the header + filter row mount (all of them unless
   *  the column window is on). */
  const headCols = $derived(hvirt ? cols.slice(colStart, colEnd) : cols);

  // Pointer-drag resize on a header's right edge.
  let dragName = $state<string | null>(null);
  let dragStartX = 0;
  let dragStartCh = 0;
  const PX_PER_CH = 7.2; // approx for the monospace cell font at 12px

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
  /** Double-click the resize handle → back to the auto width. */
  function resetWidth(colIndex: number): void {
    const name = result?.columns[colIndex]?.name ?? '';
    const next = { ...colWidths };
    delete next[name];
    colWidths = next;
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
  // `c` the DISPLAY column. The scroll container owns focus + keydown; the
  // focused cell gets a ring. Arrows/Home/End/Page move, Enter edits (or expands
  // a complex / read-only cell), ⌘/Ctrl+C copies the cell, and ContextMenu /
  // Shift+F10 opens the row menu anchored to the cell.
  let focusCell = $state<{ r: number; c: number } | null>(null);

  function cellHit(e: Event): CellHit | null {
    const target = e.target instanceof Element ? e.target : null;
    const td = target?.closest<HTMLElement>('td[data-c]');
    if (!td) return null;
    return {
      idx: Number(td.dataset.r),
      ci: Number(td.dataset.c),
      pos: Number(td.dataset.p),
      vpos: rowPos(Number(td.dataset.r)),
      kind: td.dataset.k ?? 'p',
      expand: !!target?.closest('.cell-expand'),
    };
  }
  /** viewRows position of the mounted row showing liveRows[idx]. */
  function rowPos(idx: number): number {
    const w = windowRows.findIndex((e) => e.idx === idx);
    return w < 0 ? Number.NaN : startIdx + w;
  }
  function cellValue(h: CellHit): unknown {
    return liveRows[h.idx]?.[h.ci];
  }
  function onBodyClick(e: MouseEvent): void {
    const h = cellHit(e);
    if (!h) return;
    if (h.expand) {
      flow.openCell(cellValue(h), h.idx, h.ci);
      return;
    }
    focusCell = { r: h.vpos, c: h.pos };
    if (h.kind === 'j') flow.openCell(cellValue(h), h.idx, h.ci);
  }
  function onBodyDblClick(e: MouseEvent): void {
    const h = cellHit(e);
    if (!h || h.expand) return;
    if (h.kind === 'u') {
      flow.openCell(cellValue(h), h.idx, h.ci);
    } else if (h.kind === 'j') {
      flow.openCell(cellValue(h), h.idx, h.ci);
      flow.startViewerEdit();
    } else {
      flow.beginEdit(h.idx, h.ci);
    }
  }
  function onBodyContextMenu(e: MouseEvent): void {
    const h = cellHit(e);
    if (h) oncellmenu(e, h.ci, cellValue(h), h.idx);
  }
  /** Editability per ORIGINAL column index, derived once per result/flow
   *  change instead of re-asked by every rendered cell. */
  const editableCols = $derived(result.columns.map((_c, ci) => flow.isEditableCell(ci)));
  /** Skip the per-cell pending lookups entirely while nothing is parked. */
  const anyPending = $derived(flow.pending.size > 0);
  /** Sticky-header height the top of a row must clear to be visible. */
  const HEAD_H = $derived(filterRow ? 70 : 40);

  // Roving keyboard focus is positional — a new result invalidates it.
  $effect(() => {
    void result;
    focusCell = null;
  });
  // Report the row under the cursor (drives the row-detail panel).
  $effect(() => {
    const r = focusCell?.r;
    onfocusrow?.(r === undefined ? null : (viewRows[r]?.idx ?? null));
  });

  // ── Body cells (imperative, see grid-cells.ts) ─────────────────────────────
  // The cursor as a liveRows index, so a mounted row's cells never depend on
  // their window position (`startIdx`) — a scroll step leaves them alone.
  const focusIdx = $derived(focusCell ? (viewRows[focusCell.r]?.idx ?? null) : null);

  /** Fill a body row's data cells. Runs in the row's attachment effect: it
   *  re-runs (rebuilding that row) when anything the cells' markup reads
   *  changes — columns / order / kinds / editability, the open editor's
   *  position, parked drafts, the search needle, Expand-JSON. Widths and the
   *  keyboard ring are read untracked here and patched by `rowWidths` /
   *  the focus effect below, so a column drag or an arrow key rebuilds
   *  nothing. */
  function rowCells(row: unknown[], idx: number): Attachment<HTMLTableRowElement> {
    return (tr) => {
      const order = cols;
      const cs = colStart;
      const ce = colEnd;
      const columns = result.columns;
      const kindsNow = kinds;
      const editableNow = editableCols;
      const pendingOn = anyPending;
      const ed = flow.editing;
      const editCol = ed && ed.rowIdx === idx ? ed.colIdx : -1;
      const needle = filtering ? searchLc : null;
      const expand = expandJson;
      const pv = new Array<string | undefined>(order.length);
      const under = new Array<boolean>(order.length);
      for (let p = cs; p < ce; p++) {
        const ci = order[p];
        pv[p] = pendingOn ? flow.pendingValue(idx, ci) : undefined;
        under[p] = pendingOn && pv[p] === undefined ? flow.hasPendingUnder(idx, columns[ci].name) : false;
      }
      untrack(() => {
        // A horizontal step only moves the column window: when nothing else
        // the cells render from changed, keep the cells still in the window
        // where they are and build only the columns entering it (a 400 px
        // step swaps ~4 of ~20 columns instead of rebuilding every row).
        const sig: unknown[] = [row, order, columns, kindsNow, editableNow, editCol, editCol >= 0 ? ed : null, needle, expand];
        const prev = rowState.get(tr);
        const reuse = prev && !pendingOn && !prev.pending && sameSig(prev.sig, sig) ? prev.cells : null;
        const focusPos = focusIdx === idx ? (focusCell?.c ?? -1) : -1;
        const cells = new Map<number, HTMLTableCellElement>();
        const kept: HTMLTableCellElement[] = [];
        if (prev) {
          for (const pad of prev.pads) pad.remove();
          for (const [pos, td] of prev.cells) if (!reuse || pos < cs || pos >= ce) td.remove();
        }
        // The kept cells are one contiguous run of positions (two windows
        // intersect in one interval), already in display order in the row.
        if (reuse)
          for (let pos = cs; pos < ce; pos++) {
            const td = reuse.get(pos);
            if (td) {
              cells.set(pos, td);
              kept.push(td);
            }
          }
        const firstKept = kept.length ? Number(kept[0].dataset.p) : ce;
        const front = document.createDocumentFragment();
        const back = document.createDocumentFragment();
        const pads: HTMLTableCellElement[] = [];
        if (cs > 0) pads.push(front.appendChild(hpadCell(cs)));
        for (let pos = cs; pos < ce; pos++) {
          if (cells.has(pos)) continue;
          const ci = order[pos];
          const td =
            ci === editCol
              ? editorCell(ci, pos)
              : buildCell(row[ci], idx, ci, pos, {
                  kind: kindsNow[ci],
                  editable: editableNow[ci],
                  widthCh: widthsCh[ci],
                  pv: pv[pos],
                  pendingUnder: under[pos],
                  needle,
                  expandJson: expand,
                  focused: pos === focusPos,
                });
          cells.set(pos, td);
          (pos < firstKept ? front : back).append(td);
        }
        if (ce < order.length) pads.push(back.appendChild(hpadCell(order.length - ce)));
        tr.insertBefore(front, kept[0] ?? null);
        if (kept.length) kept[kept.length - 1].after(back);
        else tr.append(back);
        rowState.set(tr, { sig, cells, pads, pending: pendingOn });
      });
      // Cells stay for the next run to reuse; an unmounted row takes them
      // with it. A replaced row value (`row` in the signature) rebuilds.
    };
  }
  /** Per mounted row: the cells `rowCells` built last and what they were built from. */
  const rowState = new WeakMap<
    HTMLTableRowElement,
    { sig: unknown[]; cells: Map<number, HTMLTableCellElement>; pads: HTMLTableCellElement[]; pending: boolean }
  >();
  function sameSig(a: unknown[], b: unknown[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

  /** A spacer cell standing for `span` columns outside the column window. */
  function hpadCell(span: number): HTMLTableCellElement {
    const td = document.createElement('td');
    td.className = 'hpad';
    td.colSpan = span;
    td.setAttribute('aria-hidden', 'true');
    td.dataset.findSkip = '';
    return td;
  }

  /** The open inline editor (the old `bind:value` + `use:focusEditor` input). */
  function editorCell(ci: number, pos: number): HTMLTableCellElement {
    const td = document.createElement('td');
    td.className = 'cell editing';
    td.dataset.p = String(pos);
    td.setAttribute('style', widthStyle(widthFor(ci)));
    const input = document.createElement('input');
    input.className = 'cell-input mono';
    input.value = flow.editing?.value ?? '';
    input.addEventListener('input', () => {
      if (flow.editing) flow.editing.value = input.value;
    });
    input.addEventListener('keydown', (e) => flow.onEditKeydown(e));
    input.addEventListener('blur', () => flow.commitEdit());
    td.append(input);
    focusEditor(input);
    return td;
  }

  /** Keep a row's cell widths in step with auto widths / drag-resizes. */
  const rowWidths: Attachment<HTMLTableRowElement> = (tr) => {
    const styles = widthStyles;
    const tds = tr.cells;
    // cells[0] is the row-number column; data cells carry their display
    // position (the column window may start past 0, behind a spacer).
    for (let i = 1; i < tds.length; i++) {
      const td = tds[i];
      const p = td.dataset.p;
      if (p === undefined) continue;
      const style = styles[Number(p)];
      if (style !== undefined && td.getAttribute('style') !== style) td.setAttribute('style', style);
    }
  };

  // The grid's accessible active cell. Cells are plain DOM (grid-cells.ts), so
  // the one focused cell is given an id + an `aria-label` carrying its column
  // name and value, and the focusable grid frame points at it with
  // `aria-activedescendant` — a screen reader hears "id: 42", not a bare "42".
  const uid = $props.id();
  const activeCellDomId = `${uid}-active-cell`;
  let activeCellId = $state<string | undefined>(undefined);

  // Move the keyboard ring. Rows built later pick it up in `rowCells`; the
  // window is read so a rebuilt row/column re-applies the ring + the aria bits.
  $effect(() => {
    const r = focusIdx;
    const c = focusCell?.c;
    const el = scrollEl;
    void windowRows;
    void colStart;
    void colEnd;
    if (!el) return;
    for (const td of el.querySelectorAll('tbody td.kbd-focus')) {
      td.classList.remove('kbd-focus');
      if (td.id === activeCellDomId) {
        td.removeAttribute('id');
        td.removeAttribute('aria-label');
      }
    }
    activeCellId = undefined;
    if (r === null || c === undefined) return;
    const td = el.querySelector<HTMLTableCellElement>(`tbody td[data-r="${r}"][data-p="${c}"]`);
    if (!td) return;
    td.classList.add('kbd-focus');
    if (mini) return;
    const name = result.columns[cols[c]]?.name ?? '';
    const value = (td.textContent ?? '').trim().slice(0, 200);
    td.id = activeCellDomId;
    td.setAttribute('aria-colindex', String(c + 2)); // column 1 is the row number
    td.setAttribute('aria-label', name ? `${name}: ${value || 'empty'}` : value);
    activeCellId = activeCellDomId;
  });

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
    const ci = cols[focusCell.c];
    const rect = scrollEl?.querySelector('td.kbd-focus')?.getBoundingClientRect();
    const ev = new MouseEvent('contextmenu', {
      clientX: rect ? rect.left + Math.min(rect.width, 160) / 2 : 80,
      clientY: rect ? rect.bottom - 2 : 80,
    });
    oncellmenu(ev, ci, entry.row[ci], entry.idx);
  }

  function onGridKeydown(e: KeyboardEvent): void {
    // Inline editors consume Enter/Escape before the event reaches the grid.
    // Committing clears flow.editing synchronously; don't reopen that cell.
    if (e.defaultPrevented) return;
    if (mini || !result || flow.editing || flow.reviewSql || flow.viewer || flow.docEditor) return;
    // Typing in the filter row must not move the cell cursor.
    if ((e.target as HTMLElement | null)?.closest('.filter-row')) return;
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
      ensureColVisible(focusCell.c);
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
    const ci = cols[focusCell.c];
    const v = entry.row[ci];
    if (e.key === 'Enter') {
      e.preventDefault();
      if (flow.isEditableCell(ci) && !isComplex(v)) flow.beginEdit(entry.idx, ci);
      else flow.openCell(v, entry.idx, ci);
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'c') {
      // A real text selection keeps the native copy.
      if (window.getSelection()?.toString()) return;
      e.preventDefault();
      void copyWithToast(v === null || v === undefined ? '' : cellStr(v), ['Copied', 'Cell value copied']);
      return;
    }
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault();
      openFocusMenu();
      return;
    }
    if (e.key === 'Escape') focusCell = null;
  }
;
async () => {


  { const $$_div0 = svelteHTML.createElement("div", {                  "class":`grid-scroll`,...__sveltets_2_empty({"data-grid-frame":mini ? undefined : ''}),"onscroll":onScroll,"tabindex":mini ? undefined : 0,"role":mini ? undefined : 'group',"aria-activedescendant":activeCellId,"aria-label":mini
    ? undefined
    : 'Results grid — arrow keys move, Enter edits or expands, ⌘C copies the cell, Shift+F10 opens the row menu',"onkeydown":onGridKeydown,});mini;scrollEl = $$_div0;
   { svelteHTML.createElement("table", {              "class":`grid mono`,"role":mini ? undefined : 'grid',"aria-rowcount":mini ? undefined : liveRows.length + 1,"aria-colcount":mini ? undefined : result.columns.length + 1,"style":`--last:${result.columns.length}; --row-h:${ROW_H}px; --rn-w:calc(${rnCh}ch + 30px)`,});expandJson;mini;hvirt;
    if(hvirt){
      
       { svelteHTML.createElement("colgroup", {});
         { svelteHTML.createElement("col", {  "class":`rn-col`,});}
           for(let ci of __sveltets_2_ensureArray(cols)){ci; { svelteHTML.createElement("col", {  "style":`width:${widthsCh[ci]}ch`,});}}
       }
    }
     { svelteHTML.createElement("thead", {});
       { svelteHTML.createElement("tr", { "aria-rowindex":mini ? undefined : 1,});
         { svelteHTML.createElement("th", {   "class":`rownum`,"aria-colindex":mini ? undefined : 1,});
          
           { svelteHTML.createElement("span", { "class":`sel-slot`,});
            if(flow.editable){
               { svelteHTML.createElement("input", {             "class":`sel-box`,"type":`checkbox`,"checked":flow.allInViewSelected,"onchange":() => flow.toggleAllInView(viewRows.map((r) => r.idx)),"title":`Select all rows in view`,"aria-label":`Select all rows`,});}
            }
           }
           { svelteHTML.createElement("span", { "class":`rownum-n`,});  }
         }
        if(colStart > 0){ { svelteHTML.createElement("th", {     "class":`hpad`,"colspan":colStart,"aria-hidden":`true`,}); }}
            for(let ci of __sveltets_2_ensureArray(headCols)){let k = 1;ci;
          const pos = colStart + k;
          const c = result.columns[ci];
          const isPk = flow.editable && flow.editPkCols.includes(c.name);
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('th')));{ svelteHTML.createElement("th", __sveltets_2_union($$action_0), {                      "title":mini ? (c.type_hint ?? undefined) : `${c.name}${c.type_hint ? ` · ${c.type_hint}` : ''} — click to sort, drag to reorder, right-click for more`,"aria-sort":sortCol === ci ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none',"aria-colindex":mini ? undefined : pos + 2,"style":`width:${widthFor(ci)}ch; max-width:${widthFor(ci)}ch;`,"oncontextmenu":(e) => onheadermenu(e, ci),"ondragover":(e) => onHeadDragOver(e, pos),"ondrop":(e) => onHeadDrop(e, pos),});isPk;!mini;sortCol === ci;kinds[ci] === 'num';dropAt === pos && dragFrom !== null && dragFrom > pos;dropAt === pos && dragFrom !== null && dragFrom < pos;
            if(mini){
               { svelteHTML.createElement("span", { "class":`th-inner one-line`,});
                 { svelteHTML.createElement("span", { "class":`th-name`,});c.name; }
                if(c.type_hint){ { svelteHTML.createElement("span", { "class":`th-type`,});c.type_hint; }}
               }
            }else{
               { svelteHTML.createElement("button", {             "class":`th-sort`,"type":`button`,"draggable":`true`,"ondragstart":(e) => onHeadDragStart(e, pos),"ondragend":onHeadDragEnd,"onclick":() => oncyclesort(ci),});
                 { svelteHTML.createElement("span", { "class":`th-inner`,});
                   { svelteHTML.createElement("span", { "class":`th-line`,});
                     { svelteHTML.createElement("span", { "class":`th-name`,});c.name; }
                    if(isPk){ { svelteHTML.createElement("span", {   "class":`th-pk`,"title":`Primary key (read-only)`,});  }}
                     { svelteHTML.createElement("span", {     "class":`th-sort-ind`,"aria-hidden":`true`,});sortCol === ci;sortCol === ci ? (sortDir === 'asc' ? '▲' : '▼') : '↕'; }
                   }
                   { svelteHTML.createElement("span", { "class":`th-type`,});c.type_hint ?? ' '; }
                 }
               }
              
               { svelteHTML.createElement("span", {                  "role":`presentation`,"class":`th-resize`,"title":`Drag to resize · double-click to fit`,"onpointerdown":(e) => startResize(e, ci),"onpointermove":onResizeMove,"onpointerup":endResize,"onpointercancel":endResize,"ondblclick":() => resetWidth(ci),});dragName === c.name; }
            }
           }}
        }
        if(colEnd < cols.length){ { svelteHTML.createElement("th", {     "class":`hpad`,"colspan":cols.length - colEnd,"aria-hidden":`true`,}); }}
       }
      if(filterRow && !mini){
        
         { svelteHTML.createElement("tr", { "class":`filter-row`,});
           { svelteHTML.createElement("td", { "class":`rownum`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`filter`,"size":12,}});} }
          if(colStart > 0){ { svelteHTML.createElement("td", {     "class":`hpad`,"colspan":colStart,"aria-hidden":`true`,}); }}
             for(let ci of __sveltets_2_ensureArray(headCols)){ci;
            const c = result.columns[ci];
             { svelteHTML.createElement("td", { "style":`width:${widthFor(ci)}ch; max-width:${widthFor(ci)}ch;`,});
               { svelteHTML.createElement("input", {                        "dir":`ltr`,"class":`col-filter mono`,"type":`text`,"spellcheck":`false`,"autocomplete":`off`,"placeholder":`Filter…`,"value":colFilters[ci] ?? '',"aria-label":`Filter ${c.name}`,"title":`Contains · =exact · >n <n · NULL · !NULL`,"oninput":(e) => oncolfilter?.(ci, e.currentTarget.value),"onfocus":() => oncolfocus?.(ci),"onblur":() => oncolfocus?.(null),});}
             }
          }
          if(colEnd < cols.length){ { svelteHTML.createElement("td", {     "class":`hpad`,"colspan":cols.length - colEnd,"aria-hidden":`true`,}); }}
         }
      }
     }
    
     { svelteHTML.createElement("tbody", {     "onclick":onBodyClick,"ondblclick":onBodyDblClick,"oncontextmenu":onBodyContextMenu,});
      if(padTop > 0){
         { svelteHTML.createElement("tr", {   "class":`spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":result.columns.length + 1,"style":`height:${padTop}px`,}); } }
      }
         for(let { row, idx } of __sveltets_2_ensureArray(windowRows)){idx;
        
         { svelteHTML.createElement("tr", {        "aria-rowindex":mini ? undefined : idx + 2,[Symbol("@attach")]:rowCells(row, idx),[Symbol("@attach")]:rowWidths,});idx % 2 === 1;flow.selected.has(idx);focusIdx === idx;
           { svelteHTML.createElement("td", {   "class":`rownum`,...__sveltets_2_empty({"data-find-skip":true}),"aria-colindex":mini ? undefined : 1,});
             { svelteHTML.createElement("span", { "class":`sel-slot`,});
              if(flow.editable){
                 { svelteHTML.createElement("input", {             "class":`sel-box`,"type":`checkbox`,"checked":flow.selected.has(idx),"onclick":(e) => flow.toggleRow(idx, e),"title":`Select row (shift-click for a range)`,"aria-label":`Select row ${idx + 1}`,});}
              }
             }
             { svelteHTML.createElement("span", { "class":`rownum-n`,});idx + 1; }
            
            if(flow.editable && flow.engine !== 'redis'){
               { svelteHTML.createElement("button", {         "class":`row-dup`,"title":`Duplicate row (review INSERT before running)`,"aria-label":`Duplicate row`,"onclick":() => flow.duplicateRow(idx),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
               }
            }
           }
         }
      }
      if(padBottom > 0){
         { svelteHTML.createElement("tr", {   "class":`spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":result.columns.length + 1,"style":`height:${padBottom}px`,}); } }
      }
     }
   }
  if(!mini && viewRows.length === 0 && liveRows.length > 0){
     { svelteHTML.createElement("div", { "class":`grid-nomatch`,});      }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GridView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GridView__SvelteComponent_ = ReturnType<typeof GridView__SvelteComponent_>;
/*Ωignore_endΩ*/export default GridView__SvelteComponent_;