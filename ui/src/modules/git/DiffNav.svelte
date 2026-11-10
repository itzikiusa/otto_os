<script lang="ts">
  // DiffViewer's file navigator (PR mode, shown when the file tree is on):
  // the directory tree with per-file viewed checkbox / stats / comment badge,
  // its own filter field (bound to the viewer's search), a collapse-to-rail
  // button and a drag / keyboard-resizable width. Rows are windowed
  // (ListWindow) so a 10k-file PR mounts only what is on screen.
  import type { FileDiff } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { plural } from '../../lib/plural';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { ListWindow } from '../../lib/listWindow.svelte';
  import { paneResizer } from '../../lib/paneResizer';
  import { fileStat } from './diff-model';
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

  let navDirCollapsed: Record<string, boolean> = $state({});
  function toggleNavDir(path: string): void {
    navDirCollapsed = { ...navDirCollapsed, [path]: !navDirCollapsed[path] };
  }

  /** Dirs with at least one matching file beneath (bottom-up, once per term). */
  const navVisibleDirs = $derived.by(() => {
    const m = matchSet;
    if (!m) return null;
    const vis = new Set<string>();
    const walk = (d: NavDir): boolean => {
      let any = d.files.some((f) => m.has(f.path));
      for (const sub of d.dirs) if (walk(sub)) any = true;
      if (any) vis.add(d.path);
      return any;
    };
    walk(tree);
    return vis;
  });
  // The nav tree as the flat list of VISIBLE rows (folded dirs and search misses
  // skipped), windowed against `.nav-files` (`ListWindow`, with ⌘F over every
  // row): dir and file rows share one fixed pitch (CSS) so the math is exact.
  type NavRow =
    | { kind: 'dir'; d: NavDir; depth: number; key: string }
    | { kind: 'file'; f: FileDiff; depth: number; key: string };
  const navRows = $derived.by(() => {
    const out: NavRow[] = [];
    const vis = navVisibleDirs;
    const m = matchSet;
    const shut = navDirCollapsed;
    const walk = (d: NavDir, depth: number): void => {
      for (const sub of d.dirs) {
        if (vis !== null && !vis.has(sub.path)) continue;
        out.push({ kind: 'dir', d: sub, depth, key: `d:${sub.path}` });
        if (!shut[sub.path]) walk(sub, depth + 1);
      }
      for (const f of d.files) {
        if (m !== null && !m.has(f.path)) continue;
        out.push({ kind: 'file', f, depth, key: `f:${f.path}` });
      }
    };
    walk(tree, 0);
    return out;
  });
  const navWin = new ListWindow('.nav-file, .nav-dir-row', { min: 200, overscan: 15 });
  const navRange = $derived(navWin.range(navRows.length));
  $effect(() => navWin.findRows(() => null, () => navRows, (r) => (r.kind === 'dir' ? r.d.label : (r.f.path.split('/').pop() ?? ''))));
  // Nav: basename (the tree rows carry the directory context).
  function baseName(path: string): string {
    return path.split('/').pop() ?? path;
  }

</script>

  <!-- File Navigator Sidebar -->
{#snippet navFileRow(file: FileDiff, depth: number)}
    {@const stats = fileStat(file)}
    {@const cCount = commentCounts.get(file.path) ?? 0}
    {@const isViewed = viewed.has(file.path)}
    <!-- The row is a plain box holding TWO controls side by side — the "viewed"
         checkbox and a real button that jumps to the file — so nothing interactive
         is nested inside a role=button. Enter / Space both activate the button. -->
    <div
      class="nav-file"
      class:nav-file-viewed={isViewed}
      style="padding-inline-start: {8 + depth * 12}px"
      title={file.path}
    >
      <span class="nav-viewed-cb">
        <input
          type="checkbox"
          checked={isViewed}
          title="Mark as viewed"
          onchange={() => ontoggleviewed(file.path)}
          aria-label="Mark {file.path} as viewed"
        />
      </span>
      <button class="nav-file-open" onclick={() => onjump(file.path)} aria-label="Go to {file.path}">
        <span class="nav-file-path">
          <span class="nav-base">{baseName(file.path)}</span>
        </span>
        <span class="nav-file-stats">
          <span class="add">+{stats.add}</span>
          <span class="del">−{stats.del}</span>
        </span>
        {#if cCount > 0}
          <span class="nav-comment-badge" title="{plural(cCount, 'comment')}">
            💬{cCount}
          </span>
        {/if}
      </button>
    </div>
  {/snippet}

  {#snippet navDirRow(sub: NavDir, depth: number)}
    <button
      class="nav-dir-row"
      style="padding-inline-start: {8 + depth * 12}px"
      aria-expanded={!navDirCollapsed[sub.path]}
      onclick={() => toggleNavDir(sub.path)}
      title={sub.path}
    >
      <span class="nav-dir-chevron">
        <Icon name={navDirCollapsed[sub.path] ? 'chevronRight' : 'chevronDown'} size={12} />
      </span>
      <Icon name="folder" size={12} />
      <span class="nav-dir-label">{sub.label}</span>
    </button>
  {/snippet}

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
          <div class="nav-rows" {@attach navWin.attach}>
            {#if navRange.top > 0}<div style="height:{navRange.top}px" aria-hidden="true"></div>{/if}
            {#each navRows.slice(navRange.start, navRange.end) as r (r.key)}
              {#if r.kind === 'dir'}
                {@render navDirRow(r.d, r.depth)}
              {:else}
                {@render navFileRow(r.f, r.depth)}
              {/if}
            {/each}
            {#if navRange.bottom > 0}<div style="height:{navRange.bottom}px" aria-hidden="true"></div>{/if}
          </div>
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
  /* Directory group rows (tree). */
  /* One fixed pitch for dir and file rows: the nav is windowed (ListWindow
     measures it from a rendered row), so every row must be the same height. */
  .nav-dir-row,
  .nav-file {
    height: 26px;
    box-sizing: border-box;
    overflow: hidden;
  }
  .nav-dir-row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    inline-size: 100%;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
    white-space: nowrap;
  }
  .nav-dir-row:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nav-dir-chevron {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
  .nav-dir-label {
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 600;
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
  .nav-file {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    padding-inline-start: 6px;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text);
    border-radius: 0;
    transition: background var(--dur-fast);
    min-width: 0;
  }
  .nav-file:hover {
    background: var(--hover);
  }
  .nav-file.nav-file-viewed {
    opacity: 0.45;
  }
  /* The file-jump button fills the row beside the checkbox. */
  .nav-file-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    block-size: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .nav-viewed-cb {
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }
  .nav-viewed-cb input[type='checkbox'] {
    width: 12px;
    height: 12px;
    cursor: pointer;
    accent-color: var(--accent);
  }
  .nav-file-path {
    flex: 1;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    min-width: 0;
    line-height: 1.3;
  }
  .nav-base {
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .nav-file-stats {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
  }
  .nav-comment-badge {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    white-space: nowrap;
  }

  .nav-collapse-ico {
    display: inline-flex;
  }
  .add {
    color: var(--success);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .del {
    color: var(--danger);
    font-weight: 600;
    font-size: var(--fs-xs);
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
    .nav-file { font-size: var(--fs-m); padding: 8px; height: 38px; }
    .nav-base { font-size: var(--fs-m); }
    .nav-dir-row { font-size: var(--fs-m); padding: 6px 8px; height: 38px; }
    .nav-file-stats { font-size: var(--fs-s); }
    .nav-search { font-size: var(--fs-m); height: 32px; }
    /* No drag-resize on touch layouts — the sidebar is full-width there. */
    .nav-resize-handle { display: none; }
    .add, .del { font-size: var(--fs-m); }
    /* "Mark as viewed" checkbox: 12px is below the touch minimum. */
    .nav-viewed-cb input[type='checkbox'] { width: 18px; height: 18px; }
    /* Sidebar collapse button. */
    .nav-collapse-btn { min-width: 36px; min-height: 36px; justify-content: center; }
  }
</style>
