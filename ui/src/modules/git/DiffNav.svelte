<script lang="ts">
  // DiffViewer's file navigator (PR mode, shown when the file tree is on):
  // the shared changed-files tree (DiffFileTree, also the PR workspace's
  // Files panel), its own filter field (bound to the viewer's search), a
  // collapse-to-rail button and a drag / keyboard-resizable width.
  import Icon from '../../lib/components/Icon.svelte';
  import { plural } from '../../lib/plural';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { paneResizer } from '../../lib/paneResizer';
  import DiffFileTree from './DiffFileTree.svelte';
  import type { NavDir } from './diff-pr';

  interface Props {
    tree: NavDir;
    /** Paths matching the viewer's search (null = no search). */
    matchSet: Set<string> | null;
    viewed: Set<string>;
    commentCounts: Map<string, number>;
    totalFiles: number;
    viewedCount: number;
    matchCount: number;
    searching: boolean;
    /** Visible height of the diff's scroll container (caps the rail). */
    maxH: number;
    rawSearch: string;
    onjump: (path: string) => void;
    ontoggleviewed: (path: string) => void;
  }
  let {
    tree,
    matchSet,
    viewed,
    commentCounts,
    totalFiles,
    viewedCount,
    matchCount,
    searching,
    maxH,
    rawSearch = $bindable(),
    onjump,
    ontoggleviewed,
  }: Props = $props();

  // Drag-resizable nav width (desktop), persisted. The ≤1024 media query still
  // wins via !important-free specificity because the width is a CSS var the
  // mobile rules simply ignore.
  let navCollapsed = $state(false);
  const NAV_W_KEY = 'otto_diffnav_w';
  let navW = $state(Number(localStorage.getItem(NAV_W_KEY)) || 240);
  let navResizing = $state(false);
  const clampNavW = (w: number): number => Math.max(180, Math.min(520, Math.round(w)));
  function persistNavW(): void {
    try {
      localStorage.setItem(NAV_W_KEY, String(navW));
    } catch {
      /* blocked storage: the width just doesn't persist */
    }
  }
  function setNavW(w: number): void {
    navW = clampNavW(w);
    persistNavW();
  }
  // Through lib/dragCursor (r3-03-08): an overlay carries the cursor instead
  // of an INHERITED `body.style` write (65–690 ms of whole-document restyle
  // on this very page, at drag start and end), moves are coalesced to one
  // per frame, and localStorage is written once on release, not per move.
  function startNavResize(e: MouseEvent): void {
    navResizing = true;
    const startX = e.clientX;
    const startW = navW;
    startMouseDrag(e, {
      cursor: 'col-resize',
      // Physical left-anchored sidebar: dragging right widens it.
      onMove: (ev) => {
        navW = clampNavW(startW + (ev.clientX - startX));
      },
      onEnd: () => {
        navResizing = false;
        persistNavW();
      },
    });
  }

</script>

    <aside
      class="diff-nav"
      class:nav-collapsed={navCollapsed}
      class:nav-resizing={navResizing}
      style={`--nav-max:${Math.max(160, maxH)}px;${navCollapsed ? '' : `--navw:${navW}px`}`}
    >
      <div class="nav-header">
        {#if !navCollapsed}
          <span class="nav-title">
            {plural(totalFiles, 'file')}
            <span class="nav-viewed-count">· {viewedCount}/{totalFiles} viewed</span>
          </span>
        {/if}
        <button
          class="nav-collapse-btn"
          onclick={() => (navCollapsed = !navCollapsed)}
          title={navCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          aria-label={navCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          <!-- Physical chevron mirrored in RTL (the rail sits on the start side). -->
          <span class="nav-collapse-ico">
            <Icon name={navCollapsed ? 'chevronRight' : 'chevronLeft'} size={12} />
          </span>
        </button>
      </div>

      {#if !navCollapsed}
        <!-- Search inside nav -->
        <div class="nav-search-wrap">
          <input dir="ltr"
            class="nav-search input"
            type="text"
            placeholder="Filter files…"
            bind:value={rawSearch}
            aria-label="Filter files by path or content"
          />
          {#if searching}
            <span class="nav-match-count">{matchCount}</span>
          {/if}
        </div>

        <div class="nav-files">
          <DiffFileTree {tree} {matchSet} {viewed} {commentCounts} {onjump} {ontoggleviewed} />
        </div>
        <!-- Drag the trailing edge to resize (desktop); double-click resets. -->
        <!-- A focusable separator is the ARIA window-splitter widget (paneResizer adds ←/→, Home/End, Enter). -->
        <div
          class="nav-resize-handle"
          role="separator"
          aria-label="Resize file sidebar"
          title="Drag or use ←/→ to resize · double-click or Enter to reset"
          use:paneResizer={{ value: navW, min: 180, max: 520, onChange: setNavW, onReset: () => setNavW(240), onDragStart: startNavResize, text: (v) => `${Math.round(v)} pixels wide` }}
        ></div>
      {/if}
    </aside>

<style>
  /* ── Navigator sidebar ── */
  .diff-nav {
    /* Drag-resizable: --navw is set inline (persisted); default 240px. */
    width: var(--navw, 240px);
    min-width: var(--navw, 240px);
    max-width: var(--navw, 240px);
    position: relative;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    position: sticky;
    top: 0;
    /* Capped at the visible height of the diff's scroll container (measured
       in `readView`), not 100vh: in WKWebView 100vh is the SCREEN height, so
       the rail ran past the scrollport and its last files were unreachable
       until the diff itself was scrolled to the end. */
    max-height: var(--nav-max, 480px);
    overflow: hidden;
    /* Let the inner .nav-files (flex:1, overflow-y:auto) own the scroll: a flex
       column child can't scroll unless the column allows itself to shrink. */
    min-height: 0;
  }
  .diff-nav.nav-collapsed {
    width: 32px;
    min-width: 32px;
    max-width: 32px;
  }
  .nav-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px 6px;
    border-bottom: 1px solid var(--border);
    gap: 6px;
    flex-shrink: 0;
  }
  .nav-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }
  .nav-viewed-count {
    font-weight: 400;
  }
  .nav-collapse-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    padding: 2px 4px;
    border-radius: var(--radius-s);
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
  .nav-collapse-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nav-search-wrap {
    position: relative;
    padding: 6px 8px;
    flex-shrink: 0;
  }
  .nav-search {
    width: 100%;
    font-size: var(--fs-xs);
    height: 26px;
    padding: 0 6px;
    box-sizing: border-box;
  }
  .nav-match-count {
    position: absolute;
    inset-inline-end: 14px;
    top: 50%;
    transform: translateY(-50%);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    pointer-events: none;
  }
  .nav-files {
    overflow-y: auto;
    flex: 1;
    padding: 2px 0 8px;
  }
  /* Drag handle on the trailing edge of the sidebar. Fully INSIDE the aside —
     it has overflow:hidden, so any part hanging outside is unclickable. */
  .nav-resize-handle {
    position: absolute;
    inset-block: 0;
    inset-inline-end: 0;
    width: 6px;
    cursor: col-resize;
    z-index: 2;
  }
  .nav-resize-handle:hover,
  .nav-resize-handle:focus-visible,
  .nav-resizing .nav-resize-handle {
    outline: none;
    background: var(--accent-line);
  }
  .nav-collapse-ico {
    display: inline-flex;
  }
  /* Responsive: collapse nav to a thin rail on small screens */
  @media (max-width: 640px) {
    .diff-nav {
      width: 32px;
      min-width: 32px;
      max-width: 32px;
    }
  }


  @media (max-width: 1024px) {
    .diff-nav,
    .diff-nav.nav-collapsed {
      width: 100%;
      min-width: 0;
      max-width: 100%;
      position: static;
      max-height: 34vh;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    .nav-title { font-size: var(--fs-m); }
    .nav-search { font-size: var(--fs-m); height: 32px; }
    /* No drag-resize on touch layouts — the sidebar is full-width there. */
    .nav-resize-handle { display: none; }
    /* Sidebar collapse button. */
    .nav-collapse-btn { min-width: 36px; min-height: 36px; justify-content: center; }
  }
</style>
