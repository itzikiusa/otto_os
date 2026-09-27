<script lang="ts">
  // Global find-in-page overlay.
  // Uses the CSS Custom Highlight API when available; falls back to a
  // scroll-to-first-match approach on older WebViews.
  //
  // Mount once in App.svelte next to <ContextMenu />.
  // Opened via the `findInPage` store (Cmd+F when no terminal is focused).

  import { untrack } from 'svelte';
  import { findInPage } from '../findinpage.svelte';
  import Icon from './Icon.svelte';

  // ---- state ----
  let query = $state('');
  let currentIdx = $state(0);
  let totalCount = $state(0);
  /** The walk stopped at MAX_MATCHES — the label reads "N+". */
  let truncated = $state(false);
  let inputEl: HTMLInputElement | null = $state(null);

  /** Matches past this aren't collected: a one-letter query on a 100k-line
   *  diff would otherwise build (and highlight) hundreds of thousands. */
  const MAX_MATCHES = 5000;

  // ---- feature detection ----
  const supportsHighlight =
    typeof CSS !== 'undefined' &&
    typeof (CSS as unknown as { highlights?: unknown }).highlights !== 'undefined' &&
    typeof Highlight !== 'undefined';
  const supportsStatic = typeof StaticRange !== 'undefined';

  // All matched ranges in document order. StaticRange, not Range: the document
  // tracks every live Range and walks them all on EACH DOM mutation anywhere
  // in the app (5.7 ms per mutation at 30k matches). A static range goes stale
  // if its text re-renders — next()/prev() re-search when that happens.
  let ranges: AbstractRange[] = [];

  function dropRanges(): void {
    clearHighlights();
    ranges = [];
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
    } else {
      // Hidden by any path: release the matches, not just their paint.
      dropRanges();
    }
  });

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

  // ---- core search ----
  function runSearch(): void {
    clearHighlights();
    truncated = false;
    if (!query) {
      totalCount = 0;
      currentIdx = 0;
      ranges = [];
      return;
    }

    const lower = query.toLowerCase();
    const found: AbstractRange[] = [];
    const bar = document.querySelector('.otto-find-bar');

    outer: for (const root of getContentRoots()) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          if (node.nodeType === Node.ELEMENT_NODE) {
            // Whole subtrees skipped once at their root (was a closest() walk
            // up from every text node).
            const tag = (node as Element).localName;
            if (node === bar || tag === 'script' || tag === 'style') return NodeFilter.FILTER_REJECT;
            return NodeFilter.FILTER_SKIP;
          }
          const parent = node.parentElement;
          if (!parent) return NodeFilter.FILTER_REJECT;
          if ((parent as HTMLElement).offsetParent === null && parent.tagName !== 'BODY') {
            // hidden via display:none or visibility:hidden — skip
            const style = getComputedStyle(parent);
            if (style.display === 'none' || style.visibility === 'hidden') {
              return NodeFilter.FILTER_REJECT;
            }
          }
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
          found.push(makeRange(textNode, pos, pos + lower.length));
          if (found.length >= MAX_MATCHES) {
            truncated = true;
            break outer;
          }
          pos += lower.length;
        }
      }
    }

    ranges = found;
    totalCount = found.length;
    currentIdx = found.length > 0 ? 0 : -1;
    applyHighlights();
    scrollToCurrent();
  }

  // ---- highlight helpers ----
  type HighlightRegistry = { set: (k: string, v: unknown) => void; delete: (k: string) => void };
  const registry = (): HighlightRegistry => (CSS as unknown as { highlights: HighlightRegistry }).highlights;

  /** Paint every match (once per search). */
  function applyHighlights(): void {
    if (!supportsHighlight) return;
    const hl = registry();
    if (ranges.length === 0) {
      hl.delete('otto-find');
      hl.delete('otto-find-current');
      return;
    }
    const all = new Highlight();
    for (const r of ranges) all.add(r);
    hl.set('otto-find', all);
    applyCurrent();
  }

  /** Repaint only the current match (next / prev). */
  function applyCurrent(): void {
    if (!supportsHighlight) return;
    const hl = registry();
    if (currentIdx >= 0 && currentIdx < ranges.length) hl.set('otto-find-current', new Highlight(ranges[currentIdx]));
    else hl.delete('otto-find-current');
  }

  function clearHighlights(): void {
    if (!supportsHighlight) return;
    const hl = registry();
    hl.delete('otto-find');
    hl.delete('otto-find-current');
  }

  // ---- scroll to current match ----
  function scrollToCurrent(): void {
    if (currentIdx < 0 || currentIdx >= ranges.length) return;
    const range = ranges[currentIdx];
    const el = range.startContainer.parentElement;
    el?.scrollIntoView({ block: 'center', behavior: 'smooth' });
  }

  // ---- navigation ----
  /** Move to match `idx`; when its text node was re-rendered away (static
   *  ranges don't follow DOM edits), re-search and land as close as possible. */
  function goTo(idx: number): void {
    if (!ranges[idx]?.startContainer.isConnected) {
      runSearch();
      if (totalCount === 0) return;
      idx = Math.min(idx, totalCount - 1);
    }
    currentIdx = idx;
    applyCurrent();
    scrollToCurrent();
  }

  function next(): void {
    if (totalCount === 0) return;
    goTo((currentIdx + 1) % totalCount);
  }

  function prev(): void {
    if (totalCount === 0) return;
    goTo((currentIdx - 1 + totalCount) % totalCount);
  }

  function close(): void {
    clearHighlights();
    ranges = [];
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
    totalCount === 0
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
      <span class="find-count" aria-live="polite" aria-atomic="true">{countLabel}</span>
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
