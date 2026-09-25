<script lang="ts">
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
  import { tick } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { bsonScalar } from './bson';
  import type { QueryResult } from '../../lib/api/types';
  import { SET_EMPTY, SET_NULL, type EditFlow } from './EditFlow.svelte';
  import { cellDisplay, cellStr, clip, compactJson, copyText, isComplex, prettyJson } from './results-format';
  import { columnKind, moveColumn, rowNumberWidthCh, type ColumnKind } from './grid-format';

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
    /** The keyboard/click cursor moved onto a row (its liveRows index). */
    onfocusrow?: (idx: number | null) => void;
    oncellmenu: (e: MouseEvent, ci: number, v: unknown, rowIdx: number) => void;
    onheadermenu: (e: MouseEvent, ci: number) => void;
    oncyclesort: (ci: number) => void;
  }
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

  // Preserve column widths / order / scroll when the new result has the SAME
  // columns (a re-run of the same query), so the grid doesn't jump; reset them
  // only when the shape actually changes — `resetToken` is that signature.
  $effect(() => {
    void resetToken;
    scrollTop = 0;
    if (scrollEl) scrollEl.scrollTop = 0;
    colWidths = {};
    dragName = null;
    order = result.columns.map((_c, i) => i);
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

  function onScroll(): void {
    if (scrollEl) scrollTop = scrollEl.scrollTop;
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
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex a11y_no_noninteractive_element_interactions a11y_no_static_element_interactions -->
<div
  class="grid-scroll"
  class:mini
  data-grid-frame={mini ? undefined : ''}
  bind:this={scrollEl}
  onscroll={onScroll}
  tabindex={mini ? undefined : 0}
  role={mini ? undefined : 'group'}
  aria-label={mini
    ? undefined
    : 'Results grid — arrow keys move, Enter edits or expands, ⌘C copies the cell, Shift+F10 opens the row menu'}
  onkeydown={onGridKeydown}
>
  <table
    class="grid mono"
    class:expanded={expandJson}
    class:mini
    style="--last:{result.columns.length}; --row-h:{ROW_H}px; --rn-w:calc({rnCh}ch + 30px)"
  >
    <thead>
      <tr>
        <th class="rownum">
          <!-- The checkbox slot is ALWAYS reserved (see the layout-stability note). -->
          <span class="sel-slot">
            {#if flow.editable}
              <input
                class="sel-box"
                type="checkbox"
                checked={flow.allInViewSelected}
                onchange={() => flow.toggleAllInView(viewRows.map((r) => r.idx))}
                title="Select all rows in view"
                aria-label="Select all rows"
              />
            {/if}
          </span>
          <span class="rownum-n">#</span>
        </th>
        {#each cols as ci, pos (ci)}
          {@const c = result.columns[ci]}
          {@const isPk = flow.editable && flow.editPkCols.includes(c.name)}
          <th
            title={mini ? (c.type_hint ?? undefined) : `${c.name}${c.type_hint ? ` · ${c.type_hint}` : ''} — click to sort, drag to reorder, right-click for more`}
            class:pk={isPk}
            class:sortable={!mini}
            class:sorted={sortCol === ci}
            class:num={kinds[ci] === 'num'}
            class:drop-before={dropAt === pos && dragFrom !== null && dragFrom > pos}
            class:drop-after={dropAt === pos && dragFrom !== null && dragFrom < pos}
            aria-sort={sortCol === ci ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'}
            style="width:{widthFor(ci)}ch; max-width:{widthFor(ci)}ch;"
            oncontextmenu={(e) => onheadermenu(e, ci)}
            ondragover={(e) => onHeadDragOver(e, pos)}
            ondrop={(e) => onHeadDrop(e, pos)}
          >
            {#if mini}
              <span class="th-inner one-line">
                <span class="th-name">{c.name}</span>
                {#if c.type_hint}<span class="th-type">{c.type_hint}</span>{/if}
              </span>
            {:else}
              <button
                class="th-sort"
                type="button"
                draggable="true"
                ondragstart={(e) => onHeadDragStart(e, pos)}
                ondragend={onHeadDragEnd}
                onclick={() => oncyclesort(ci)}
              >
                <span class="th-inner">
                  <span class="th-line">
                    <span class="th-name">{c.name}</span>
                    {#if isPk}<span class="th-pk" title="Primary key (read-only)">PK</span>{/if}
                    <span class="th-sort-ind" class:on={sortCol === ci} aria-hidden="true"
                      >{sortCol === ci ? (sortDir === 'asc' ? '▲' : '▼') : '↕'}</span
                    >
                  </span>
                  <span class="th-type">{c.type_hint ?? ' '}</span>
                </span>
              </button>
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <span
                class="th-resize"
                class:active={dragName === c.name}
                title="Drag to resize · double-click to fit"
                onpointerdown={(e) => startResize(e, ci)}
                onpointermove={onResizeMove}
                onpointerup={endResize}
                onpointercancel={endResize}
                ondblclick={() => resetWidth(ci)}
              ></span>
            {/if}
          </th>
        {/each}
      </tr>
      {#if filterRow && !mini}
        <!-- Per-column filter (client-side, over the loaded rows). `td`, not
             `th`, so header counts stay one per column. -->
        <tr class="filter-row">
          <td class="rownum"><Icon name="filter" size={12} /></td>
          {#each cols as ci (ci)}
            {@const c = result.columns[ci]}
            <td style="width:{widthFor(ci)}ch; max-width:{widthFor(ci)}ch;">
              <input
                class="col-filter mono"
                type="text"
                spellcheck="false"
                autocomplete="off"
                placeholder="filter"
                value={colFilters[ci] ?? ''}
                aria-label="Filter {c.name}"
                title="Contains · =exact · >n <n · NULL · !NULL"
                oninput={(e) => oncolfilter?.(ci, e.currentTarget.value)}
              />
            </td>
          {/each}
        </tr>
      {/if}
    </thead>
    <tbody>
      {#if padTop > 0}
        <tr class="spacer" aria-hidden="true"><td colspan={result.columns.length + 1} style="height:{padTop}px"></td></tr>
      {/if}
      {#each windowRows as { row, idx }, wi (idx)}
        {@const vpos = startIdx + wi}
        <tr class:odd={idx % 2 === 1} class:selected={flow.selected.has(idx)} class:cursor={focusCell?.r === vpos}>
          <td class="rownum">
            <span class="sel-slot">
              {#if flow.editable}
                <input
                  class="sel-box"
                  type="checkbox"
                  checked={flow.selected.has(idx)}
                  onclick={(e) => flow.toggleRow(idx, e)}
                  title="Select row (shift-click for a range)"
                  aria-label="Select row {idx + 1}"
                />
              {/if}
            </span>
            <span class="rownum-n">{idx + 1}</span>
            <!-- Redis has no insert builder (a "row" is a key), so no duplicate. -->
            {#if flow.editable && flow.engine !== 'redis'}
              <button
                class="row-dup"
                title="Duplicate row (review INSERT before running)"
                aria-label="Duplicate row"
                onclick={() => flow.duplicateRow(idx)}
              >
                <Icon name="plus" size={12} />
              </button>
            {/if}
          </td>
          {#each cols as ci, pos (ci)}
            {@const _c = result.columns[ci]}
            {@const v = row[ci]}
            {@const w = widthFor(ci)}
            {@const kind = kinds[ci]}
            {#if flow.editing && flow.editing.rowIdx === idx && flow.editing.colIdx === ci}
              <td class="cell editing" style="width:{w}ch; max-width:{w}ch;">
                <!-- svelte-ignore a11y_autofocus -->
                <input
                  class="cell-input mono"
                  bind:value={flow.editing.value}
                  use:focusEditor
                  onkeydown={(e) => flow.onEditKeydown(e)}
                  onblur={() => flow.commitEdit()}
                />
              </td>
            {:else if flow.pendingValue(idx, ci) !== undefined}
              {@const pv = flow.pendingValue(idx, ci) ?? ''}
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <td
                class="cell dirty"
                class:num={kind === 'num'}
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === pos}
                title="Pending change — Review & apply (bar below) writes it; double-click to keep editing"
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => (focusCell = { r: vpos, c: pos })}
                ondblclick={() => flow.beginEdit(idx, ci)}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
              >{#if pv === '' || pv === SET_NULL}<span class="null-glyph">NULL</span>{:else if pv === SET_EMPTY}<span class="null-glyph">''</span>{:else}{pv}{/if}</td>
            {:else if flow.hasPendingUnder(idx, _c.name)}
              <!-- A path-level change (Vertical view: $set/$unset/$rename inside
                   this document field) — the cell keeps showing the stored value
                   but wears the dirty marker so it isn't edited over blindly. -->
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <td
                class="cell dirty"
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === pos}
                title="Nested change pending — Review & apply (bar below) writes it; see the Vertical view"
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => (focusCell = { r: vpos, c: pos })}
                ondblclick={() => flow.openCell(v, idx, ci)}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
              >{v === null || v === undefined ? '' : clip(cellStr(v))}</td>
            {:else if v === null || v === undefined}
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <td
                class="cell null"
                class:num={kind === 'num'}
                class:editable={flow.isEditableCell(ci)}
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === pos}
                title="NULL"
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => (focusCell = { r: vpos, c: pos })}
                ondblclick={() => flow.beginEdit(idx, ci)}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
              ><span class="null-glyph">NULL</span></td>
            {:else if isComplex(v)}
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <td
                class="cell json"
                class:wrap={expandJson}
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === pos}
                title="Click to expand"
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => {
                  focusCell = { r: vpos, c: pos };
                  flow.openCell(v, idx, ci);
                }}
                ondblclick={() => { flow.openCell(v, idx, ci); flow.startViewerEdit(); }}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
              >{clip(expandJson ? prettyJson(v) : compactJson(v))}<button class="cell-expand" title="Expand value" aria-label="Expand value" onclick={(e) => { e.stopPropagation(); flow.openCell(v, idx, ci); }}><Icon name="maximize" size={9} /></button></td>
            {:else}
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <td
                class="cell"
                class:num={kind === 'num'}
                class:bool={kind === 'bool'}
                class:editable={flow.isEditableCell(ci)}
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === pos}
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => (focusCell = { r: vpos, c: pos })}
                ondblclick={() => flow.beginEdit(idx, ci)}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
              >{#if filtering}{#each highlightParts(cellDisplay(v)) as part}{#if part.hit}<mark>{part.t}</mark>{:else}{part.t}{/if}{/each}{:else}{cellDisplay(v)}{/if}<button class="cell-expand" title="Expand value" aria-label="Expand value" onclick={(e) => { e.stopPropagation(); flow.openCell(v, idx, ci); }}><Icon name="maximize" size={9} /></button></td>
            {/if}
          {/each}
        </tr>
      {/each}
      {#if padBottom > 0}
        <tr class="spacer" aria-hidden="true"><td colspan={result.columns.length + 1} style="height:{padBottom}px"></td></tr>
      {/if}
    </tbody>
  </table>
  {#if !mini && viewRows.length === 0 && liveRows.length > 0}
    <div class="grid-nomatch">No rows match the current filters.</div>
  {/if}
</div>

<style>
  .grid-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    /* Reserve the scrollbar gutter up front: a result that grows past the
       viewport must not narrow the grid by a scrollbar's width mid-render. */
    scrollbar-gutter: stable;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    position: relative;
  }
  .grid-scroll.mini {
    scrollbar-gutter: auto;
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
    border-collapse: separate;
    border-spacing: 0;
    table-layout: fixed;
    width: max-content;
    min-width: 100%;
    user-select: text;
  }
  .grid thead th {
    position: sticky;
    top: 0;
    z-index: 2;
    height: 40px;
    box-sizing: border-box;
    text-align: start;
    padding: 4px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    border-inline-end: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    font-size: var(--fs-s);
    white-space: nowrap;
    vertical-align: middle;
    overflow: hidden;
  }
  .grid.mini thead th {
    height: 28px;
  }
  /* When sortable, the header content lives in a button that fills the cell. */
  .grid thead th.sortable {
    padding: 0;
  }
  .grid thead th.sorted {
    background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
  }
  .grid thead th.drop-before {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .grid thead th.drop-after {
    box-shadow: inset -2px 0 0 var(--accent);
  }
  .th-sort {
    display: flex;
    align-items: center;
    width: 100%;
    height: 100%;
    /* leave a sliver on the right for the resize handle */
    padding: 3px 12px 3px 10px;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .th-sort:hover {
    background: var(--hover);
  }
  .th-sort:focus-visible {
    outline: 1.5px solid var(--accent);
    outline-offset: -2px;
  }
  .th-inner {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    width: 100%;
  }
  .th-inner.one-line {
    flex-direction: row;
    align-items: baseline;
    gap: 6px;
  }
  .th-line {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
  }
  th.num .th-line {
    justify-content: flex-end;
  }
  th.num .th-type {
    text-align: end;
  }
  .th-name {
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .th-sort-ind {
    flex: 0 0 auto;
    font-size: var(--fs-xs);
    line-height: 1;
    color: var(--text-dim);
    opacity: 0;
    transition: opacity 120ms ease-out;
  }
  .th-sort:hover .th-sort-ind {
    opacity: 0.6;
  }
  .th-sort-ind.on {
    opacity: 1;
    color: var(--accent-text);
  }
  .th-pk {
    flex: 0 0 auto;
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 14px;
    padding: 0 4px;
    border-radius: var(--radius-s);
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .th-type {
    font-weight: 400;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Drag handle on the header's right edge. */
  .th-resize {
    position: absolute;
    top: 0;
    inset-inline-end: -3px;
    width: 7px;
    height: 100%;
    cursor: col-resize;
    z-index: 4;
    touch-action: none;
  }
  .th-resize::after {
    content: '';
    position: absolute;
    top: 6px;
    bottom: 6px;
    left: 3px;
    width: 1px;
    background: transparent;
  }
  .th-resize:hover::after,
  .th-resize.active::after {
    background: var(--accent);
  }
  /* ── Filter row (sticky under the header) ── */
  .filter-row td {
    position: sticky;
    top: 40px;
    z-index: 2;
    height: 30px;
    box-sizing: border-box;
    padding: 3px 6px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    border-inline-end: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
  }
  .filter-row td.rownum {
    z-index: 3;
    text-align: center;
    color: var(--text-dim);
  }
  .col-filter {
    width: 100%;
    height: 22px;
    box-sizing: border-box;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
  }
  .col-filter::placeholder {
    color: var(--text-dim);
  }
  .col-filter:focus {
    outline: none;
    border-color: var(--accent);
  }
  .grid td {
    padding: 4px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    border-inline-end: 1px solid color-mix(in srgb, var(--border) 45%, transparent);
    font-size: var(--fs-s);
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
  /* Numbers: right-aligned, tabular figures so digits line up. */
  .grid td.num {
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  /* Stripe by data-row index (not :nth-child) so the pattern stays stable as
     the virtualized window scrolls. */
  .grid tbody tr.odd td {
    background: color-mix(in srgb, var(--text) 2.5%, var(--surface));
  }
  .grid tbody tr:not(.spacer):hover td {
    background: var(--hover);
  }
  .grid tbody tr.cursor td {
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
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
    font-size: var(--fs-xs);
    position: sticky;
    inset-inline-start: 0;
    background: var(--surface-2);
    z-index: 1;
    /* Constant width — see the layout-stability note in the script. */
    width: var(--rn-w);
    min-width: var(--rn-w);
    max-width: var(--rn-w);
    box-sizing: border-box;
    padding-inline: 6px 8px !important;
    font-variant-numeric: tabular-nums;
  }
  .grid tbody tr.odd td.rownum,
  .grid tbody tr:not(.spacer):hover td.rownum {
    background: var(--surface-2);
  }
  .grid thead .rownum {
    z-index: 3;
  }
  .grid.mini .rownum {
    --rn-w: 5ch;
  }
  .grid.mini .sel-slot {
    display: none;
  }
  .sel-slot {
    display: inline-flex;
    align-items: center;
    float: inline-start;
    width: 16px;
    height: 17px;
  }
  .rownum-n {
    display: inline-block;
  }
  /* Per-row duplicate action: revealed on row hover, anchored to the trailing
   * edge of the # cell so it never covers the selection checkbox. */
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
    color: var(--accent-text);
    cursor: pointer;
    padding: 0;
  }
  .grid tbody tr:hover .row-dup {
    display: flex;
  }
  .row-dup:hover {
    background: color-mix(in srgb, var(--accent) 26%, var(--surface-2));
  }
  .sel-box {
    width: 12px;
    height: 12px;
    margin: 0;
    cursor: pointer;
    accent-color: var(--accent);
  }
  .grid tbody tr.selected td {
    background: var(--accent-soft);
  }
  .grid tbody tr.selected:not(.spacer):hover td {
    background: color-mix(in srgb, var(--accent) 22%, var(--surface));
  }
  /* NULL: a dim italic word, never an empty cell that reads as a bug. */
  .null-glyph {
    color: var(--text-dim);
    font-style: italic;
    font-size: var(--fs-xs);
    letter-spacing: 0.02em;
  }
  .cell.bool {
    color: var(--text);
  }
  .cell.json {
    color: var(--accent-text);
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
    top: 4px;
    inset-inline-end: 2px;
    display: none;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
    box-shadow: -3px 0 5px var(--surface);
  }
  .grid td.cell:hover .cell-expand {
    display: inline-flex;
  }
  .cell-expand:hover {
    color: var(--accent-text);
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
    background: var(--warning-soft) !important;
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warning) 55%, transparent);
    font-style: italic;
    cursor: default;
  }
  .cell-input {
    width: 100%;
    height: 100%;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
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
  .grid-nomatch {
    position: sticky;
    inset-inline-start: 0;
    padding: 18px 16px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     Make sure the grid fills its bounded block and scrolls in BOTH directions
     on touch (the toolbar wrapping lives with ResultsGrid). */
  @media (max-width: 640px) {
    .grid-scroll {
      -webkit-overflow-scrolling: touch;
    }
    .grid thead th {
      font-size: var(--fs-m);
    }
    .grid td {
      font-size: var(--fs-m);
    }
  }
</style>
