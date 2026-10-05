// Classrooms — kick-out bookkeeping the three.js scene (scene.ts) needs to
// keep honest, split out so the unit tests can drive it without WebGL.

/** Settle every in-flight kick-out whose student the new model no longer
 *  holds: `pose()` only walks ids still seated, so an orphaned kick would never
 *  reach `p ≥ 1` and its promise (the host's success toast) would hang forever.
 *  The delete already succeeded — the refetch dropping the row proves it — so
 *  the walk-out simply ends here. Returns the settled ids. */
export function settleKicks(kicks: Map<string, { resolve: () => void }>, keep: ReadonlySet<string>): string[] {
  const settled: string[] = [];
  for (const [id, k] of kicks) {
    if (keep.has(id)) continue;
    kicks.delete(id);
    k.resolve();
    settled.push(id);
  }
  return settled;
}
