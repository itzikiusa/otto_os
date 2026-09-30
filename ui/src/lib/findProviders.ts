// Find-in-page providers (r3-03-01 / r3-10-03). A windowed view mounts only a
// slice of its rows, so ⌘F's DOM walk (FindInPage.svelte) saw ~one screen of
// a 100k-line diff or a 100k-row grid and reported "0 results" for text that
// is on the page. A windowed view registers a provider for its WHOLE row
// model instead:
//
//   $effect(() => registerFindProvider({
//     root: () => bodyEl,                // its DOM subtree — not walked
//     count: () => rows.length,
//     text: (i) => plainTextOf(rows[i]),  // what the row shows
//     reveal: async (i) => { scrollRowIntoWindow(i); await tick(); },
//     rowElement: (i) => bodyEl?.querySelector(`[data-rk="${key(i)}"]`) ?? null,
//   }));
//
// FindInPage counts matches over every provider's rows first, then walks the
// rest of the DOM (skipping provider roots, so nothing is counted twice).
// Navigating to a match outside the window calls `reveal(i)`, then maps the
// n-th occurrence in the row's text onto the n-th occurrence among the
// mounted row element's text nodes (subtrees marked `data-find-skip` — line
// numbers, gutters, pod tags — are ignored there) to highlight it. The same
// subtrees (plus CodeMirror's gutters / panels) are left unpainted when the
// mounted provider text is highlighted, so what lights up is what was counted.

export interface FindProvider {
  /** The view's DOM subtree. Its text is covered by the provider, so the DOM
   *  walk rejects it; a provider whose root is detached or hidden is ignored. */
  root(): Element | null;
  /** Rows the provider can search (the full model, mounted or not). */
  count(): number;
  /** The searchable text of row `i` (case is ignored). */
  text(i: number): string;
  /** Bring row `i` into the mounted window; resolve once it is rendered. */
  reveal(i: number): Promise<void> | void;
  /** The mounted element of row `i`, or null when it is not mounted. */
  rowElement(i: number): Element | null;
  /** Optional gate: `false` → the view isn't windowing right now (everything
   *  is mounted), so the plain DOM walk is exact and the provider sits out. */
  active?(): boolean;
  /** `text(i)` is already lower-cased (a provider caching its row texts
   *  across searches) — the search skips re-lowercasing every row. */
  lowered?: boolean;
  /** The find bar closed: drop whatever the provider cached for searching. */
  release?(): void;
}

const providers = new Set<FindProvider>();

/** Register a provider; returns the unregister (an `$effect` cleanup). */
export function registerFindProvider(p: FindProvider): () => void {
  providers.add(p);
  return () => {
    providers.delete(p);
  };
}

function visible(el: Element): boolean {
  const check = (el as Element & { checkVisibility?: (o?: object) => boolean }).checkVisibility;
  if (typeof check === 'function') return check.call(el, { visibilityProperty: true });
  return (el as HTMLElement).offsetParent !== null || getComputedStyle(el).position === 'fixed';
}

/** Let every provider drop its search caches (the find bar closed). */
export function releaseFindProviders(): void {
  for (const p of providers) p.release?.();
}

/** Subtrees that are neither counted nor painted: `[data-find-skip]` and
 *  CodeMirror's line-number gutters / search panels. */
export function findSkipped(el: Element): boolean {
  return (
    el.hasAttribute('data-find-skip') || el.classList.contains('cm-gutters') || el.classList.contains('cm-panels')
  );
}

/** Providers taking part in a search now, in document order of their roots. */
export function activeFindProviders(): { provider: FindProvider; root: Element }[] {
  const out: { provider: FindProvider; root: Element }[] = [];
  for (const p of providers) {
    const root = p.root();
    if (!root || !root.isConnected || (p.active && !p.active()) || !visible(root)) continue;
    out.push({ provider: p, root });
  }
  // A provider nested inside another one's root (a windowed tool output in
  // a windowed chat) is already covered by the outer model: drop it, so no
  // text is counted twice.
  const outer = out.filter((a) => !out.some((b) => b !== a && b.root !== a.root && b.root.contains(a.root)));
  outer.sort((a, b) => (a.root.compareDocumentPosition(b.root) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1));
  return outer;
}

/** Occurrences of `needleLower` in `text` (non-overlapping, case-insensitive;
 *  `lowered`: `text` is already lower-case). */
export function countOccurrences(text: string, needleLower: string, cap = Infinity, lowered = false): number {
  if (!needleLower || text.length < needleLower.length) return 0;
  const hay = lowered ? text : text.toLowerCase();
  let n = 0;
  let pos = 0;
  while (n < cap && (pos = hay.indexOf(needleLower, pos)) !== -1) {
    n++;
    pos += needleLower.length;
  }
  return n;
}

/** The `nth` (0-based) occurrence of `needleLower` among `el`'s text nodes,
 *  skipping `findSkipped` subtrees; falls back to the last one found.
 *  Returns the text node + offset, or null when the row shows no match. */
export function locateInElement(el: Element, needleLower: string, nth: number): { node: Text; offset: number } | null {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (node.nodeType === Node.ELEMENT_NODE) {
        return findSkipped(node as Element) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_SKIP;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let seen = 0;
  let last: { node: Text; offset: number } | null = null;
  let t: Text | null;
  while ((t = walker.nextNode() as Text | null)) {
    const hay = t.data.toLowerCase();
    let pos = 0;
    while ((pos = hay.indexOf(needleLower, pos)) !== -1) {
      last = { node: t, offset: pos };
      if (seen === nth) return last;
      seen++;
      pos += needleLower.length;
    }
  }
  return last;
}
