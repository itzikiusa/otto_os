// Variable-height windowing primitives for DiffViewer.

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

export { findScroller } from '../listWindow.svelte';
