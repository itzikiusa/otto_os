<script lang="ts">
  // Virtualized results table: sticky header, monospace cells, NULL as a
  // dimmed ∅, objects/arrays shown as compact JSON with a click-to-expand cell
  // viewer. Columns auto-size to their content and are drag-resizable. Sort /
  // search / quick-filter state is owned by ResultsGrid and arrives as props;
  // the header + cell context menus are callbacks; every editing concern
  // (drafts, selection, viewer) lives on `flow`.
  import { tick } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { bsonScalar } from './bson';
  import type { QueryResult } from '../../lib/api/types';
  import { SET_EMPTY, SET_NULL, type EditFlow } from './EditFlow.svelte';
  import { cellDisplay, cellStr, clip, compactJson, copyText, isComplex, prettyJson } from './results-format';

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
    /** Column-name signature of the result — widths + scroll reset when it changes. */
    resetToken: string;
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

  // Preserve column widths / scroll when the new result has the SAME columns
  // (a re-run of the same query), so the grid doesn't jump; reset them only
  // when the shape actually changes — `resetToken` is that signature.
  $effect(() => {
    void resetToken;
    scrollTop = 0;
    if (scrollEl) scrollEl.scrollTop = 0;
    colWidths = {};
    dragName = null;
  });

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

  // Roving keyboard focus is positional — a new result invalidates it.
  $effect(() => {
    void result;
    focusCell = null;
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
    const rect = scrollEl?.querySelector('td.kbd-focus')?.getBoundingClientRect();
    const ev = new MouseEvent('contextmenu', {
      clientX: rect ? rect.left + Math.min(rect.width, 160) / 2 : 80,
      clientY: rect ? rect.bottom - 2 : 80,
    });
    oncellmenu(ev, focusCell.c, entry.row[focusCell.c], entry.idx);
  }

  function onGridKeydown(e: KeyboardEvent): void {
    if (mini || !result || flow.editing || flow.reviewSql || flow.viewer || flow.docEditor) return;
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
      if (flow.isEditableCell(focusCell.c) && !isComplex(v)) flow.beginEdit(entry.idx, focusCell.c);
      else flow.openCell(v, entry.idx, focusCell.c);
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
          {#if flow.editable}
            <input
              class="sel-box"
              type="checkbox"
              checked={flow.allInViewSelected}
              onchange={() => flow.toggleAllInView(viewRows.map((r) => r.idx))}
              title="Select all rows in view"
              aria-label="Select all rows"
            />
          {:else}#{/if}
        </th>
        {#each result.columns as c, ci (ci)}
          <th
            title={mini ? (c.type_hint ?? undefined) : `${c.name} — click to sort, right-click for filters`}
            class:pk={flow.editable && flow.editPkCols.includes(c.name)}
            class:sortable={!mini}
            class:sorted={sortCol === ci}
            aria-sort={sortCol === ci ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'}
            style="width:{widthFor(ci)}ch; max-width:{widthFor(ci)}ch;"
            oncontextmenu={(e) => onheadermenu(e, ci)}
          >
            {#if mini}
              <span class="th-inner">
                <span class="th-name">{c.name}</span>
                {#if c.type_hint}<span class="th-type">{c.type_hint}</span>{/if}
              </span>
            {:else}
              <button class="th-sort" type="button" onclick={() => oncyclesort(ci)}>
                <span class="th-inner">
                  <span class="th-name">{c.name}</span>
                  {#if flow.editable && flow.editPkCols.includes(c.name)}<span class="th-pk" title="Primary key (read-only)">PK</span>{/if}
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
        <tr class:odd={idx % 2 === 1} class:selected={flow.selected.has(idx)}>
          <td class="rownum">
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
            <span class="rownum-n">{idx + 1}</span>
            {#if flow.editable}
              <button
                class="row-dup"
                title="Duplicate row (review INSERT before running)"
                aria-label="Duplicate row"
                onclick={() => flow.duplicateRow(idx)}
              >
                <Icon name="plus" size={11} />
              </button>
            {/if}
          </td>
          {#each result.columns as _c, ci (ci)}
            {@const v = row[ci]}
            {@const w = widthFor(ci)}
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
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === ci}
                title="Pending change — Review & apply (bar below) writes it; double-click to keep editing"
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => (focusCell = { r: vpos, c: ci })}
                ondblclick={() => flow.beginEdit(idx, ci)}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
              >{#if pv === '' || pv === SET_NULL}<span class="null-glyph">∅</span>{:else if pv === SET_EMPTY}<span class="null-glyph">''</span>{:else}{pv}{/if}</td>
            {:else if v === null || v === undefined}
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <td
                class="cell null"
                class:editable={flow.isEditableCell(ci)}
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === ci}
                title="NULL"
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => (focusCell = { r: vpos, c: ci })}
                ondblclick={() => flow.beginEdit(idx, ci)}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
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
                  flow.openCell(v, idx, ci);
                }}
                ondblclick={() => { flow.openCell(v, idx, ci); flow.startViewerEdit(); }}
                oncontextmenu={(e) => oncellmenu(e, ci, v, idx)}
              >{clip(expandJson ? prettyJson(v) : compactJson(v))}<button class="cell-expand" title="Expand value" aria-label="Expand value" onclick={(e) => { e.stopPropagation(); flow.openCell(v, idx, ci); }}><Icon name="maximize" size={9} /></button></td>
            {:else}
              <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
              <td
                class="cell"
                class:editable={flow.isEditableCell(ci)}
                class:kbd-focus={focusCell?.r === vpos && focusCell?.c === ci}
                style="width:{w}ch; max-width:{w}ch;"
                onclick={() => (focusCell = { r: vpos, c: ci })}
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
</div>

<style>
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

  /* ───────────────── Phone (≤640px) ─────────────────
     Make sure the grid fills its bounded block and scrolls in BOTH directions
     on touch (the toolbar wrapping lives with ResultsGrid). */
  @media (max-width: 640px) {
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
  }
</style>
