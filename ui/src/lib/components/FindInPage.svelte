<script lang="ts">
  // Global find-in-page overlay. Windowed views (diff, DB grid, logs, long
  // chats, VirtualList users that opt in) register a row-model provider
  // (lib/findProviders.ts) so matches outside their mounted window count too.
  // Uses the CSS Custom Highlight API when available; falls back to a
  // scroll-to-first-match approach on older WebViews.
  //
  // Mount once in App.svelte next to <ContextMenu />.
  // Opened via the `findInPage` store (Cmd+F when no terminal is focused).

  import { untrack } from 'svelte';
  import { findInPage } from '../findinpage.svelte';
  import Icon from './Icon.svelte';
  import {
    activeFindProviders,
    countOccurrences,
    findSkipped,
    locateInElement,
    releaseFindProviders,
    type FindProvider,
  } from '../findProviders';

  // ---- state ----
  let query = $state('');
  let currentIdx = $state(0);
  let totalCount = $state(0);
  /** The walk stopped at MAX_MATCHES — the label reads "N+". */
  let truncated = $state(false);
  let inputEl: HTMLInputElement | null = $state(null);
  let searching = $state(false);
  let searchError = $state('');
  let searchSequence = 0;
  let searchAbort: AbortController | null = null;

  /** Matches past this aren't collected: a one-letter query on a 100k-line
   *  diff would otherwise build (and highlight) hundreds of thousands. */
  const MAX_MATCHES = 5000;

  // ---- feature detection ----
  const supportsHighlight =
    typeof CSS !== 'undefined' &&
    typeof (CSS as unknown as { highlights?: unknown }).highlights !== 'undefined' &&
    typeof Highlight !== 'undefined';
  const supportsStatic = typeof StaticRange !== 'undefined';

  // DOM matches in document order. StaticRange, not Range: the document
  // tracks every live Range and walks them all on EACH DOM mutation anywhere
  // in the app (5.7 ms per mutation at 30k matches). A static range goes stale
  // if its text re-renders — next()/prev() re-search when that happens.
  let ranges: AbstractRange[] = [];
  // Matches inside WINDOWED views (lib/findProviders.ts), found in their row
  // models — mounted or not. They come first in the match order; index
  // `rowMatches.length + k` is DOM match `k`.
  type RowMatch = { p: FindProvider; row: number; nth: number };
  let rowMatches: RowMatch[] = [];
  /** The lower-cased query the current matches were collected for. */
  let searched = '';
  /** The range painted as "current" (a DOM match, or a located row match). */
  let currentRange: AbstractRange | null = null;

  function dropRanges(): void {
    navSeq++;
    searchSequence++;
    searchAbort?.abort();
    searching = false;
    searchError = '';
    clearHighlights();
    ranges = [];
    rowMatches = [];
    currentRange = null;
    releaseFindProviders();
  }

  // ---- open / close reactions ----
  $effect(() => {
    if (findInPage.open) {
      // Focus the input on next microtask so the bar is rendered first.
      queueMicrotask(() => inputEl?.focus());
      // Run search in case a previous query is still in state. Untracked: the
      // effect must not re-run (and search undebounced) on every keystroke.
      untrack(() => {
        if (query) runSearch();
      });
      // A windowed view mounts other rows as it scrolls: repaint the visible
      // provider matches (one walk of the mounted rows per frame, at most).
      const onScroll = (): void => {
        if (rowMatches.length > 0 && !paintFrame) paintFrame = requestAnimationFrame(repaint);
      };
      document.addEventListener('scroll', onScroll, { capture: true, passive: true });
      return () => {
        document.removeEventListener('scroll', onScroll, { capture: true });
        if (paintFrame) cancelAnimationFrame(paintFrame);
        paintFrame = 0;
      };
    } else {
      // Hidden by any path: release the matches, not just their paint.
      dropRanges();
    }
  });
  let paintFrame = 0;
  function repaint(): void {
    paintFrame = 0;
    applyHighlights();
  }

  // ---- debounced search on query change ----
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  function onQueryInput(e: Event): void {
    query = (e.target as HTMLInputElement).value;
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(runSearch, 120);
  }

  // ---- content roots ----
  // The page pane plus overlays that visually sit on top of / beside it: the
  // agents right panel (.rpanel) and any open modal sheet — a browser ⌘F would
  // reach those too. Chrome (Navigator, tab strip, toolbars) stays excluded so
  // nav labels don't pollute the match list.
  function getContentRoots(): Element[] {
    const all = [...document.querySelectorAll('.content, .rpanel, .sheet[role="dialog"]')];
    // Drop roots nested inside another selected root (e.g. a modal mounted
    // within .content) so their text isn't walked — and matched — twice.
    const roots = all.filter((el) => !all.some((other) => other !== el && other.contains(el)));
    return roots.length > 0 ? roots : [document.getElementById('app') ?? document.body];
  }

  function makeRange(node: Text, start: number, end: number): AbstractRange {
    if (supportsStatic) return new StaticRange({ startContainer: node, startOffset: start, endContainer: node, endOffset: end });
    const r = document.createRange();
    r.setStart(node, start);
    r.setEnd(node, end);
    return r;
  }

  /** Hidden text: `display:none` / `visibility:hidden` on the text's element
   *  OR ANY ANCESTOR (the old check read only the direct parent's style, so
   *  text inside a hidden ancestor still matched). `offsetParent === null` is
   *  the cheap pre-filter: it is non-null for anything laid out normally. */
  function hiddenText(parent: Element): boolean {
    if ((parent as HTMLElement).offsetParent !== null || parent.tagName === 'BODY') return false;
    const check = (parent as Element & { checkVisibility?: (o?: object) => boolean }).checkVisibility;
    if (typeof check === 'function') return !check.call(parent, { visibilityProperty: true });
    for (let el: Element | null = parent; el && el !== document.body; el = el.parentElement) {
      const style = getComputedStyle(el);
      if (style.display === 'none' || style.visibility === 'hidden') return true;
      if (style.position === 'fixed') return false;
    }
    return false;
  }

  /** Walk `root`'s text, collecting ranges; `reject` subtrees are skipped, and
   *  so are `findSkipped` ones when `skipMarked` (painting a provider root's
   *  mounted text — only what its model counts may light up). */
  function walkText(
    root: Element,
    lower: string,
    into: AbstractRange[],
    cap: number,
    reject: Set<Element>,
    checkHidden: boolean,
    skipMarked = false,
  ): boolean {
    const bar = document.querySelector('.otto-find-bar');
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        if (node.nodeType === Node.ELEMENT_NODE) {
          // Whole subtrees skipped once at their root (was a closest() walk
          // up from every text node).
          const el = node as Element;
          const tag = el.localName;
          if (el === bar || tag === 'script' || tag === 'style' || reject.has(el)) return NodeFilter.FILTER_REJECT;
          if (skipMarked && findSkipped(el)) return NodeFilter.FILTER_REJECT;
          return NodeFilter.FILTER_SKIP;
        }
        const parent = node.parentElement;
        if (!parent) return NodeFilter.FILTER_REJECT;
        if (checkHidden && hiddenText(parent)) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    let textNode: Text | null;
    while ((textNode = walker.nextNode() as Text | null)) {
      const content = textNode.data;
      // Cheap pre-check before lowercasing the node (most nodes don't match).
      if (content.length < lower.length) continue;
      const contentLower = content.toLowerCase();
      let pos = 0;
      while ((pos = contentLower.indexOf(lower, pos)) !== -1) {
        into.push(makeRange(textNode, pos, pos + lower.length));
        if (into.length >= cap) return true;
        pos += lower.length;
      }
    }
    return false;
  }

  // ---- core search ----
  async function runSearch(): Promise<void> {
    navSeq++;
    const sequence = ++searchSequence;
    searchAbort?.abort();
    const controller = new AbortController();
    searchAbort = controller;
    searchError = '';
    searching = false;
    clearHighlights();
    truncated = false;
    currentRange = null;
    if (!query) {
      totalCount = 0;
      currentIdx = 0;
      ranges = [];
      rowMatches = [];
      searched = '';
      return;
    }

    const lower = query.toLowerCase();
    searched = lower;

    // 1) Windowed views: search their whole row models.
    const active = activeFindProviders();
    const current = () => sequence === searchSequence && !controller.signal.aborted && query.toLowerCase() === lower && findInPage.open;
    const sameProviders = () => {
      const now = activeFindProviders();
      return now.length === active.length && now.every((entry, i) => entry.provider === active[i].provider && entry.root === active[i].root);
    };
    const rows: RowMatch[] = [];
    searching = active.some(({ provider }) => !!provider.search);
    if (searching) { rowMatches = []; ranges = []; totalCount = 0; currentIdx = -1; }
    try {
      outer: for (const { provider } of active) {
        if (provider.search) {
          const matches = await provider.search(lower, MAX_MATCHES - rows.length, controller.signal);
          if (!current()) return;
          if (!sameProviders()) { void runSearch(); return; }
          for (const match of matches) {
            for (let nth = 0; nth < match.count && rows.length < MAX_MATCHES; nth++) rows.push({ p: provider, row: match.row, nth });
            if (rows.length >= MAX_MATCHES) { truncated = true; break outer; }
          }
        } else {
          const n = provider.count();
          for (let i = 0; i < n; i++) {
            const hits = countOccurrences(provider.text(i), lower, MAX_MATCHES - rows.length, provider.lowered);
            for (let k = 0; k < hits; k++) rows.push({ p: provider, row: i, nth: k });
            if (rows.length >= MAX_MATCHES) { truncated = true; break outer; }
          }
        }
      }
    } catch (e) {
      if (current()) {
        if (!sameProviders()) { void runSearch(); return; }
        searchError = e instanceof Error ? e.message : String(e);
      }
      return;
    } finally {
      if (sequence === searchSequence) searching = false;
    }

    // 2) The rest of the DOM, minus the provider roots (already counted).
    const found: AbstractRange[] = [];
    const reject = new Set(active.map((a) => a.root));
    if (!truncated) {
      for (const root of getContentRoots()) {
        if (walkText(root, lower, found, MAX_MATCHES - rows.length, reject, true)) {
          truncated = true;
          break;
        }
      }
    }

    rowMatches = rows;
    ranges = found;
    totalCount = rows.length + found.length;
    currentIdx = totalCount > 0 ? 0 : -1;
    applyHighlights();
    if (totalCount > 0) void goTo(0);
  }

  // ---- highlight helpers ----
  type HighlightRegistry = { set: (k: string, v: unknown) => void; delete: (k: string) => void };
  const registry = (): HighlightRegistry => (CSS as unknown as { highlights: HighlightRegistry }).highlights;

  /** Paint every DOM match plus the provider matches that are MOUNTED now. */
  function applyHighlights(): void {
    if (!supportsHighlight) return;
    const hl = registry();
    if (ranges.length === 0 && rowMatches.length === 0) {
      hl.delete('otto-find');
      hl.delete('otto-find-current');
      return;
    }
    const all = new Highlight();
    for (const r of ranges) all.add(r);
    if (rowMatches.length > 0 && searched) {
      const mounted: AbstractRange[] = [];
      for (const { root } of activeFindProviders()) {
        if (walkText(root, searched, mounted, MAX_MATCHES, new Set(), false, true)) break;
      }
      for (const r of mounted) all.add(r);
    }
    hl.set('otto-find', all);
    applyCurrent();
  }

  /** Repaint only the current match (next / prev). */
  function applyCurrent(): void {
    if (!supportsHighlight) return;
    const hl = registry();
    if (currentRange) hl.set('otto-find-current', new Highlight(currentRange));
    else hl.delete('otto-find-current');
  }

  function clearHighlights(): void {
    if (!supportsHighlight) return;
    const hl = registry();
    hl.delete('otto-find');
    hl.delete('otto-find-current');
  }

  const nextFrame = (): Promise<void> => new Promise((r) => requestAnimationFrame(() => r()));

  // ---- navigation ----
  let navSeq = 0;
  /** Move to match `idx`. A windowed-row match is revealed first (its view
   *  scrolls the row into its window), then located among the row's mounted
   *  text. A DOM match whose text node was re-rendered away (static ranges
   *  don't follow DOM edits) re-searches and lands as close as possible. */
  async function goTo(idx: number): Promise<void> {
    const seq = ++navSeq;
    const search = searchSequence, controller = searchAbort;
    const current = () => seq === navSeq && search === searchSequence && !controller?.signal.aborted && findInPage.open;
    if (idx < rowMatches.length) {
      const m = rowMatches[idx];
      if (m.row >= m.p.count()) {
        await runSearch(); // the model shrank under the match list
        return;
      }
      currentIdx = idx;
      let el = m.p.rowElement(m.row);
      if (!el) {
        await m.p.reveal(m.row, controller?.signal);
        if (!current()) return;
        await nextFrame();
        if (!current()) return;
        el = m.p.rowElement(m.row);
      }
      if (!current()) return;
      const loc = el ? locateInElement(el, searched, m.nth) : null;
      currentRange = loc ? makeRange(loc.node, loc.offset, loc.offset + searched.length) : null;
      // The window moved: repaint what is mounted now, then the current one.
      applyHighlights();
      applyCurrent();
      // Instant, not smooth: a smooth scroll across a windowed view mounts
      // every row on the way.
      (loc?.node.parentElement ?? el)?.scrollIntoView({ block: 'center' });
      return;
    }
    let k = idx - rowMatches.length;
    if (!ranges[k]?.startContainer.isConnected) {
      await runSearch();
      if (totalCount === 0 || ranges.length === 0) return;
      k = Math.min(Math.max(0, k), ranges.length - 1);
    }
    currentIdx = rowMatches.length + k;
    currentRange = ranges[k];
    applyCurrent();
    ranges[k].startContainer.parentElement?.scrollIntoView({ block: 'center', behavior: 'smooth' });
  }

  function next(): void {
    if (totalCount === 0) return;
    void goTo((currentIdx + 1) % totalCount);
  }

  function prev(): void {
    if (totalCount === 0) return;
    void goTo((currentIdx - 1 + totalCount) % totalCount);
  }

  function close(): void {
    clearHighlights();
    ranges = [];
    rowMatches = [];
    currentRange = null;
    totalCount = 0;
    currentIdx = 0;
    query = '';
    findInPage.hide();
  }

  // ---- keyboard handling inside the bar ----
  function onKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey) prev();
      else next();
    }
  }

  // ---- display label ----
  const countLabel = $derived(
    searching ? 'Searching…' : searchError ? 'Search unavailable' : totalCount === 0
      ? query
        ? '0 results'
        : ''
      : `${currentIdx + 1} / ${totalCount}${truncated ? '+' : ''}`,
  );
</script>

{#if findInPage.open}
  <div class="otto-find-bar" role="search" aria-label="Find in page">
    <input
      bind:this={inputEl}
      class="find-input"
      type="text"
      placeholder="Find…"
      value={query}
      oninput={onQueryInput}
      onkeydown={onKeyDown}
      aria-label="Search query"
      autocomplete="off"
      spellcheck={false}
    />

    {#if query}
      <span class="find-count" aria-live="polite" aria-atomic="true" title={searchError || undefined}>{countLabel}</span>
      {#if searchError}<button class="btn small" onclick={() => void runSearch()}>Retry</button>{/if}
    {/if}

    <button
      class="find-nav-btn"
      onclick={prev}
      disabled={totalCount === 0}
      title="Previous match (Shift+Enter)"
      aria-label="Previous match"
    >
      <Icon name="chevronUp" size={12} />
    </button>
    <button
      class="find-nav-btn"
      onclick={next}
      disabled={totalCount === 0}
      title="Next match (Enter)"
      aria-label="Next match"
    >
      <Icon name="chevronDown" size={12} />
    </button>
    <button class="find-close-btn" onclick={close} title="Close (Esc)" aria-label="Close find bar">
      <Icon name="x" size={11} />
    </button>
  </div>
{/if}

<style>
  .otto-find-bar {
    position: fixed;
    top: 40px; /* below the tab bar / titlebar area */
    inset-inline-end: 12px;
    z-index: var(--z-find);
    display: flex;
    align-items: center;
    gap: 4px;
    height: 34px;
    padding: 0 6px;
    /* Opaque (it hosts a field) but one family with every floating layer. */
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-m);
    box-shadow: var(--glass-shadow);
  }

  .find-input {
    width: 200px;
    height: 24px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    font-family: var(--font-ui);
    outline: none;
    transition: border-color 130ms ease-out, box-shadow 130ms ease-out;
  }
  .find-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .find-input::placeholder {
    color: color-mix(in srgb, var(--text-dim) 70%, transparent);
  }

  .find-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    min-width: 44px;
    text-align: center;
  }

  .find-nav-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
    transition: background 130ms ease-out, color 130ms ease-out;
    padding: 0;
    line-height: 1;
  }
  .find-nav-btn:hover:not(:disabled) {
    background: var(--surface);
    color: var(--text);
  }
  .find-nav-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .find-close-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-s);
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    transition: background 130ms ease-out, color 130ms ease-out;
    padding: 0;
    margin-inline-start: 2px;
  }
  .find-close-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 15%, transparent);
    color: var(--text);
  }
</style>
