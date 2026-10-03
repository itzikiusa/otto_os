// Pure half of canvasFiles.ts's fetch cache (no imports, so `node --test` can
// load it): one in-flight/resolved data URL per sha, shared by every open
// board, bounded two ways so a session of image-heavy boards doesn't pin
// every multi-MB base64 string for the page's lifetime:
//
//   1. by bytes — a byte-weighted LRU (`text.length`; data URLs are ASCII)
//      evicts the least-recently-used resolved entries once the total passes
//      `maxBytes`. The newest entry always stays, even if it alone is over.
//   2. by owner — each board passes an owner token; `release(owner)` (on
//      unmount) drops every entry no other open board still uses.
//
// An evicted/released sha is simply refetched on the next `get` — the route is
// immutable, so the webview's HTTP cache usually serves it.

export const DEFAULT_FILE_CACHE_BYTES = 64 * 1024 * 1024;

type Entry = { p: Promise<string>; bytes: number; owners: Set<unknown> };

export class FileCache {
  // Map iteration order = recency (a hit re-inserts at the end).
  private readonly entries = new Map<string, Entry>();
  private total = 0;
  // Plain fields, not parameter properties: `node --test` only strips types.
  private readonly fetcher: (sha: string) => Promise<string>;
  private readonly maxBytes: number;

  constructor(fetcher: (sha: string) => Promise<string>, maxBytes = DEFAULT_FILE_CACHE_BYTES) {
    this.fetcher = fetcher;
    this.maxBytes = maxBytes;
  }

  /** A file's data URL by sha, recorded as used by `owner` (failures aren't cached). */
  get(sha: string, owner: unknown): Promise<string> {
    const hit = this.entries.get(sha);
    if (hit) {
      hit.owners.add(owner);
      this.entries.delete(sha);
      this.entries.set(sha, hit);
      return hit.p;
    }
    const entry: Entry = { p: this.fetcher(sha), bytes: 0, owners: new Set([owner]) };
    this.entries.set(sha, entry);
    entry.p.then(
      (text) => {
        if (this.entries.get(sha) !== entry) return; // released/evicted mid-flight
        entry.bytes = text.length;
        this.total += entry.bytes;
        this.evict(sha);
      },
      () => {
        if (this.entries.get(sha) === entry) this.entries.delete(sha);
      },
    );
    return entry.p;
  }

  /** A board closed: drop every entry no other open board uses. */
  release(owner: unknown): void {
    for (const [sha, e] of this.entries) {
      if (!e.owners.delete(owner) || e.owners.size) continue;
      this.drop(sha, e);
    }
  }

  /** Resolved bytes currently held. */
  get bytes(): number {
    return this.total;
  }

  has(sha: string): boolean {
    return this.entries.has(sha);
  }

  private evict(keep: string): void {
    for (const [sha, e] of this.entries) {
      if (this.total <= this.maxBytes) return;
      if (sha === keep || !e.bytes) continue; // newest / still in flight
      this.drop(sha, e);
    }
  }

  private drop(sha: string, e: Entry): void {
    this.entries.delete(sha);
    this.total -= e.bytes;
  }
}
