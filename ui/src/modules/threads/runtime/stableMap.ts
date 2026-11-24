// A derived Map that says the same keeps its old identity: every row that
// reads it (`map.get(row.id)`) then stays put when a streamed chunk changes
// nothing it shows — a new Map each chunk re-ran every row of the timeline.

export function keepIfSame<K, V>(prev: Map<K, V> | undefined, next: Map<K, V>, eq: (a: V, b: V) => boolean = Object.is): Map<K, V> {
  if (!prev || prev.size !== next.size) return next;
  for (const [k, v] of next) {
    if (!prev.has(k) || !eq(prev.get(k) as V, v)) return next;
  }
  return prev;
}

/** Arrays with the same items (by identity) in the same order. */
export const sameItems = <T>(a: readonly T[], b: readonly T[]): boolean => a.length === b.length && a.every((x, i) => x === b[i]);
