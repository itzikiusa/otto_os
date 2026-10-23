// Row windowing for plain `<table>` lists that can grow to tens of thousands of
// rows (S3 prefixes, SFTP directories). Unlike `VirtualList` it keeps the
// table's semantics and styles: the body renders only the visible slice
// between two spacer rows sized to the rows it skipped. Below `min` rows it
// renders everything (no behaviour change for ordinary lists).
//
// It is the shared `ListWindow` (lib/listWindow.svelte.ts) in self-scrolling
// mode: one range, one pitch rule, one find provider. The row height is
// MEASURED from a rendered row (single-line, nowrap rows are uniform), so the
// spacers stay exact without hard-coding CSS metrics.
//
//   const tw = new TableWindow();
//   const win = $derived(tw.range(rows.length));
//   <div class="tbl-wrap" bind:this={el} bind:clientHeight={tw.viewH} onscroll={tw.onscroll}>
//     {#if win.top}<tr class="tw-spacer" aria-hidden="true"><td colspan={N} style="height:{win.top}px"></td></tr>{/if}
//     {#each rows.slice(win.start, win.end) as r (r.key)}…{/each}
//     {#if win.bottom}<tr class="tw-spacer" aria-hidden="true"><td colspan={N} style="height:{win.bottom}px"></td></tr>{/if}
//   (spacer cells need `padding: 0; border: 0` so their height is exact)
//   $effect(() => { void win; tw.measure(el); });
//
// ⌘F over the WHOLE list (not just the mounted slice) is opt-in, like
// VirtualList's `findText` — one provider per window (lib/findProviders.ts):
//   $effect(() => tw.findRows(() => el, () => rows, (r) => `${r.name}\n${r.size}`));
// The text should list the row's cells in DOM order (see findProviders.ts).

import { ListWindow, type ListRange } from './listWindow.svelte';

export type WindowRange = ListRange;

export class TableWindow extends ListWindow {
  constructor(min = 400, overscan = 15) {
    super('tbody tr:not(.tw-spacer)', { min, overscan, rowH: 30, selfScroll: true });
    this.viewH = 600;
  }

  /** The table scroller's scrollTop (the window's `rel`). */
  get scrollTop(): number {
    return this.rel;
  }

  set scrollTop(v: number) {
    this.rel = v;
  }
}
