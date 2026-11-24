<script lang="ts">
  // One row of "Where it went" (provider, model, kind of work): name … cost on
  // line 1, the tokens behind it dim on line 2, and a faint bar under the row
  // sized against the biggest cost in the list. Two lines, so nothing scrolls
  // sideways at any width.
  import { formatUsd } from '../../lib/metric-format';

  let { label, sub, cost, share, title }: { label: string; sub: string; cost: number; share: number; title?: string } = $props();
</script>

<div class="cost-row" {title}>
  <span class="bar" style="inline-size: {share}%" aria-hidden="true"></span>
  <span class="l1"><span class="name" dir="auto">{label}</span><span class="cost bar-cost">{formatUsd(cost)}</span></span>
  <span class="l2">{sub}</span>
</div>

<style>
  .cost-row {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding-block: var(--sp-3);
    padding-inline: var(--sp-4);
    border-block-end: 1px solid var(--separator);
    min-inline-size: 0;
  }
  .bar {
    position: absolute;
    inset-block-end: -1px;
    inset-inline-start: 0;
    block-size: 2px;
    background: var(--accent-solid);
    border-radius: var(--radius-s);
  }
  .l1 {
    display: flex;
    align-items: baseline;
    gap: var(--sp-5);
    min-inline-size: 0;
  }
  .name {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
  }
  .cost {
    flex: none;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .l2 {
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
</style>
