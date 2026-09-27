// Row windowing for plain `<table>` lists that can grow to tens of thousands of
// rows (S3 prefixes, SFTP directories). Unlike `VirtualList` it keeps the
// table's semantics and styles: the body renders only the visible slice
// between two spacer rows sized to the rows it skipped. Below `min` rows it
// renders everything (no behaviour change for ordinary lists).
//
// The row height is MEASURED from a rendered row (single-line, nowrap rows are
// uniform), so the spacers stay exact without hard-coding CSS metrics.
//
//   const tw = new TableWindow();
//   const win = $derived(tw.range(rows.length));
//   <div class="tbl-wrap" bind:this={el} bind:clientHeight={tw.viewH} onscroll={tw.onscroll}>
//     {#if win.top}<tr class="tw-spacer" aria-hidden="true"><td colspan={N} style="height:{win.top}px"></td></tr>{/if}
//     {#each rows.slice(win.start, win.end) as r (r.key)}…{/each}
//     {#if win.bottom}<tr class="tw-spacer" aria-hidden="true"><td colspan={N} style="height:{win.bottom}px"></td></tr>{/if}
//   (spacer cells need `padding: 0; border: 0` so their height is exact)
//   $effect(() => { void win; tw.measure(el); });

export interface WindowRange {
  start: number;
  end: number;
  /** Spacer height above the slice (px). */
  top: number;
  /** Spacer height below the slice (px). */
  bottom: number;
}

export class TableWindow {
  scrollTop = $state(0);
  viewH = $state(600);
  rowH = $state(30);
  #raf = 0;
  readonly #min: number;
  readonly #overscan: number;

  constructor(min = 400, overscan = 15) {
    this.#min = min;
    this.#overscan = overscan;
  }

  range(n: number): WindowRange {
    if (n <= this.#min) return { start: 0, end: n, top: 0, bottom: 0 };
    const h = this.rowH;
    const start = Math.max(0, Math.min(n, Math.floor(this.scrollTop / h) - this.#overscan));
    const end = Math.min(n, start + Math.ceil((this.viewH || 600) / h) + this.#overscan * 2);
    return { start, end, top: start * h, bottom: (n - end) * h };
  }

  /** Scroll handler for the scroll container (rAF-coalesced). */
  onscroll = (e: Event): void => {
    const el = e.currentTarget as HTMLElement;
    if (this.#raf) return;
    this.#raf = requestAnimationFrame(() => {
      this.#raf = 0;
      this.scrollTop = el.scrollTop;
    });
  };

  /** Re-measure the row height from the first rendered data row. */
  measure(container: HTMLElement | undefined | null, selector = 'tbody tr:not(.tw-spacer)'): void {
    const row = container?.querySelector<HTMLElement>(selector);
    if (!row) return;
    const h = row.getBoundingClientRect().height;
    if (h > 4 && Math.abs(h - this.rowH) > 0.25) this.rowH = h;
  }

  /** Back to the top (a new listing replaced the rows). */
  reset(container?: HTMLElement | null): void {
    this.scrollTop = 0;
    if (container) container.scrollTop = 0;
  }
}
