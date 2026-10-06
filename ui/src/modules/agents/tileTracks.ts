// Tiled view track weights (TiledView.svelte). Weights are stored per grid
// SHAPE (`cols×rows`), but one shape covers several tile counts — 3×2 holds
// 5 or 6 tiles — so a stored row can be longer or shorter than the row on
// screen. The RENDERED weights are therefore always the normalized view of
// the stored ones, and every edit (drag, arrow-key nudge) must start from
// that view: copying the stored arrays verbatim wrote a 2-tile drag into a
// 3-entry row, which `normalize` then threw away, so the divider was dead
// (S14-302). Pure, so unit/tileTracks.test.ts can drive the pair math.

export type Tracks = { rows: number[]; cols: number[][] };

/** Positive finite weights of exactly `n` entries, else all-equal. */
export function normalize(fr: number[] | undefined, n: number): number[] {
  if (!Array.isArray(fr) || fr.length !== n || fr.some((f) => !Number.isFinite(f) || f <= 0)) return Array(n).fill(1);
  return fr;
}

/** The weights as rendered for a grid whose row `r` holds `rowLens[r]`
 *  tiles — a fresh copy, safe to edit and store back under the shape. */
export function liveTracks(stored: Tracks | undefined, rowLens: number[]): Tracks {
  return {
    rows: [...normalize(stored?.rows, rowLens.length)],
    cols: rowLens.map((n, r) => [...normalize(stored?.cols?.[r], n)]),
  };
}

