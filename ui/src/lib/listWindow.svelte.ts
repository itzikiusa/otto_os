// Row windowing for long uniform-height lists — the one shared primitive
// (new long lists use `VirtualList` or this; `GridView`'s 2-D window and
// `DiffViewer`'s variable-height body are the documented exceptions and only
// borrow `findScroller` / the pitch rule below).
//
// `ListWindow` covers a list that scrolls inside an OUTER container it shares
// with other content (the WIP panel's two sections, the PR file-nav tree, the
// side-panel file tree). Unlike `VirtualList`, the list is not its own
// scroller: the window is computed from the scroller's viewport and the list's
// offset inside it, and the list renders only the visible slice between two
// spacers sized to the rows it skipped. At or below `min` rows it renders
// everything (ordinary lists are unchanged — find-in-page still sees every
// row). The row pitch is MEASURED from a rendered row, so a mobile media query
// that changes it keeps the spacers exact.
//
//   const lw = new ListWindow('.row');
//   const win = $derived(lw.range(rows.length));
//   <div {@attach lw.attach}>
//     {#if win.top}<div style="height:{win.top}px" aria-hidden="true"></div>{/if}
//     {#each rows.slice(win.start, win.end) as r (r.key)}…{/each}
//     {#if win.bottom}<div style="height:{win.bottom}px" aria-hidden="true"></div>{/if}
//   </div>
//
// `{ selfScroll: true }` is the same window over a list that IS its scroller
// (`TableWindow`, lib/tableWindow.svelte.ts): `rel` is then the element's own
// scrollTop, fed by `onscroll`, and `measure()` re-reads the pitch.
//
// ⌘F over the WHOLE list (not just the mounted slice) is opt-in — one
// provider per window (lib/findProviders.ts). `text` lists the row's cells in
// DOM order; `rowSel` must match the rendered rows one-to-one, in order:
//   $effect(() => lw.findRows(() => listEl, () => rows, (r) => r.path));

import { tick, untrack } from 'svelte';
import { registerFindProvider } from './findProviders';

export interface ListRange {
  start: number;
  end: number;
  /** Spacer height above the slice (px). */
  top: number;
  /** Spacer height below the slice (px). */
  bottom: number;
}

/** An extra band of `h` px laid out right after row `after` (an expanded
 *  row's detail row): every later row sits `h` px lower. */
export interface ListExtra {
  after: number;
  h: number;
}

/** Pure window arithmetic (unit-tested): rows `[start, end)` of `n` rows of
 *  pitch `rowH` that intersect a viewport `[rel, rel + viewH)` measured from
 *  the list's own top, widened by `overscan` rows each way. `extra` (optional)
 *  inserts one taller band after a row; a viewport edge inside that band maps
 *  to the row that owns it, so the expanded row stays mounted while its
 *  detail is on screen. */
export function listRange(
  n: number,
  rowH: number,
  rel: number,
  viewH: number,
  overscan: number,
  min: number,
  extra?: ListExtra,
): ListRange {
  if (n <= min || rowH <= 0) return { start: 0, end: n, top: 0, bottom: 0 };
  const after = extra && extra.after >= 0 && extra.after < n ? extra.after : -1;
  const band = after >= 0 ? Math.max(0, extra!.h) : 0;
  const bandTop = (after + 1) * rowH;
  // y → fractional row position, with the band collapsed onto its owner row.
  const pos = (y: number): number => {
    if (band <= 0 || y < bandTop) return y / rowH;
    if (y < bandTop + band) return after + 0.5;
    return (y - band) / rowH;
  };
  const top = (i: number): number => i * rowH + (band > 0 && i > after ? band : 0);
  const first = Math.floor(pos(Math.max(0, rel)));
  const last = Math.ceil(pos(Math.max(0, rel + Math.max(viewH, 1))));
  const start = Math.max(0, Math.min(n, first - overscan));
  const end = Math.max(start, Math.min(n, last + overscan));
  return { start, end, top: top(start), bottom: top(n) - top(end) };
}

/** The measured pitch to adopt for a rendered row of height `h`, or null to
 *  keep `current` (tiny or sub-pixel differences are noise). */
export function nextPitch(h: number, current: number): number | null {
  return h > 4 && Math.abs(h - current) > 0.25 ? h : null;
}

/** Nearest ancestor that scrolls vertically; null ⇒ the window/document. */
export function findScroller(el: HTMLElement): HTMLElement | null {
  let p = el.parentElement;
  while (p && p !== document.body && p !== document.documentElement) {
    const oy = getComputedStyle(p).overflowY;
    if (oy === 'auto' || oy === 'scroll' || oy === 'overlay') return p;
    p = p.parentElement;
  }
  return null;
}

export interface ListWindowOptions {
  /** Render everything at or below this many rows (default 300). */
  min?: number;
  /** Extra rows mounted above and below the viewport (default 20). */
  overscan?: number;
  /** Starting pitch until a rendered row is measured (default 26). */
  rowH?: number;
  /** The list element is its own scroller (`TableWindow`). */
  selfScroll?: boolean;
}

export class ListWindow {
  /** Scroller scrollTop minus the list's offset inside the scroll content
   *  (a self-scrolling list: its own scrollTop). */
  rel = $state(0);
  viewH = $state(800);
  rowH = $state(26);
  readonly #min: number;
  readonly #overscan: number;
  readonly #selfScroll: boolean;
  /** Picks the rendered rows (pitch measurement, find-in-page mapping). */
  protected readonly rowSel: string;
  #raf = 0;
  #list: HTMLElement | null = null;
  #scroller: HTMLElement | null = null;

  /** `rowSel` picks a rendered row to measure the pitch from. */
  constructor(rowSel: string, opts: ListWindowOptions = {}) {
    this.rowSel = rowSel;
    this.#min = opts.min ?? 300;
    this.#overscan = opts.overscan ?? 20;
    this.#selfScroll = opts.selfScroll ?? false;
    this.rowH = opts.rowH ?? 26;
  }

  /** True when `n` rows are windowed (above `min`). Lets a list give its
   *  scroller a bounded height only while windowing, e.g. on phones where the
   *  page (not the list) would otherwise scroll and the slice never moves. */
  active(n: number): boolean {
    return n > this.#min;
  }

  range(n: number): ListRange {
    // A self-scrolling list reads its height from `bind:clientHeight`, which
    // is 0 until layout: assume a pane's worth rather than one row.
    const viewH = this.#selfScroll ? this.viewH || 600 : this.viewH;
    return listRange(n, this.rowH, this.rel, viewH, this.#overscan, this.#min);
  }

  /** Re-measure the row pitch from the first rendered row of `container`. */
  measure(container: HTMLElement | undefined | null, selector = this.rowSel): void {
    const row = container?.querySelector<HTMLElement>(selector);
    const next = row ? nextPitch(row.getBoundingClientRect().height, this.rowH) : null;
    if (next !== null) this.rowH = next;
  }

  /** Scroll handler for a self-scrolling list (rAF-coalesced). */
  onscroll = (e: Event): void => {
    const el = e.currentTarget as HTMLElement;
    if (this.#raf) return;
    this.#raf = requestAnimationFrame(() => {
      this.#raf = 0;
      this.rel = el.scrollTop;
    });
  };

  /** Back to the top (a new listing replaced the rows). */
  reset(container?: HTMLElement | null): void {
    this.rel = 0;
    if (container) container.scrollTop = 0;
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
    this.measure(list);
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

  /**
   * Register a find provider over every row (`rows()` is the full list the
   * window slices, `text` what row i shows). Sits out while the list isn't
   * windowed — then every row is mounted and the plain DOM walk is exact.
   * `container` is the list (a self-scrolling list: its scroller); an outer-
   * scroller list falls back to the attached element. `rowSelector` matches
   * the rendered rows, in order. Returns the unregister — call it from an
   * `$effect`.
   */
  findRows<T>(
    container: () => HTMLElement | undefined | null,
    rows: () => readonly T[],
    text: (row: T, i: number) => string,
    rowSelector = this.rowSel,
  ): () => void {
    const root = (): HTMLElement | null => container() ?? this.#list;
    return registerFindProvider({
      root,
      active: () => this.active(rows().length),
      count: () => rows().length,
      text: (i) => {
        const r = rows()[i];
        return r === undefined ? '' : text(r, i);
      },
      reveal: async (i) => {
        if (this.#selfScroll) {
          const el = root();
          if (!el) return;
          el.scrollTop = Math.max(0, i * this.rowH - (this.viewH || 600) / 2);
          this.rel = el.scrollTop;
        } else {
          this.reveal(i);
          this.#sync();
        }
        await tick();
      },
      rowElement: (i) => {
        const { start, end } = this.range(rows().length);
        if (i < start || i >= end) return null;
        return root()?.querySelectorAll(rowSelector)[i - start] ?? null;
      },
    });
  }
}
