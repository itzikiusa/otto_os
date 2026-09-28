// Row windowing for a uniform-height list that scrolls inside an OUTER
// container it shares with other content (the WIP panel's two sections, the
// PR file-nav tree, the blame table). Unlike `VirtualList`, the list is not
// its own scroller: the window is computed from the scroller's viewport and
// the list's offset inside it, and the list renders only the visible slice
// between two spacers sized to the rows it skipped. At or below `min` rows it
// renders everything (ordinary lists are unchanged — find-in-page still sees
// every row). The row pitch is MEASURED from a rendered row, so a mobile
// media query that changes it keeps the spacers exact.
//
//   const lw = new ListWindow('.row');
//   const win = $derived(lw.range(rows.length));
//   <div {@attach lw.attach}>
//     {#if win.top}<div style="height:{win.top}px" aria-hidden="true"></div>{/if}
//     {#each rows.slice(win.start, win.end) as r (r.key)}…{/each}
//     {#if win.bottom}<div style="height:{win.bottom}px" aria-hidden="true"></div>{/if}
//   </div>

import { untrack } from 'svelte';
import { findScroller } from './diff-virtual';
import { listRange, type ListRange } from './list-range';

export { listRange, type ListRange };

export class ListWindow {
  /** Scroller scrollTop minus the list's offset inside the scroll content. */
  rel = $state(0);
  viewH = $state(800);
  rowH = $state(26);
  readonly #min: number;
  readonly #overscan: number;
  readonly #rowSel: string;
  #raf = 0;
  #list: HTMLElement | null = null;
  #scroller: HTMLElement | null = null;

  /** `rowSel` picks a rendered row to measure the pitch from. */
  constructor(rowSel: string, opts: { min?: number; overscan?: number; rowH?: number } = {}) {
    this.#rowSel = rowSel;
    this.#min = opts.min ?? 300;
    this.#overscan = opts.overscan ?? 20;
    this.rowH = opts.rowH ?? 26;
  }

  range(n: number): ListRange {
    return listRange(n, this.rowH, this.rel, this.viewH, this.#overscan, this.#min);
  }

  #sync = (): void => {
    this.#raf = 0;
    const list = this.#list;
    if (!list?.isConnected) return;
    const top = list.getBoundingClientRect().top;
    const sc = this.#scroller;
    if (sc) {
      this.rel = sc.getBoundingClientRect().top - top;
      this.viewH = sc.clientHeight;
    } else {
      this.rel = -top;
      this.viewH = window.innerHeight;
    }
    const row = list.querySelector<HTMLElement>(this.#rowSel);
    const h = row?.getBoundingClientRect().height ?? 0;
    if (h > 4 && Math.abs(h - this.rowH) > 0.25) this.rowH = h;
  };

  /** Re-read the geometry on the next frame — after content ABOVE the list
   *  changed height (its offset moved without a scroll or a resize). */
  refresh = (): void => {
    if (!this.#raf && this.#list) this.#raf = requestAnimationFrame(this.#sync);
  };

  /** `{@attach lw.attach}` on the list element: follows the nearest scrolling
   *  ancestor (rAF-coalesced) and its size. Only STATE is written from the
   *  observers — never DOM inside a ResizeObserver callback, so it can't feed
   *  a "ResizeObserver loop" back into itself. */
  attach = (list: HTMLElement): (() => void) => {
    this.#list = list;
    const scroller = findScroller(list);
    this.#scroller = scroller;
    const target: HTMLElement | Window = scroller ?? window;
    // Untracked: an attachment is an effect, and `#sync` reads `rowH` — a
    // dependency here would tear the listeners down on every re-measure.
    untrack(this.#sync);
    target.addEventListener('scroll', this.refresh, { passive: true });
    const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(this.refresh) : null;
    if (ro && scroller) ro.observe(scroller);
    return () => {
      target.removeEventListener('scroll', this.refresh);
      ro?.disconnect();
      if (this.#raf) cancelAnimationFrame(this.#raf);
      this.#raf = 0;
      if (this.#list === list) this.#list = this.#scroller = null;
    };
  };

  /** Scroll the outer container so row `i` is in view (keyboard/reveal). */
  reveal(i: number): void {
    const list = this.#list;
    if (!list) return;
    const sc = this.#scroller;
    const rowTop = list.getBoundingClientRect().top + i * this.rowH;
    if (sc) {
      const sr = sc.getBoundingClientRect();
      if (rowTop < sr.top || rowTop + this.rowH > sr.bottom) {
        sc.scrollTop += rowTop - sr.top - sc.clientHeight / 2;
      }
    } else {
      window.scrollBy(0, rowTop - window.innerHeight / 2);
    }
  }
}
