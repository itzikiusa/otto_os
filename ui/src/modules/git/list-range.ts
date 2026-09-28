// Pure window arithmetic for `ListWindow` (list-window.svelte.ts), kept
// rune-free so `node --test` can exercise it directly.

export interface ListRange {
  start: number;
  end: number;
  /** Spacer height above the slice (px). */
  top: number;
  /** Spacer height below the slice (px). */
  bottom: number;
}

/** Pure window arithmetic (unit-tested): rows `[start, end)` of `n` rows of
 *  pitch `rowH` that intersect a viewport `[rel, rel + viewH)` measured from
 *  the list's own top, widened by `overscan` rows each way. */
export function listRange(n: number, rowH: number, rel: number, viewH: number, overscan: number, min: number): ListRange {
  if (n <= min || rowH <= 0) return { start: 0, end: n, top: 0, bottom: 0 };
  const first = Math.floor(Math.max(0, rel) / rowH);
  const last = Math.ceil(Math.max(0, rel + Math.max(viewH, 1)) / rowH);
  const start = Math.max(0, Math.min(n, first - overscan));
  const end = Math.max(start, Math.min(n, last + overscan));
  return { start, end, top: start * rowH, bottom: (n - end) * rowH };
}
