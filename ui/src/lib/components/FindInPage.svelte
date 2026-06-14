<script lang="ts">
  // Global find-in-page overlay.
  // Uses the CSS Custom Highlight API when available; falls back to a
  // scroll-to-first-match approach on older WebViews.
  //
  // Mount once in App.svelte next to <ContextMenu />.
  // Opened via the `findInPage` store (Cmd+F when no terminal is focused).

  import { findInPage } from '../findinpage.svelte';

  // ---- state ----
  let query = $state('');
  let currentIdx = $state(0);
  let totalCount = $state(0);
  let inputEl: HTMLInputElement | null = $state(null);

  // ---- feature detection ----
  const supportsHighlight =
    typeof CSS !== 'undefined' &&
    typeof (CSS as unknown as { highlights?: unknown }).highlights !== 'undefined' &&
    typeof Highlight !== 'undefined';

  // All matched ranges in document order.
  let ranges: Range[] = [];

  // ---- open / close reactions ----
  $effect(() => {
    if (findInPage.open) {
      // Focus the input on next microtask so the bar is rendered first.
      queueMicrotask(() => inputEl?.focus());
      // Run search in case a previous query is still in state.
      if (query) runSearch();
    } else {
      clearHighlights();
    }
  });

  // ---- debounced search on query change ----
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  function onQueryInput(e: Event): void {
    query = (e.target as HTMLInputElement).value;
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(runSearch, 120);
  }

  // ---- content root ----
  function getContentRoot(): Element {
    return document.querySelector('.content') ?? document.getElementById('app') ?? document.body;
  }

  // ---- core search ----
  function runSearch(): void {
    clearHighlights();
    if (!query) {
      totalCount = 0;
      currentIdx = 0;
      ranges = [];
      return;
    }

    const root = getContentRoot();
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const parent = node.parentElement;
        if (!parent) return NodeFilter.FILTER_REJECT;
        // Skip find bar itself, script, style, and hidden elements.
        if (parent.closest('.otto-find-bar')) return NodeFilter.FILTER_REJECT;
        const tag = parent.tagName.toLowerCase();
        if (tag === 'script' || tag === 'style') return NodeFilter.FILTER_REJECT;
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

    const lower = query.toLowerCase();
    const found: Range[] = [];

    let textNode: Text | null;
    while ((textNode = walker.nextNode() as Text | null)) {
      const content = textNode.textContent ?? '';
      const contentLower = content.toLowerCase();
      let pos = 0;
      while ((pos = contentLower.indexOf(lower, pos)) !== -1) {
        const range = document.createRange();
        range.setStart(textNode, pos);
        range.setEnd(textNode, pos + lower.length);
        found.push(range);
        pos += lower.length;
      }
    }

    ranges = found;
    totalCount = found.length;
    currentIdx = found.length > 0 ? 0 : -1;
    applyHighlights();
    scrollToCurrent();
  }

  // ---- highlight helpers ----
  function applyHighlights(): void {
    if (!supportsHighlight) return;
    const hl = (CSS as unknown as { highlights: Map<string, unknown> }).highlights;
    if (ranges.length === 0) {
      hl.delete('otto-find');
      hl.delete('otto-find-current');
      return;
    }
    // All matches.
    (hl as unknown as { set: (k: string, v: unknown) => void }).set(
      'otto-find',
      new Highlight(...ranges),
    );
    // Current match.
    if (currentIdx >= 0 && currentIdx < ranges.length) {
      (hl as unknown as { set: (k: string, v: unknown) => void }).set(
        'otto-find-current',
        new Highlight(ranges[currentIdx]),
      );
    }
  }

  function clearHighlights(): void {
    if (!supportsHighlight) return;
    const hl = (CSS as unknown as { highlights: Map<string, unknown> }).highlights;
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
  function next(): void {
    if (totalCount === 0) return;
    currentIdx = (currentIdx + 1) % totalCount;
    applyHighlights();
    scrollToCurrent();
  }

  function prev(): void {
    if (totalCount === 0) return;
    currentIdx = (currentIdx - 1 + totalCount) % totalCount;
    applyHighlights();
    scrollToCurrent();
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
      : `${currentIdx + 1} / ${totalCount}`,
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
      ↑
    </button>
    <button
      class="find-nav-btn"
      onclick={next}
      disabled={totalCount === 0}
      title="Next match (Enter)"
      aria-label="Next match"
    >
      ↓
    </button>
    <button class="find-close-btn" onclick={close} title="Close (Esc)" aria-label="Close find bar">
      ✕
    </button>
  </div>
{/if}

<style>
  .otto-find-bar {
    position: fixed;
    top: 40px; /* below the tab bar / titlebar area */
    right: 12px;
    z-index: 9000;
    display: flex;
    align-items: center;
    gap: 4px;
    height: 34px;
    padding: 0 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow:
      0 4px 16px rgba(0, 0, 0, 0.22),
      0 1px 4px rgba(0, 0, 0, 0.12);
  }

  .find-input {
    width: 200px;
    height: 24px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12.5px;
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
    font-size: 11px;
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
    font-size: 12px;
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
    font-size: 11px;
    cursor: pointer;
    transition: background 130ms ease-out, color 130ms ease-out;
    padding: 0;
    margin-left: 2px;
  }
  .find-close-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 15%, transparent);
    color: var(--text);
  }
</style>
