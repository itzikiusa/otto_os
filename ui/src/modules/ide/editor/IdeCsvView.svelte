<script lang="ts">
  // DataGrip-grade Virtualized CSV / TSV / JSONL Table Grid:
  // Quote-aware RFC-4180 multiline record parsing, preference-driven delimiter,
  // first-row header & rainbow column coloring, ~36 rendered viewport rows even
  // for 100k-row CSVs, sticky header, sort/filter, inline cell editing with
  // undo/redo, add/delete row & column, and live column aggregate stats.
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import {
    parseCsvOrJsonlTable,
    resolveCsvDelimiter,
    type CsvDelimiter,
  } from '../../../lib/components/cm-ide-csv';
  import { idePreferences } from '../../../lib/idePreferences.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { rowMenu } from '../../../lib/rowMenu';

  interface Props {
    path: string;
    content: string;
    readOnly?: boolean;
    isJsonl?: boolean;
    onchange?: (nextContent: string) => void;
    onswitchtext?: (line: number) => void;
  }

  let { path, content, readOnly = false, isJsonl = false, onchange, onswitchtext }: Props = $props();

  const ROW_HEIGHT = 28;
  const OVERSCAN = 8;
  const MAX_UNDO = 24;

  let filterQuery = $state('');
  let stickyHeader = $state(true);
  let sortCol = $state<number | null>(null);
  let sortDir = $state<'asc' | 'desc'>('asc');
  let selectedCol = $state(0);
  let selectedRow = $state(0);
  let editingCell = $state<{ rowIdx: number; colIdx: number; draft: string } | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(420);
  let rootEl = $state<HTMLDivElement | undefined>(undefined);
  let scrollerEl = $state<HTMLDivElement | undefined>(undefined);

  let undoStack = $state<string[]>([]);
  let redoStack = $state<string[]>([]);

  const delimiter = $derived<CsvDelimiter>(
    resolveCsvDelimiter(content.slice(0, 8192), path, idePreferences.current.csvDelimiter),
  );
  const firstRowHeader = $derived(idePreferences.current.csvFirstRowHeader);
  const rainbowColumns = $derived(idePreferences.current.csvRainbowColumns);
  const delimiterLabel = $derived(
    isJsonl
      ? 'JSONL'
      : delimiter === '\t'
        ? 'TSV (Tab)'
        : delimiter === ';'
          ? 'CSV (;)'
          : delimiter === '|'
            ? 'CSV (|)'
            : 'CSV (,)',
  );

  const table = $derived(
    parseCsvOrJsonlTable(content, { isJsonl, delimiter, firstRowHeader }),
  );

  // Filtered + sorted row index mapping into `table.rows` (zero full-string copies).
  const visibleRowIndices = $derived.by((): number[] => {
    const { rows } = table;
    const q = filterQuery.trim().toLowerCase();
    const indices: number[] = [];
    for (let i = 0; i < rows.length; i++) {
      if (!q || rows[i].some((cell) => cell.toLowerCase().includes(q))) indices.push(i);
    }
    if (sortCol !== null) {
      const col = sortCol;
      const mult = sortDir === 'asc' ? 1 : -1;
      indices.sort((a, b) => {
        const va = rows[a]?.[col] ?? '';
        const vb = rows[b]?.[col] ?? '';
        const na = Number(va);
        const nb = Number(vb);
        if (va !== '' && vb !== '' && Number.isFinite(na) && Number.isFinite(nb)) return (na - nb) * mult;
        return va.localeCompare(vb) * mult;
      });
    }
    return indices;
  });

  const totalRows = $derived(visibleRowIndices.length);
  const startOffsetIdx = $derived(Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN));
  const endOffsetIdx = $derived(
    Math.min(totalRows, Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN),
  );
  const virtualSlice = $derived(visibleRowIndices.slice(startOffsetIdx, endOffsetIdx));
  const topPadPx = $derived(startOffsetIdx * ROW_HEIGHT);
  const bottomPadPx = $derived(Math.max(0, (totalRows - endOffsetIdx) * ROW_HEIGHT));

  // Column aggregate statistics for active column (capped scan at 50k rows -> <2ms).
  const colStats = $derived.by(() => {
    const col = Math.min(selectedCol, Math.max(0, table.headers.length - 1));
    const colName = table.headers[col] ?? `col_${col + 1}`;
    const scanCap = Math.min(visibleRowIndices.length, 50_000);
    const uniq = new Set<string>();
    let nonEmpty = 0;
    let numCount = 0;
    let sum = 0;
    let min = Infinity;
    let max = -Infinity;
    for (let i = 0; i < scanCap; i++) {
      const raw = (table.rows[visibleRowIndices[i]]?.[col] ?? '').trim();
      if (!raw) continue;
      nonEmpty++;
      if (uniq.size < 10_000) uniq.add(raw);
      const n = Number(raw);
      if (Number.isFinite(n)) {
        numCount++;
        sum += n;
        if (n < min) min = n;
        if (n > max) max = n;
      }
    }
    return {
      colName,
      nonEmpty,
      unique: uniq.size,
      numeric: numCount > 0,
      min: numCount > 0 ? min : null,
      max: numCount > 0 ? max : null,
      sum: numCount > 0 ? Number(sum.toFixed(4)) : null,
      avg: numCount > 0 ? Number((sum / numCount).toFixed(4)) : null,
    };
  });

  $effect(() => {
    const el = scrollerEl;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver((entries) => {
      const h = entries[0]?.contentRect.height;
      if (h && h > 0) viewportHeight = h;
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  function formatRfc4180Cell(val: string, delim: CsvDelimiter): string {
    if (val.includes('"') || val.includes('\n') || val.includes('\r') || val.includes(delim)) {
      return `"${val.replace(/"/g, '""')}"`;
    }
    return val;
  }

  function serializeTable(headers: string[], rows: string[][]): string {
    if (isJsonl) {
      return `${rows
        .map((r) => {
          const obj: Record<string, unknown> = {};
          headers.forEach((h, idx) => {
            const raw = r[idx] ?? '';
            const num = Number(raw);
            obj[h] = raw !== '' && Number.isFinite(num) && String(num) === raw ? num : raw;
          });
          return JSON.stringify(obj);
        })
        .join('\n')}\n`;
    }
    const lines: string[] = [];
    if (firstRowHeader) {
      lines.push(headers.map((h) => formatRfc4180Cell(h, delimiter)).join(delimiter));
    }
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i];
      const padded: string[] = [];
      for (let c = 0; c < headers.length; c++) padded.push(formatRfc4180Cell(r[c] ?? '', delimiter));
      lines.push(padded.join(delimiter));
    }
    return lines.length > 0 ? `${lines.join('\n')}\n` : '';
  }

  function commitTableMutation(nextHeaders: string[], nextRows: string[][]): void {
    if (readOnly || !onchange) return;
    undoStack = [...undoStack.slice(-(MAX_UNDO - 1)), content];
    redoStack = [];
    onchange(serializeTable(nextHeaders, nextRows));
  }

  function undoGrid(): void {
    if (readOnly || !onchange || undoStack.length === 0) return;
    const prev = undoStack[undoStack.length - 1];
    undoStack = undoStack.slice(0, -1);
    redoStack = [...redoStack.slice(-(MAX_UNDO - 1)), content];
    onchange(prev);
  }

  function redoGrid(): void {
    if (readOnly || !onchange || redoStack.length === 0) return;
    const next = redoStack[redoStack.length - 1];
    redoStack = redoStack.slice(0, -1);
    undoStack = [...undoStack.slice(-(MAX_UNDO - 1)), content];
    onchange(next);
  }

  function toggleSort(colIdx: number): void {
    selectedCol = colIdx;
    if (sortCol !== colIdx) {
      sortCol = colIdx;
      sortDir = 'asc';
    } else if (sortDir === 'asc') {
      sortDir = 'desc';
    } else {
      sortCol = null;
    }
  }

  function startEditCell(rowIdx: number, colIdx: number): void {
    if (readOnly) return;
    selectedRow = rowIdx;
    selectedCol = colIdx;
    editingCell = { rowIdx, colIdx, draft: table.rows[rowIdx]?.[colIdx] ?? '' };
  }

  function commitCellEdit(): void {
    const ed = editingCell;
    if (!ed) return;
    editingCell = null;
    const prevVal = table.rows[ed.rowIdx]?.[ed.colIdx] ?? '';
    if (ed.draft === prevVal) return;
    const nextRows = table.rows.slice();
    const rowCopy = [...(nextRows[ed.rowIdx] ?? [])];
    while (rowCopy.length <= ed.colIdx) rowCopy.push('');
    rowCopy[ed.colIdx] = ed.draft;
    nextRows[ed.rowIdx] = rowCopy;
    commitTableMutation(table.headers, nextRows);
  }

  function addRow(): void {
    if (readOnly) return;
    const emptyRow = table.headers.map(() => '');
    commitTableMutation(table.headers, [...table.rows, emptyRow]);
  }

  function deleteSelectedRow(): void {
    if (readOnly || table.rows.length === 0) return;
    const idx = Math.max(0, Math.min(table.rows.length - 1, selectedRow));
    const nextRows = table.rows.filter((_, i) => i !== idx);
    commitTableMutation(table.headers, nextRows);
  }

  function addColumn(): void {
    if (readOnly) return;
    const name = `col_${table.headers.length + 1}`;
    const nextHeaders = [...table.headers, name];
    const nextRows = table.rows.map((r) => [...r, '']);
    commitTableMutation(nextHeaders, nextRows);
  }

  function formatDisplayCell(val: string): string {
    return val.includes('\n') || val.includes('\r') ? val.replace(/\r?\n/g, ' ↵ ') : val;
  }

  function onGridKeydown(e: KeyboardEvent): void {
    if (!rootEl || !rootEl.contains(document.activeElement)) return;
    if ((e.metaKey || e.ctrlKey) && !editingCell) {
      if (e.key.toLowerCase() === 'z' && e.shiftKey) {
        e.preventDefault();
        redoGrid();
      } else if (e.key.toLowerCase() === 'z') {
        e.preventDefault();
        undoGrid();
      }
    }
  }
</script>

<svelte:window onkeydown={onGridKeydown} />

<div
  class="csv-root"
  bind:this={rootEl}
  role="region"
  aria-label="Structured data table editor"
  data-testid="ide-csv-view"
>
  <header class="csv-toolbar">
    <Badge tone="info" variant="outline" label={delimiterLabel} />
    <span class="csv-dim mono" dir="ltr">
      {totalRows.toLocaleString()} {totalRows === 1 ? 'row' : 'rows'} · {table.headers.length} {table.headers.length === 1 ? 'col' : 'cols'}
    </span>

    <div class="input-group csv-filter">
      <Icon name="filter" size={12} />
      <input
        type="search"
        dir="ltr"
        aria-label="Filter table rows"
        placeholder="Filter rows…"
        bind:value={filterQuery}
        data-testid="ide-csv-filter-input"
      />
    </div>

    <button
      type="button"
      class="btn small ghost"
      class:active={stickyHeader}
      aria-pressed={stickyHeader}
      onclick={() => (stickyHeader = !stickyHeader)}
    >
      Sticky header
    </button>

    {#if !readOnly}
      <button type="button" class="btn small ghost" onclick={addRow} data-testid="ide-csv-add-row">
        <Icon name="plus" size={12} />
        <span>Row</span>
      </button>
      <button type="button" class="btn small ghost" onclick={addColumn} data-testid="ide-csv-add-col">
        <Icon name="plus" size={12} />
        <span>Column</span>
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Delete selected row"
        title="Delete selected row"
        disabled={table.rows.length === 0}
        onclick={deleteSelectedRow}
      >
        <Icon name="trash" size={12} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Undo table edit"
        title="Undo table edit (⌘Z)"
        disabled={undoStack.length === 0}
        onclick={undoGrid}
      >
        <Icon name="undo" size={12} />
      </button>
    {/if}

    <span class="grow"></span>

    <div class="csv-stats mono" dir="ltr" data-testid="ide-csv-stats">
      <span class="csv-stat-col">{colStats.colName}:</span>
      <span>n={colStats.nonEmpty}</span>
      <span>uniq={colStats.unique}</span>
      {#if colStats.numeric}
        <span>min={colStats.min}</span>
        <span>max={colStats.max}</span>
        <span>sum={colStats.sum}</span>
        <span>avg={colStats.avg}</span>
      {/if}
    </div>
  </header>

  <div
    class="csv-scroller"
    bind:this={scrollerEl}
    onscroll={(e) => (scrollTop = (e.currentTarget as HTMLDivElement).scrollTop)}
  >
    {#if table.headers.length === 0}
      <EmptyState icon="grid" title="Empty table" body="Add rows or switch to Text view to paste data." />
    {:else}
      <table class="csv-table" dir="ltr">
        <thead class:is-sticky={stickyHeader}>
          <tr>
            <th class="csv-rownum-head">#</th>
            {#each table.headers as h, colIdx (`h:${colIdx}`)}
              <th class:is-active-col={selectedCol === colIdx}>
                <button
                  type="button"
                  class="csv-col-btn"
                  onclick={() => toggleSort(colIdx)}
                  title="Sort or inspect stats for {h}"
                >
                  <span class="csv-col-title">{h}</span>
                  {#if sortCol === colIdx}
                    <Icon name={sortDir === 'asc' ? 'arrowUp' : 'arrowDown'} size={12} />
                  {/if}
                </button>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#if topPadPx > 0}
            <tr aria-hidden="true"><td colspan={table.headers.length + 1} style:block-size={`${topPadPx}px`} class="csv-spacer"></td></tr>
          {/if}
          {#each virtualSlice as realRowIdx (realRowIdx)}
            {@const row = table.rows[realRowIdx] ?? []}
            {@const srcLine = table.rowStartLines[realRowIdx] ?? realRowIdx + (isJsonl || !firstRowHeader ? 1 : 2)}
            <tr class:is-selected-row={selectedRow === realRowIdx}>
              <td class="csv-rownum">
                <button
                  type="button"
                  class="csv-line-jump"
                  aria-label="Open row {realRowIdx + 1} in text editor"
                  title="Jump to source line {srcLine}"
                  onclick={() => onswitchtext?.(srcLine)}
                >
                  {realRowIdx + 1}
                </button>
              </td>
              {#each table.headers as _h, colIdx (`c:${realRowIdx}:${colIdx}`)}
                {@const val = row[colIdx] ?? ''}
                {@const isEditing = editingCell?.rowIdx === realRowIdx && editingCell?.colIdx === colIdx}
                <td
                  class="csv-cell"
                  class:is-active-col={selectedCol === colIdx}
                >
                  {#if isEditing && editingCell}
                    {#if editingCell.draft.includes('\n') || editingCell.draft.includes('\r')}
                      <textarea
                        class="csv-cell-input csv-cell-textarea mono"
                        rows="2"
                        dir="ltr"
                        data-autofocus
                        aria-label="Edit row {realRowIdx + 1} column {table.headers[colIdx]}"
                        bind:value={editingCell.draft}
                        onblur={commitCellEdit}
                        onkeydown={(e) => {
                          if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); commitCellEdit(); }
                          else if (e.key === 'Escape') { e.preventDefault(); editingCell = null; }
                        }}
                      ></textarea>
                    {:else}
                      <input
                        class="csv-cell-input mono"
                        type="text"
                        dir="ltr"
                        data-autofocus
                        aria-label="Edit row {realRowIdx + 1} column {table.headers[colIdx]}"
                        bind:value={editingCell.draft}
                        onblur={commitCellEdit}
                        onkeydown={(e) => {
                          if (e.key === 'Enter' && e.shiftKey && editingCell) {
                            e.preventDefault();
                            editingCell.draft = `${editingCell.draft}\n`;
                          } else if (e.key === 'Enter') {
                            e.preventDefault();
                            commitCellEdit();
                          } else if (e.key === 'Escape') {
                            e.preventDefault();
                            editingCell = null;
                          }
                        }}
                      />
                    {/if}
                  {:else}
                    <button
                      type="button"
                      class="csv-cell-btn mono"
                      data-rainbow={rainbowColumns ? String(colIdx % 6) : undefined}
                      title={val || `Row ${realRowIdx + 1}, ${table.headers[colIdx]}`}
                      aria-label="{table.headers[colIdx]} row {realRowIdx + 1}: {val || 'empty'}"
                      onclick={() => {
                        selectedRow = realRowIdx;
                        selectedCol = colIdx;
                      }}
                      ondblclick={() => startEditCell(realRowIdx, colIdx)}
                      onkeydown={(e) => {
                        if (e.key === 'Enter' || e.key === 'F2') {
                          e.preventDefault();
                          startEditCell(realRowIdx, colIdx);
                        } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'c') {
                          e.preventDefault();
                          void copyWithToast(val, table.headers[colIdx]);
                        }
                      }}
                      oncontextmenu={(e) => {
                        e.preventDefault();
                        void copyWithToast(val, table.headers[colIdx]);
                      }}
                      use:rowMenu
                    >{formatDisplayCell(val)}</button>
                  {/if}
                </td>
              {/each}
            </tr>
          {/each}
          {#if bottomPadPx > 0}
            <tr aria-hidden="true"><td colspan={table.headers.length + 1} style:block-size={`${bottomPadPx}px`} class="csv-spacer"></td></tr>
          {/if}
        </tbody>
      </table>
    {/if}
  </div>
</div>

<style>
  .csv-root {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-inline-size: 0;
    min-block-size: 0;
    background: var(--bg);
  }
  .csv-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-4);
    background: var(--surface);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .csv-dim {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .csv-filter {
    block-size: 24px;
    inline-size: 176px;
    font-size: var(--fs-xs);
  }
  .csv-stats {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-3);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .csv-stat-col {
    color: var(--accent-text);
    font-weight: 600;
  }
  .csv-scroller {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
  }
  .csv-table {
    inline-size: max-content;
    min-inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
  }
  .csv-table thead.is-sticky th {
    position: sticky;
    inset-block-start: 0;
    z-index: var(--z-sticky);
  }
  .csv-table th {
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
    border-inline-end: 1px solid var(--separator);
    padding: 0;
    text-align: start;
    font-weight: 600;
  }
  .csv-table th.is-active-col {
    background: var(--accent-soft);
  }
  .csv-rownum-head,
  .csv-rownum {
    inline-size: 46px;
    min-inline-size: 46px;
    text-align: end;
    padding-inline: var(--sp-3);
    color: var(--text-dim);
    background: var(--surface);
    border-inline-end: 1px solid var(--border);
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .csv-col-btn {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    inline-size: 100%;
    min-inline-size: 108px;
    max-inline-size: 260px;
    block-size: 28px;
    padding-inline: var(--sp-4);
    border: none;
    background: transparent;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
    cursor: pointer;
    text-align: start;
  }
  .csv-col-btn:hover {
    background: var(--hover);
  }
  .csv-col-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .csv-table tbody tr {
    block-size: 28px;
    border-block-end: 1px solid var(--separator);
  }
  .csv-table tbody tr:hover {
    background: var(--hover);
  }
  .csv-table tbody tr.is-selected-row {
    background: var(--accent-faint);
  }
  .csv-cell {
    max-inline-size: 280px;
    block-size: 28px;
    padding: 0;
    border-inline-end: 1px solid var(--separator);
    vertical-align: middle;
  }
  .csv-cell.is-active-col {
    box-shadow: inset 0 0 0 1px var(--accent-line);
  }
  .csv-cell-btn {
    display: block;
    inline-size: 100%;
    block-size: 28px;
    padding-inline: var(--sp-4);
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: 28px;
  }
  .csv-cell-btn[data-rainbow='0'] { color: var(--code-keyword); }
  .csv-cell-btn[data-rainbow='1'] { color: var(--code-string); }
  .csv-cell-btn[data-rainbow='2'] { color: var(--code-number); }
  .csv-cell-btn[data-rainbow='3'] { color: var(--code-builtin); }
  .csv-cell-btn[data-rainbow='4'] { color: var(--info); }
  .csv-cell-btn[data-rainbow='5'] { color: var(--warning); }
  .csv-cell-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .csv-cell-input {
    inline-size: 100%;
    block-size: 26px;
    padding-inline: var(--sp-3);
    border: 1px solid var(--accent-text);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    outline: none;
  }
  .csv-cell-textarea {
    block-size: auto;
    min-block-size: 28px;
    padding-block: 2px;
    resize: vertical;
  }
  .csv-cell-input:focus,
  .csv-cell-input:focus-visible {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .csv-line-jump {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    cursor: pointer;
    padding: 0;
  }
  .csv-line-jump:hover {
    color: var(--accent-text);
    text-decoration: underline;
  }
  .csv-spacer {
    padding: 0;
    border: none;
  }
</style>
