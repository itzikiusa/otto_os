// Variable-height windowing primitives for DiffViewer. Heights live in a
// Float64Array aligned to the row model (estimates first, then measured), with
// a prefix-sum `offsets` array (offsets[i] = top of row i, offsets[n] = total).
// A measurement patches one height and re-sums from that index only — a flat
// O(n) float loop (~0.1 ms at 124k rows), so scrolling never walks Maps.

/** First index i with offsets[i+1] > y (the row containing y). */
export function rowAt(offsets: Float64Array, n: number, y: number): number {
  let lo = 0;
  let hi = n - 1;
  if (n <= 0) return 0;
  if (y <= 0) return 0;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid + 1] <= y) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** Recompute offsets[from+1 .. n] from heights. */
export function resum(heights: Float64Array, offsets: Float64Array, from: number): void {
  const n = heights.length;
  let acc = offsets[from];
  for (let i = from; i < n; i++) {
    acc += heights[i];
    offsets[i + 1] = acc;
  }
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
