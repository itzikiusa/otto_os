// Small weighted LRU for IMMUTABLE commit-diff data (keyed by sha), shared by
// every GraphView instance. A commit's summary and its per-file patches never
// change, so re-clicking a branch — or switching tabs and back — paints from
// memory instead of re-running `git show` on the daemon. Bounded by weight
// (files / diff lines), never by entry count alone, so one huge commit can't
// pin tens of MB in the webview.
export class WeightedLru<V> {
  private map = new Map<string, { v: V; w: number }>();
  private total = 0;

  constructor(
    private readonly maxWeight: number,
    private readonly weigh: (v: V) => number,
  ) {}

  get(key: string): V | undefined {
    const hit = this.map.get(key);
    if (!hit) return undefined;
    // Re-insert = most recently used (Map keeps insertion order).
    this.map.delete(key);
    this.map.set(key, hit);
    return hit.v;
  }

  set(key: string, v: V): void {
    const w = Math.max(1, this.weigh(v));
    const old = this.map.get(key);
    if (old) {
      this.map.delete(key);
      this.total -= old.w;
    }
    // A single value bigger than the whole budget is never worth caching.
    if (w > this.maxWeight) return;
    this.map.set(key, { v, w });
    this.total += w;
    for (const [k, e] of this.map) {
      if (this.total <= this.maxWeight) break;
      this.map.delete(k);
      this.total -= e.w;
    }
  }
}

/** Per-(repo, sha) key; NUL can't appear in an id or a sha. */
export function diffKey(...parts: string[]): string {
  return parts.join('\0');
}
