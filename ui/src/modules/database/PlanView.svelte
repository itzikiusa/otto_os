<script lang="ts">
  // Normalized query-plan tree (from POST …/db/query-plan). Renders the engine's
  // EXPLAIN as a collapsible op·object·est-rows tree with red badges on costly
  // patterns (full scans, filesort, temp tables), plus a raw-JSON toggle.
  import Icon from '../../lib/components/Icon.svelte';
  import type { DbQueryPlan, DbPlanNode } from '../../lib/api/types';

  interface Props {
    plan: DbQueryPlan;
    onclose: () => void;
  }
  let { plan, onclose }: Props = $props();

  let rawMode = $state(false);
  // Node paths the user has collapsed (default: all expanded).
  let collapsed = $state<Set<string>>(new Set());
  function toggle(id: string): void {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
  }
</script>

<div class="plan-panel">
  <div class="plan-head">
    <Icon name="zap" size={13} />
    <span class="plan-title">Query plan</span>
    <span class="plan-engine mono">{plan.engine}</span>
    <span class="plan-grow"></span>
    <button
      class="plan-btn"
      class:on={rawMode}
      onclick={() => (rawMode = !rawMode)}
      title="Toggle the engine's raw EXPLAIN JSON"
    >
      <Icon name="grid" size={11} />{rawMode ? 'Tree' : 'Raw JSON'}
    </button>
    <button class="plan-close" onclick={onclose} aria-label="Close plan"><Icon name="x" size={13} /></button>
  </div>
  {#if rawMode}
    <pre class="plan-raw mono">{JSON.stringify(plan.raw, null, 2)}</pre>
  {:else}
    <div class="plan-tree">
      {@render node(plan.root, '0', 0)}
    </div>
  {/if}
</div>

{#snippet node(n: DbPlanNode, id: string, depth: number)}
  {@const kids = n.children ?? []}
  {@const open = !collapsed.has(id)}
  <div class="plan-row" style="padding-inline-start: {depth * 16 + 6}px">
    {#if kids.length > 0}
      <button class="plan-caret" onclick={() => toggle(id)} aria-label="Toggle node">
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} />
      </button>
    {:else}
      <span class="plan-caret-spacer"></span>
    {/if}
    <span class="plan-op">{n.op}</span>
    {#if n.object}<span class="plan-object mono">{n.object}</span>{/if}
    {#if n.est_rows != null}<span class="plan-rows">~{Math.round(n.est_rows).toLocaleString()} rows</span>{/if}
    {#if n.detail}<span class="plan-detail" title={n.detail}>{n.detail}</span>{/if}
    {#each n.warnings ?? [] as w, wi (wi)}
      <span class="plan-warn" title={w}><Icon name="zap" size={9} />{w}</span>
    {/each}
  </div>
  {#if open}
    {#each kids as child, i (i)}
      {@render node(child, `${id}.${i}`, depth + 1)}
    {/each}
  {/if}
{/snippet}

<style>
  .plan-panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    overflow: hidden;
  }
  .plan-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    flex-shrink: 0;
  }
  .plan-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .plan-engine {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 1px 7px;
    border-radius: 999px;
  }
  .plan-grow {
    flex: 1;
  }
  .plan-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    border-radius: var(--radius-s);
    font-size: 11px;
    padding: 2px 8px;
    cursor: pointer;
  }
  .plan-btn:hover,
  .plan-btn.on {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .plan-close {
    display: inline-flex;
    place-items: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px;
    border-radius: var(--radius-s);
  }
  .plan-close:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
  }
  .plan-tree,
  .plan-raw {
    overflow: auto;
    min-height: 0;
    margin: 0;
  }
  .plan-tree {
    padding: 6px 8px;
  }
  .plan-raw {
    padding: 10px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text);
    white-space: pre;
  }
  .plan-row {
    display: flex;
    align-items: center;
    gap: 7px;
    min-height: 24px;
    font-size: 12px;
    color: var(--text);
    flex-wrap: wrap;
  }
  .plan-caret {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .plan-caret:hover {
    color: var(--text);
  }
  .plan-caret-spacer {
    width: 16px;
    flex-shrink: 0;
  }
  .plan-op {
    font-weight: 600;
  }
  .plan-object {
    font-size: 11.5px;
    color: var(--accent);
  }
  .plan-rows {
    font-size: 10.5px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .plan-detail {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 340px;
  }
  .plan-warn {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--status-exited) 35%, transparent);
    border-radius: 999px;
    padding: 0 7px;
  }
</style>
