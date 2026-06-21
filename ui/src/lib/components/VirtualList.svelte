<script lang="ts" generics="T">
  // Windowed list primitive (T2). Renders only the visible slice (+ overscan) of
  // a large, uniform-height collection so big diffs/results/streams stay at a
  // bounded DOM-node count. The consumer constrains the height (CSS on the
  // element, or a `class`) and supplies a `row` snippet:
  //
  //   <VirtualList items={rows} estimateHeight={28} class="grid-body">
  //     {#snippet row(item, index)}<div class="r">{item.name}</div>{/snippet}
  //   </VirtualList>
  //
  // Rows are assumed ~uniform `estimateHeight` px; mild variance is tolerated via
  // overscan. For wildly variable heights, wrap rows to a fixed height.
  import type { Snippet } from 'svelte';

  interface Props {
    items: T[];
    estimateHeight: number;
    overscan?: number;
    row: Snippet<[T, number]>;
    class?: string;
  }
  let { items, estimateHeight, overscan = 6, row, class: cls = '' }: Props = $props();

  let scrollTop = $state(0);
  let clientH = $state(0);

  const total = $derived(items.length * estimateHeight);
  const start = $derived(Math.max(0, Math.floor(scrollTop / estimateHeight) - overscan));
  const count = $derived(
    Math.min(items.length - start, Math.ceil((clientH || 600) / estimateHeight) + overscan * 2 + 1),
  );
  const slice = $derived(items.slice(start, start + count));

  function onScroll(e: Event): void {
    scrollTop = (e.currentTarget as HTMLElement).scrollTop;
  }
</script>

<div class="vlist {cls}" bind:clientHeight={clientH} onscroll={onScroll}>
  <div class="vlist-sizer" style="height:{total}px">
    <div class="vlist-win" style="transform:translateY({start * estimateHeight}px)">
      {#each slice as item, i (start + i)}
        {@render row(item, start + i)}
      {/each}
    </div>
  </div>
</div>

<style>
  .vlist {
    overflow: auto;
    position: relative;
  }
  .vlist-sizer {
    position: relative;
    width: 100%;
  }
  .vlist-win {
    position: absolute;
    top: 0;
    inset-inline-start: 0;
    inset-inline-end: 0;
  }
</style>
