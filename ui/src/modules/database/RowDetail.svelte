<script lang="ts">
  // Row detail for the results grid (TablePlus's "row detail"), the grid's
  // Detail pane: the record under the grid cursor, one field per line — name +
  // type, the value wrapped in full (NULL dim, JSON pretty-printed), a
  // per-field copy. Read-only on purpose: editing stays in the grid / Vertical
  // view where it is review-gated. The footer steps through the rows in view
  // ("‹ Previous · 3 of 50 · Next ›"); ⋯ holds the raw JSON view. Inline
  // beside the grid while there is room, a sheet otherwise (full width on a
  // phone). ⌥⌘B opens / closes it.
  import Icon from '../../lib/components/Icon.svelte';
  import DetailPane from '../../lib/components/side/DetailPane.svelte';
  import type { QueryResult } from '../../lib/api/types';
  import { cellStr, copyWithToast, isComplex, prettyJson } from './results-format';
  import { columnKind } from './grid-format';

  interface Props {
    result: QueryResult;
    row: unknown[] | null;
    /** liveRows index of `row` (shown as "Row N"). */
    rowIdx: number | null;
    /** Position of the row within the filtered/sorted view (-1 = not in view). */
    position: number;
    total: number;
    onstep: (delta: number) => void;
    /** Open a field in the full cell viewer. */
    onopen: (ci: number) => void;
    /** Open the whole row as searchable raw JSON (labelled `rawLabel`). */
    onraw?: () => void;
    rawLabel?: string;
    open: boolean;
    onclose: () => void;
    /** ⌥⌘B: open / close. */
    ontoggle: () => void;
  }
  let { result, row, rowIdx, position, total, onstep, onopen, onraw, rawLabel = 'View raw JSON', open, onclose, ontoggle }: Props = $props();

  let fieldFilter = $state('');
  const fields = $derived.by(() => {
    const q = fieldFilter.trim().toLowerCase();
    return result.columns
      .map((c, ci) => ({ c, ci }))
      .filter(({ c }) => !q || c.name.toLowerCase().includes(q));
  });
  function text(v: unknown): string {
    if (v === null || v === undefined) return '';
    return isComplex(v) ? prettyJson(v) : cellStr(v);
  }
</script>

{#snippet rowNav()}
  <button type="button" class="btn small ghost" disabled={position <= 0} onclick={() => onstep(-1)} aria-label="Previous row" title="Previous row">
    <Icon name="chevronLeft" size={12} /> Previous
  </button>
  <span class="rd-pos">{position >= 0 ? `${position + 1} of ${total.toLocaleString()}` : ''}</span>
  <button type="button" class="btn small ghost" disabled={position < 0 || position >= total - 1} onclick={() => onstep(1)} aria-label="Next row" title="Next row">
    Next <Icon name="chevronRight" size={12} />
  </button>
{/snippet}

<DetailPane
  module="database"
  pane="row"
  title={rowIdx === null ? 'Row' : `Row ${rowIdx + 1}`}
  {open}
  {onclose}
  {ontoggle}
  menu={onraw ? () => [{ label: rawLabel, icon: 'eye', disabled: !row, action: () => onraw?.() }] : undefined}
  footer={rowNav}
  testid="row-detail"
>
  <div class="row-detail">
    <div class="rd-search">
      <Icon name="search" size={12} />
      <input dir="ltr"
        class="rd-search-input"
        type="text"
        placeholder="Find a field…"
        aria-label="Find a field"
        spellcheck="false"
        bind:value={fieldFilter}
      />
    </div>
    {#if !row}
      <p class="rd-empty">Click a row in the grid to see all of its fields here.</p>
    {:else}
      <dl class="rd-fields">
        {#each fields as { c, ci } (ci)}
          {@const v = row[ci]}
          {@const kind = columnKind(c.type_hint, [v])}
          <div class="rd-field">
            <dt>
              <span class="rd-name mono" title={c.name}>{c.name}</span>
              {#if c.type_hint}<span class="rd-type mono">{c.type_hint}</span>{/if}
              <span class="grow"></span>
              <button
                class="icon-btn rd-act"
                onclick={() => copyWithToast(v === null || v === undefined ? '' : cellStr(v))}
                aria-label="Copy {c.name}"
                title="Copy value"
              ><Icon name="copy" size={12} /></button>
              <button class="icon-btn rd-act" onclick={() => onopen(ci)} aria-label="Open {c.name} in the viewer" title="Open in the viewer">
                <Icon name="maximize" size={12} />
              </button>
            </dt>
            <dd class="mono" class:num={kind === 'num'}>
              {#if v === null || v === undefined}
                <span class="rd-null">NULL</span>
              {:else if isComplex(v)}
                <pre class="rd-json">{text(v)}</pre>
              {:else}
                {text(v)}
              {/if}
            </dd>
          </div>
        {/each}
        {#if fields.length === 0}
          <p class="rd-empty">No field matches “{fieldFilter}”.</p>
        {/if}
      </dl>
    {/if}
  </div>
</DetailPane>

<style>
  .row-detail {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .rd-pos {
    flex: 1;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .rd-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
  }
  /* Full-width bar, so the focus cue is an accent underline, not a ring. */
  .rd-search:focus-within {
    border-bottom-color: var(--accent-text);
    box-shadow: inset 0 -1px 0 var(--accent-text);
  }
  .rd-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .rd-fields {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 4px 0;
  }
  .rd-field {
    padding: 6px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .rd-field dt {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .rd-name {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rd-type {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .rd-act {
    width: 20px;
    height: 20px;
    opacity: 0;
  }
  .rd-field:hover .rd-act,
  .rd-field:focus-within .rd-act,
  .rd-act:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .rd-act {
      opacity: 1;
    }
  }
  .rd-field dd {
    margin: 2px 0 0;
    font-size: var(--fs-s);
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
  }
  .rd-field dd.num {
    font-variant-numeric: tabular-nums;
  }
  .rd-null {
    color: var(--text-dim);
    font-style: italic;
    font-size: var(--fs-xs);
  }
  .rd-json {
    margin: 0;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    max-height: 220px;
    overflow: auto;
  }
  .grow {
    flex: 1;
  }
  .rd-empty {
    margin: 0;
    padding: 16px 12px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
