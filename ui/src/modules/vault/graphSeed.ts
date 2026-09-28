// Layout carry-over for the vault graph (SD-03 step 3). When a REAL change
// (a new note, a moved link) refetches the graph, the view used to throw the
// whole layout away: every node re-seeded from the fixed random disc, the
// simulation restarted hot and the camera re-fit — the graph visibly
// "re-exploded" on every agent write. Instead the view sends the previous
// positions of surviving nodes (by path) with `init`; new nodes start next to
// a placed neighbour, and a mostly-placed graph starts warm (low alpha) so it
// only settles the local change. Pure — shared by the worker and unit tests.

/** Positions of `nextPaths` from the previous layout: [x0,y0,…], NaN = new. */
export function carrySeed(
  prevPaths: readonly string[],
  prevPos: Float32Array | null,
  nextPaths: readonly string[],
): { seed: Float32Array<ArrayBuffer>; known: number } | null {
  if (!prevPos || prevPos.length !== prevPaths.length * 2 || nextPaths.length === 0) return null;
  const at = new Map<string, number>();
  for (let i = 0; i < prevPaths.length; i++) at.set(prevPaths[i], i);
  const seed = new Float32Array(nextPaths.length * 2).fill(Number.NaN);
  let known = 0;
  for (let i = 0; i < nextPaths.length; i++) {
    const j = at.get(nextPaths[i]);
    if (j === undefined) continue;
    const x = prevPos[j * 2], y = prevPos[j * 2 + 1];
    if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
    seed[i * 2] = x;
    seed[i * 2 + 1] = y;
    known++;
  }
  return known ? { seed, known } : null;
}

/** Share of placed nodes at which the simulation starts warm, not hot. */
export const WARM_SHARE = 0.5;
/** Starting alpha for a mostly-placed graph (a fresh one starts at 1). */
export const WARM_ALPHA = 0.25;

/**
 * Overwrite the random seeding with carried positions (worker side). A new
 * node lands beside its first placed neighbour (small jitter), else keeps its
 * random seed. Returns how many nodes were placed from the carry.
 */
export function applySeed(
  px: Float32Array,
  py: Float32Array,
  seed: Float32Array,
  edges: Uint32Array,
  rnd: () => number,
): number {
  const n = px.length;
  if (seed.length !== n * 2) return 0;
  const placed = new Uint8Array(n);
  let known = 0;
  for (let i = 0; i < n; i++) {
    const x = seed[i * 2], y = seed[i * 2 + 1];
    if (Number.isFinite(x) && Number.isFinite(y)) {
      px[i] = x;
      py[i] = y;
      placed[i] = 1;
      known++;
    }
  }
  if (!known || known === n) return known;
  for (let e = 0; e + 1 < edges.length; e += 2) {
    const a = edges[e], b = edges[e + 1];
    if (a >= n || b >= n) continue;
    const [from, to] = placed[a] === 1 && !placed[b] ? [a, b] : placed[b] === 1 && !placed[a] ? [b, a] : [-1, -1];
    if (from < 0) continue;
    px[to] = px[from] + (rnd() - 0.5) * 30;
    py[to] = py[from] + (rnd() - 0.5) * 30;
    placed[to] = 2; // placed beside a neighbour (not a carry source itself)
  }
  return known;
}
