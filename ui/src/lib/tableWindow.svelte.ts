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
//
// ⌘F over the WHOLE list (not just the mounted slice) is opt-in, like
// VirtualList's `findText` — one provider per window (lib/findProviders.ts):
//   $effect(() => tw.findRows(() => el, () => rows, (r) => `${r.name}\n${r.size}`));
// The text should list the row's cells in DOM order (see findProviders.ts).

import { tick } from 'svelte';
import { registerFindProvider } from './findProviders';

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

  /** True when `n` rows are windowed (above `min`). Lets a list give its
   *  scroller a bounded height only while windowing, e.g. on phones where the
   *  page (not the list) would otherwise scroll and the slice never moves. */
  active(n: number): boolean {
    return n > this.#min;
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

  /**
   * Register a find provider over every row (`rows()` is the full list the
   * window slices, `text` what row i shows). Sits out while the list isn't
   * windowed — then every row is mounted and the plain DOM walk is exact.
   * `rowSelector` matches the rendered data rows, in order (as `measure`).
   * Returns the unregister — call it from an `$effect`.
   */
  findRows<T>(
    container: () => HTMLElement | undefined | null,
    rows: () => readonly T[],
    text: (row: T, i: number) => string,
    rowSelector = 'tbody tr:not(.tw-spacer)',
  ): () => void {
    return registerFindProvider({
      root: () => container() ?? null,
      active: () => this.active(rows().length),
      count: () => rows().length,
      text: (i) => {
        const r = rows()[i];
        return r === undefined ? '' : text(r, i);
      },
      reveal: async (i) => {
        const el = container();
        if (!el) return;
        el.scrollTop = Math.max(0, i * this.rowH - (this.viewH || 600) / 2);
        this.scrollTop = el.scrollTop;
        await tick();
      },
      rowElement: (i) => {
        const { start, end } = this.range(rows().length);
        if (i < start || i >= end) return null;
        return container()?.querySelectorAll(rowSelector)[i - start] ?? null;
      },
    });
  }

  /** Back to the top (a new listing replaced the rows). */
  reset(container?: HTMLElement | null): void {
    this.scrollTop = 0;
    if (container) container.scrollTop = 0;
  }
}
