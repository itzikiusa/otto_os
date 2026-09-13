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
    /** Optional keyboard-controlled row. Pinning keeps its ARIA target mounted. */
    pinnedIndex?: number;
    tabindex?: -1;
    scrollIndex?: number;
    scrollVersion?: number;
    key?: (item: T, index: number) => string | number;
  }
  let { items, estimateHeight, overscan = 6, row, class: cls = '', pinnedIndex = -1, scrollIndex = -1, scrollVersion = 0, key, tabindex }: Props = $props();
  let viewport: HTMLDivElement | undefined = $state();

  let scrollTop = $state(0);
  let clientH = $state(0);

  const total = $derived(items.length * estimateHeight);
  const start = $derived(Math.max(0, Math.floor(scrollTop / estimateHeight) - overscan));
  const count = $derived(
    Math.min(items.length - start, Math.ceil((clientH || 600) / estimateHeight) + overscan * 2 + 1),
  );
  const slice = $derived(items.slice(start, start + count));

  $effect(() => {
    void scrollVersion;
    const index = scrollIndex;
    if (!viewport || index < 0 || index >= items.length) return;
    const top = index * estimateHeight;
    const bottom = top + estimateHeight;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (bottom > viewport.scrollTop + (clientH || 600)) viewport.scrollTop = bottom - (clientH || 600);
    scrollTop = viewport.scrollTop;
  });

  function onScroll(e: Event): void {
    scrollTop = (e.currentTarget as HTMLElement).scrollTop;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex (Only -1 is accepted to opt out of the browser scroll-container tab stop.) -->
<div class="vlist {cls}" {tabindex} bind:this={viewport} bind:clientHeight={clientH} onscroll={onScroll}>
  <div class="vlist-sizer" style="height:{total}px">
    <div class="vlist-win" style="transform:translateY({start * estimateHeight}px)">
      {#each slice as item, i (key ? key(item, start + i) : start + i)}
        {@render row(item, start + i)}
      {/each}
    </div>
    {#if pinnedIndex >= 0 && pinnedIndex < items.length && (pinnedIndex < start || pinnedIndex >= start + count)}
      <div class="vlist-pin" style="top:{pinnedIndex * estimateHeight}px">
        {@render row(items[pinnedIndex], pinnedIndex)}
      </div>
    {/if}
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
  .vlist-pin { position: absolute; inset-inline: 0; }
  .vlist-win {
    position: absolute;
    top: 0;
    inset-inline-start: 0;
    inset-inline-end: 0;
  }
</style>
