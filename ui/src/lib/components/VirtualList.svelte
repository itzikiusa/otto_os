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
  import { tick, untrack, type Snippet } from 'svelte';
  import { registerFindProvider } from '../findProviders';

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
    /** Opt into ⌘F over the WHOLE list (lib/findProviders.ts): the text a row
     *  shows. Without it find-in-page only sees the mounted window. */
    findText?: (item: T, index: number) => string;
    /** Bump when `items` was mutated IN PLACE (same array, new length /
     *  contents) — e.g. a capped log ring appended without copying. */
    version?: number;
  }
  let { items, estimateHeight, overscan = 6, row, class: cls = '', pinnedIndex = -1, scrollIndex = -1, scrollVersion = 0, key, tabindex, findText, version = 0 }: Props = $props();
  let viewport: HTMLDivElement | undefined = $state();

  let scrollTop = $state(0);
  let clientH = $state(0);

  const total = $derived((void version, items.length * estimateHeight));
  const start = $derived(Math.max(0, Math.floor(scrollTop / estimateHeight) - overscan));
  const count = $derived(
    (void version, Math.min(items.length - start, Math.ceil((clientH || 600) / estimateHeight) + overscan * 2 + 1)),
  );
  const slice = $derived((void version, items.slice(start, start + count)));

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

  let winEl: HTMLDivElement | undefined = $state();
  $effect(() => {
    const text = findText;
    if (!text) return;
    return registerFindProvider({
      root: () => viewport ?? null,
      count: () => items.length,
      text: (i) => text(items[i], i),
      reveal: async (i) => {
        if (!viewport) return;
        viewport.scrollTop = Math.max(0, i * estimateHeight - (clientH || 600) / 2);
        scrollTop = viewport.scrollTop;
        await tick();
      },
      // One element per row (the row snippet's root), in window order.
      rowElement: (i) => (i >= start && i < start + count ? (winEl?.children[i - start] ?? null) : null),
    });
  });

  // The pinned row renders from one of two places: the window, or the
  // out-of-window `.vlist-pin` copy. Crossing between them (scrolling the
  // focused row out of view, or back in) swaps in a NEW element and the
  // browser drops focus to <body> — a keyboard user's ↑/↓ then go nowhere.
  // Remember whether focus was in the pinned row before the swap and hand it
  // to the row's new element after.
  let pinEl: HTMLDivElement | undefined = $state();
  const pinOut = $derived(pinnedIndex >= 0 && pinnedIndex < items.length && (pinnedIndex < start || pinnedIndex >= start + count));
  let pinHadFocus = false;
  // The pinned row's element as last painted (read before the DOM updates).
  let paintedPinRow: Element | null | undefined = null;
  $effect.pre(() => {
    void pinOut;
    const active = document.activeElement;
    pinHadFocus = !!active && !!paintedPinRow?.isConnected && paintedPinRow.contains(active);
  });
  $effect(() => {
    const out = pinOut;
    const at = pinnedIndex - start;
    const rowEl = untrack(() => (out ? pinEl?.firstElementChild : winEl?.children[at]));
    paintedPinRow = rowEl;
    if (!pinHadFocus) return;
    pinHadFocus = false;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    if (!(rowEl instanceof HTMLElement)) return;
    const target = rowEl.hasAttribute('tabindex') ? rowEl : rowEl.querySelector<HTMLElement>('[tabindex="0"]');
    target?.focus({ preventScroll: true });
  });

  function onScroll(e: Event): void {
    scrollTop = (e.currentTarget as HTMLElement).scrollTop;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex (Only -1 is accepted to opt out of the browser scroll-container tab stop.) -->
<div class="vlist {cls}" {tabindex} bind:this={viewport} bind:clientHeight={clientH} onscroll={onScroll}>
  <div class="vlist-sizer" style="height:{total}px">
    <div class="vlist-win" bind:this={winEl} style="transform:translateY({start * estimateHeight}px)">
      {#each slice as item, i (key ? key(item, start + i) : start + i)}
        {@render row(item, start + i)}
      {/each}
    </div>
    {#if pinOut}
      <div class="vlist-pin" bind:this={pinEl} style="top:{pinnedIndex * estimateHeight}px">
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
