<script lang="ts">
  // k9s-style resource table: sticky header row + `VirtualList` body sharing
  // one CSS grid template, health-colored status pill, keyboard-navigable rows
  // (listbox/option semantics — the whole table is one selection widget).
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { K8sResourceKind, K8sRow } from '../../lib/api/types';
  import { columnsFor, gridTemplate } from './columns';
  import { healthClass, kindDef } from './k8s-util';

  interface Props {
    kind: K8sResourceKind;
    rows: K8sRow[];
    /** Rows before the text filter (for the "n of m" footer). */
    total: number;
    hasMetrics: boolean;
    allNamespaces: boolean;
    loading: boolean;
    error: string;
    selected: { ns: string; name: string } | null;
    onselect: (row: K8sRow) => void;
    onopen: (row: K8sRow) => void;
    onmenu: (e: MouseEvent | KeyboardEvent, row: K8sRow) => void;
    onretry: () => void;
  }
  let { kind, rows, total, hasMetrics, allNamespaces, loading, error, selected, onselect, onopen, onmenu, onretry }: Props = $props();

  const ROW_H = 30;
  const cols = $derived(columnsFor(kind, rows, hasMetrics, allNamespaces));
  const template = $derived(gridTemplate(cols));
  const isSel = (r: K8sRow): boolean => !!selected && selected.name === r.name && selected.ns === r.namespace;

  function rowKey(e: KeyboardEvent, r: K8sRow, i: number): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      onopen(r);
    } else if (e.key === ' ') {
      e.preventDefault();
      onselect(r);
    } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      onmenu(e, r);
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const j = e.key === 'ArrowDown' ? i + 1 : i - 1;
      const next = rows[j];
      if (!next) return;
      onselect(next);
      // Focus the neighbouring row when it's rendered (VirtualList keeps a
      // window around the viewport, so it usually is).
      (e.currentTarget as HTMLElement).parentElement
        ?.querySelector<HTMLElement>(`[data-i="${j}"]`)
        ?.focus();
    }
  }
</script>

<div class="rt" data-testid="k8s-resource-table">
  <div class="rt-head" role="row" style="grid-template-columns:{template}">
    {#each cols as c (c.key)}
      <div class="rt-hcell" class:num={c.num} role="columnheader">{c.label}</div>
    {/each}
  </div>

  {#if error && !rows.length}
    <div class="rt-state" data-testid="k8s-table-error">
      <EmptyState icon="info" title="Couldn't load {kindDef(kind).label.toLowerCase()}" body={error} actionLabel="Retry" onaction={onretry} />
    </div>
  {:else if loading && !rows.length}
    <div class="rt-state"><Skeleton rows={8} height={26} /></div>
  {:else if !rows.length}
    <div class="rt-state">
      <EmptyState icon="box" title={total ? 'No rows match the filter' : `No ${kindDef(kind).label.toLowerCase()} here`} body={total ? `${total} hidden by the filter.` : 'Try another namespace, or "All namespaces".'} />
    </div>
  {:else}
    <div class="rt-body" role="listbox" aria-label="{kindDef(kind).label} rows" aria-multiselectable="false">
      <VirtualList items={rows} estimateHeight={ROW_H} class="rt-vlist">
        {#snippet row(r, i)}
          <div
            class="rt-row {healthClass(r.health, r.status)}"
            class:selected={isSel(r)}
            role="option"
            aria-selected={isSel(r)}
            tabindex="0"
            data-i={i}
            data-testid="k8s-row"
            style="grid-template-columns:{template};height:{ROW_H}px"
            onclick={() => onselect(r)}
            ondblclick={() => onopen(r)}
            onkeydown={(e) => rowKey(e, r, i)}
            oncontextmenu={(e) => onmenu(e, r)}
          >
            {#each cols as c (c.key)}
              {@const v = c.value(r)}
              <div class="rt-cell" class:num={c.num} class:mono={c.mono} title={v}>
                {#if c.status}
                  <span class="status-pill"><span class="hdot"></span>{v}</span>
                {:else}
                  {v}
                {/if}
              </div>
            {/each}
          </div>
        {/snippet}
      </VirtualList>
    </div>
    {#if error}
      <div class="rt-stale" role="status"><Icon name="info" size={12} /> Showing the last good load — refresh failed: {error}</div>
    {/if}
  {/if}
</div>

<style>
  .rt {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    overflow: hidden;
  }
  .rt-head,
  .rt-row {
    display: grid;
    align-items: center;
    column-gap: 10px;
    padding: 0 12px;
    min-width: max-content;
  }
  .rt-head {
    position: sticky;
    top: 0;
    z-index: 1;
    height: 28px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-dim);
    overflow: hidden;
  }
  .rt-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .rt-body :global(.rt-vlist) {
    flex: 1;
    min-height: 0;
    height: 100%;
  }
  .rt-row {
    font-size: 12.5px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
    cursor: default;
    outline: none;
    width: 100%;
    box-sizing: border-box;
  }
  .rt-row:hover {
    background: var(--surface-2);
  }
  .rt-row.selected {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .rt-row:focus-visible {
    box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .rt-cell,
  .rt-hcell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .num {
    text-align: right;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .health-ok .hdot {
    background: var(--status-working);
  }
  .health-ok .status-pill {
    color: var(--status-working);
  }
  .health-bad .hdot {
    background: var(--status-exited);
  }
  .health-bad .status-pill {
    color: var(--status-exited);
  }
  .health-warn .hdot {
    background: var(--status-idle, #d9a400);
  }
  .health-warn .status-pill {
    color: var(--status-idle, #d9a400);
  }
  .health-progressing .hdot {
    background: var(--accent);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .health-progressing .status-pill {
    color: var(--accent);
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .rt-state {
    padding: 12px;
    overflow: auto;
  }
  .rt-stale {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 4px 12px;
    font-size: 11px;
    color: var(--status-exited);
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--status-exited) 8%, var(--surface));
  }
</style>
