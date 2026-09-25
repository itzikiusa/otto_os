<script lang="ts">
  // Row detail side panel for the results grid (TablePlus's "row detail"): the
  // record under the grid cursor, one field per line — name + type, the value
  // wrapped in full (NULL dim, JSON pretty-printed), a per-field copy. Read-only
  // on purpose: editing stays in the grid / Vertical view where it is
  // review-gated. ‹ › step through the rows in view.
  import Icon from '../../lib/components/Icon.svelte';
  import type { QueryResult } from '../../lib/api/types';
  import { cellStr, copyText, isComplex, prettyJson } from './results-format';
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
    onclose: () => void;
  }
  let { result, row, rowIdx, position, total, onstep, onopen, onclose }: Props = $props();

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

<aside class="row-detail" aria-label="Row detail">
  <header class="rd-head">
    <span class="rd-title">{rowIdx === null ? 'Row' : `Row ${rowIdx + 1}`}</span>
    {#if position >= 0}<span class="rd-pos">{position + 1} of {total.toLocaleString()}</span>{/if}
    <span class="grow"></span>
    <button class="icon-btn" disabled={position <= 0} onclick={() => onstep(-1)} aria-label="Previous row" title="Previous row">
      <Icon name="chevronUp" size={13} />
    </button>
    <button class="icon-btn" disabled={position < 0 || position >= total - 1} onclick={() => onstep(1)} aria-label="Next row" title="Next row">
      <Icon name="chevronDown" size={13} />
    </button>
    <button class="icon-btn" onclick={onclose} aria-label="Close row detail" title="Close row detail">
      <Icon name="x" size={13} />
    </button>
  </header>
  <div class="rd-search">
    <Icon name="search" size={12} />
    <input
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
              onclick={() => copyText(v === null || v === undefined ? '' : cellStr(v))}
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
</aside>

<style>
  .row-detail {
    flex: 0 0 300px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    overflow: hidden;
  }
  .rd-head {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 40px;
    box-sizing: border-box;
    padding: 0 6px 0 12px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
  }
  .rd-title {
    font-weight: 600;
    font-size: var(--fs-s);
  }
  .rd-pos {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-start: 6px;
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
    opacity: 0.8;
    white-space: nowrap;
  }
  .rd-act {
    width: 20px;
    height: 20px;
    opacity: 0;
  }
  .rd-field:hover .rd-act,
  .rd-act:focus-visible {
    opacity: 1;
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
  .rd-empty {
    margin: 0;
    padding: 16px 12px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .grow {
    flex: 1;
  }
  @media (max-width: 900px) {
    .row-detail {
      flex-basis: 240px;
    }
  }
</style>
