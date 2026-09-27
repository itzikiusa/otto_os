// Bounded md → html memo for the conversation's `Markdown` blocks. A live
// delta rebuilds the growing response's blocks, and every `Markdown` whose
// input string did not change would otherwise re-run the whole pipeline
// (marked + hljs + DOMParser sanitize) for byte-identical output. LRU by Map
// insertion order (a hit is re-inserted); capped by entry count AND total
// characters so a transcript of 64 KB tool dumps can't pin megabytes.

export interface MdCacheOptions {
  /** Most entries kept. */
  maxEntries: number;
  /** Most md+html characters kept across all entries. */
  maxChars: number;
  /** Inputs longer than this are rendered but never cached. */
  maxEntryChars: number;
}

export interface MdCache {
  /** Cached html for `md`, rendering (and caching, when `cacheable`) on a miss. */
  get(md: string, cacheable?: boolean): string;
  readonly size: number;
  readonly chars: number;
  /** Renders (cache misses) so far — the perf spec's "≤ 1 parse per delta". */
  readonly renders: number;
}

export function createMdCache(render: (md: string) => string, opts: MdCacheOptions): MdCache {
  const map = new Map<string, string>();
  let chars = 0;
  let renders = 0;
  const evict = (): void => {
    for (const [k, v] of map) {
      if (map.size <= opts.maxEntries && chars <= opts.maxChars) break;
      map.delete(k);
      chars -= k.length + v.length;
    }
  };
  return {
    get(md: string, cacheable = true): string {
      const hit = map.get(md);
      if (hit !== undefined) {
        map.delete(md);
        map.set(md, hit);
        return hit;
      }
      const html = render(md);
      renders++;
      if (cacheable && md.length + html.length <= opts.maxEntryChars) {
        map.set(md, html);
        chars += md.length + html.length;
        evict();
      }
      return html;
    },
    get size() {
      return map.size;
    },
    get chars() {
      return chars;
    },
    get renders() {
      return renders;
    },
  };
}
