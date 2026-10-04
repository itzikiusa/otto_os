<script lang="ts">
  // Shared diff renderer (Changes / commit / PR views): unified or
  // side-by-side, per-file collapse, syntax highlight, and (PR mode) inline
  // comment threads + line-gutter composer, file-navigator sidebar, search.
  //
  // Huge diffs (100k+ changed lines) stay O(viewport):
  //  • the whole diff is flattened into ONE row model (file header, hunk
  //    header, line / split row, comment, composer, "show more", …) and a
  //    single variable-height window renders only the rows that intersect the
  //    visible part of whatever scrolls us (any ancestor or the page). Row
  //    heights are measured (ResizeObserver) so wrapped code lines keep the
  //    same wrapping UX as before — rows are never forced to a fixed height;
  //  • collapse state is DERIVED (unknown ⇒ its default), never filled in by
  //    an effect after a first render of everything;
  //  • counts come from the server's `added`/`deleted`; a `summary=true` diff
  //    lists files first and each file's hunks are fetched (`loadFile`) only
  //    once it is expanded and near the viewport, abortable;
  //  • syntax highlighting is deferred to rAF slices for mounted rows only.
  import { tick, untrack } from 'svelte';
  import type {
    DiffResp,
    FileDiff,
    DiffLine,
    Hunk,
    HunkOp,
    PrComment,
    StageHunkResp,
  } from '../../lib/api/types';
  import { langFromPath, ensureHljs } from '../../lib/hl';
  import { api, ApiError, isAbortError } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { gitBridge } from './gitBridge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CommentThread from './CommentThread.svelte';
  import type { DiffFileLoader } from './diff-load';
  import {
    COLLAPSE_ALL_FILES,
    COLLAPSE_ALL_LINES,
    FILE_COLLAPSE_LINES,
    buildFileRows,
    estimateRow,
    fileMatches,
    fileStat,
    inHunk,
    indexComments,
    lineAnchor,
    isGenerated,
    type ComposerAt,
    type Row,
  } from './diff-model';
  import { findScroller, resum, rowAt } from './diff-virtual';
  import { registerFindProvider } from '../../lib/findProviders';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { ListWindow } from './list-window.svelte';
  import { DeferredHighlighter } from './diff-highlight.svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { paneResizer } from '../../lib/paneResizer';

  interface Props {
    diff: DiffResp;
    prMode?: boolean;
    showNav?: boolean;
    comments?: PrComment[];
    /** Post an inline comment. `side` is the diff side `line` counts on
     *  (`old` for a deleted row); `oldLine` is the row's old number when it
     *  has one (GitLab needs both for a context line). */
    onAddComment?: (
      path: string,
      line: number,
      body: string,
      anchor: { side: 'old' | 'new'; oldLine: number | null },
    ) => Promise<void>;
    /** Reply to an existing thread (works on resolved threads too). */
    onReplyComment?: (parentId: string, body: string) => Promise<void>;
    /** Resolve/reopen a thread on the provider. */
    onResolveComment?: (threadId: string, resolved: boolean) => Promise<void>;
    /** Repo this diff belongs to — enables the per-file ⋯ History/Blame menu. */
    repoId?: string;
    /**
     * WIP mode: per-hunk Stage/Unstage/Discard + line selection. `target` is the
     * diff this viewer is rendering, so the hunk indices the buttons send match
     * the raw diff the server rebuilds the patch from. Requires `repoId`.
     */
    wip?: {
      target: 'worktree' | 'staged';
      onapplied: (r: StageHunkResp) => void;
    };
    /**
     * Lazy hunks (see `diff-load.ts`): files the server sent without hunks
     * (`hunks_omitted` — summary mode or a response cap) are fetched through
     * this once expanded and near the viewport; `too_large` files offer
     * "Load anyway" (`full`). Without it such files show a note instead.
     */
    loadFile?: DiffFileLoader;
  }
  let {
    diff,
    prMode = false,
    showNav = false,
    comments = [],
    onAddComment,
    onReplyComment,
    onResolveComment,
    repoId,
    wip,
    loadFile,
  }: Props = $props();

  let mode = $state<'unified' | 'split'>('unified');
  let hlReady = $state(false);
  const hl = new DeferredHighlighter();
  // Minified/generated lines: the DOM only ever gets the first LINE_CUT chars
  // of a line until the user expands it — layout + measurement of a single
  // 200 KB text node (or 5 MB after "Load anyway") was a multi-hundred-ms task.
  const LINE_CUT = 10_000;
  const expandedLines = new SvelteSet<string>();
  function kb(n: number): string {
    return n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`;
  }

  // ≤1024 (phone + tablet): side-by-side is unusable in the narrow width (two
  // code columns + 140-char lines either clip off-screen or wrap into an
  // unreadable mess), so we force the unified renderer and hide the toggle.
  // `mode` is left untouched so the user's choice is restored on a wider screen.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  const effMode = $derived<'unified' | 'split'>(isMobile ? 'unified' : mode);

  // ── Per-diff view state ─────────────────────────────────────────────────
  // Everything the user toggles belongs to ONE diff object: the state records
  // which diff it was made for, and a different `diff` prop reads as a fresh
  // state synchronously (no reset effect ⇒ no render with stale state first).
  // Split by what it feeds: `vs` changes the row model; `viewed` /
  // `sel` only restyle rows, so toggling them never rebuilds rows.
  interface ViewState {
    for: DiffResp | null;
    /** Explicit collapse choices; a missing path uses its derived default. */
    overrides: Record<string, boolean>;
    composer: ComposerAt | null;
    /** path → hunk indices uncapped by "Show N more lines". */
    uncapped: Map<string, Set<number>>;
    /** Lazily fetched files (path → full FileDiff). */
    loaded: Map<string, FileDiff>;
    errors: Map<string, string>;
    /** Too-large files the user asked to load anyway. */
    full: Set<string>;
  }
  const freshState = (d: DiffResp | null): ViewState => ({
    for: d,
    overrides: {},
    composer: null,
    uncapped: new Map(),
    loaded: new Map(),
    errors: new Map(),
    full: new Set(),
  });
  let vsRaw = $state.raw<ViewState>(freshState(null));
  const vs: ViewState = $derived(vsRaw.for === diff ? vsRaw : freshState(diff));
  /** Patch the view state of diff `d` — a no-op once `d` is no longer shown. */
  function patchVs(p: Partial<ViewState>, d: DiffResp = diff): void {
    if (d !== diff) return;
    vsRaw = { ...vs, ...p, for: d };
  }

  let viewedRaw = $state.raw<{ for: DiffResp | null; v: Set<string> }>({ for: null, v: new Set() });
  const viewed = $derived(viewedRaw.for === diff ? viewedRaw.v : new Set<string>());

  let composerText = $state('');
  let composerBusy = $state(false);

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

  // ── Nav tree: group files by directory (GitHub-style), compressing
  // single-child chains ("apps" → "sinatra" → "src" renders once as
  // "apps/sinatra/src"). Directories are collapsible; files keep the viewed
  // checkbox / stats / comment badge of the old flat rows.
  interface NavDir {
    /** Compressed display label, e.g. "apps/sinatra/src". */
    label: string;
    /** Full prefix path (unique key), e.g. "apps/sinatra/src". */
    path: string;
    dirs: NavDir[];
    files: FileDiff[];
  }
  const navTree = $derived.by(() => {
    interface Node {
      dirs: Map<string, Node>;
      files: FileDiff[];
    }
    const root: Node = { dirs: new Map(), files: [] };
    for (const f of diff.files) {
      const parts = f.path.split('/');
      let cur = root;
      for (const p of parts.slice(0, -1)) {
        let next = cur.dirs.get(p);
        if (!next) {
          next = { dirs: new Map(), files: [] };
          cur.dirs.set(p, next);
        }
        cur = next;
      }
      cur.files.push(f);
    }
    // Compress chains + emit sorted (dirs first, then files, both A→Z).
    const emit = (node: Node, label: string, path: string): NavDir => {
      let n = node;
      let lbl = label;
      let pth = path;
      while (n.files.length === 0 && n.dirs.size === 1) {
        const [k, child] = [...n.dirs.entries()][0];
        lbl = lbl === '' ? k : `${lbl}/${k}`;
        pth = pth === '' ? k : `${pth}/${k}`;
        n = child;
      }
      const dirs = [...n.dirs.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([k, child]) => emit(child, k, pth === '' ? k : `${pth}/${k}`));
      const files = [...n.files].sort((a, b) => a.path.localeCompare(b.path));
      return { label: lbl, path: pth, dirs, files };
    };
    return emit(root, '', '');
  });
  let navDirCollapsed: Record<string, boolean> = $state({});
  function toggleNavDir(path: string): void {
    navDirCollapsed = { ...navDirCollapsed, [path]: !navDirCollapsed[path] };
  }

  // ── Search: one debounced pass per term ──────────────────────────────────
  let rawSearch = $state('');
  let search = $state('');
  $effect(() => {
    const val = rawSearch;
    const t = setTimeout(() => (search = val.trim().toLowerCase()), 200);
    return () => clearTimeout(t);
  });
  /** Files matching the search (null = no search). ONE scan per term; the nav
   *  rows and dirs below only do Set lookups (the old code rescanned every
   *  subtree for every directory level on every keystroke). */
  const matchSet = $derived.by(() => {
    if (!search) return null;
    const s = new Set<string>();
    for (const f of diff.files) if (fileMatches(vs.loaded.get(f.path) ?? f, search)) s.add(f.path);
    return s;
  });
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
    walk(navTree);
    return vis;
  });
  // The nav tree as the flat list of VISIBLE rows (folded dirs and search
  // misses skipped), windowed against `.nav-files` (`ListWindow`): a 1k-file
  // PR used to mount 1k nav rows (+ a checkbox each) next to the windowed
  // body. Dir and file rows share one fixed pitch (CSS) so the math is exact.
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
    walk(navTree, 0);
    return out;
  });
  const navWin = new ListWindow('.nav-file, .nav-dir-row', { min: 200, overscan: 15 });
  const navRange = $derived(navWin.range(navRows.length));
  const filteredFiles = $derived(matchSet ? diff.files.filter((f) => matchSet.has(f.path)) : diff.files);
  const matchCount = $derived(filteredFiles.length);

  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });

  // ── Stats + collapse defaults (derived — never an after-render effect) ────
  const totals = $derived.by(() => {
    if (diff.total_added != null && diff.total_deleted != null) {
      return { add: diff.total_added, del: diff.total_deleted };
    }
    let add = 0;
    let del = 0;
    for (const f of diff.files) {
      const s = fileStat(f);
      add += s.add;
      del += s.del;
    }
    return { add, del };
  });
  const collapseAll = $derived(
    diff.files.length > COLLAPSE_ALL_FILES || totals.add + totals.del > COLLAPSE_ALL_LINES,
  );
  function defaultCollapsed(f: FileDiff): boolean {
    if (collapseAll || isGenerated(f.path)) return true;
    const s = fileStat(f);
    return s.add + s.del > FILE_COLLAPSE_LINES;
  }
  function isCollapsed(f: FileDiff): boolean {
    return vs.overrides[f.path] ?? defaultCollapsed(f);
  }
  function toggleCollapsed(f: FileDiff): void {
    patchVs({ overrides: { ...vs.overrides, [f.path]: !isCollapsed(f) } });
  }
  const allCollapsed = $derived(diff.files.every((f) => isCollapsed(f)));
  /** Files per frame for a chunked Expand / Collapse all. */
  const EXPAND_CHUNK = 150;
  let expandFrame = 0;
  $effect(() => () => {
    if (expandFrame) cancelAnimationFrame(expandFrame);
  });
  /**
   * Expand / collapse every file. On a big diff the change is applied in
   * chunks of EXPAND_CHUNK files per frame, starting from the file at the top
   * of the view and working outward, so the files on screen open in the first
   * frame and no single frame rebuilds the rows of all of them (a 1000-file PR
   * spent 300+ ms in one task). Scroll anchoring keeps the view put while the
   * files above it open.
   */
  function setAllCollapsed(v: boolean): void {
    if (expandFrame) cancelAnimationFrame(expandFrame);
    expandFrame = 0;
    const d = diff;
    const files = d.files;
    if (files.length <= EXPAND_CHUNK) {
      const next: Record<string, boolean> = {};
      for (const f of files) next[f.path] = v;
      patchVs({ overrides: next });
      return;
    }
    const topRow = rows[rowAt(layout.offsets, layout.n, view.top)];
    const at = Math.max(0, topRow ? files.indexOf(topRow.file) : 0);
    // On-screen first, then below, then above (nearest first).
    const order = [...files.slice(at), ...files.slice(0, at).reverse()];
    let pos = 0;
    const step = () => {
      expandFrame = 0;
      if (d !== diff) return;
      const next = { ...vs.overrides };
      const end = Math.min(order.length, pos + EXPAND_CHUNK);
      for (; pos < end; pos++) next[order[pos].path] = v;
      patchVs({ overrides: next }, d);
      if (pos < order.length) expandFrame = requestAnimationFrame(step);
    };
    step();
  }

  // ── PR comments: indexed by path once per comment list ────────────────────
  const commentIdx = $derived(prMode ? indexComments(comments) : new Map());
  // Per-file comment counts in one pass (nav + file headers).
  const commentCounts = $derived.by(() => {
    const m = new Map<string, number>();
    if (!prMode) return m;
    for (const c of comments) {
      if (c.path !== null) m.set(c.path, (m.get(c.path) ?? 0) + 1);
    }
    return m;
  });

  // ── Row model (memoized per file) ────────────────────────────────────────
  // A toggle / composer / comment / lazy load rebuilds ONE file's rows; the
  // rest are reused by reference. Split pairing is per hunk (WeakMap), built
  // only when a hunk is first flattened in split mode.
  let rowCache = new Map<string, { deps: unknown[]; rows: Row[] }>();
  let rowCacheFor: DiffResp | null = null;
  const sameDeps = (a: unknown[], b: unknown[]) => a.every((x, i) => x === b[i]);
  const rows: Row[] = $derived.by(() => {
    if (rowCacheFor !== diff) {
      rowCache = new Map();
      rowCacheFor = diff;
    }
    const st = vs;
    const split = effMode === 'split';
    const canLoad = !!loadFile;
    const out: Row[] = [];
    const next = new Map<string, { deps: unknown[]; rows: Row[] }>();
    const files = filteredFiles;
    for (let fi = 0; fi < files.length; fi++) {
      const file = files[fi];
      const eff = st.loaded.get(file.path) ?? file;
      const collapsed = st.overrides[file.path] ?? defaultCollapsed(file);
      const fc = prMode ? commentIdx.get(file.path) : undefined;
      const composer = st.composer && st.composer.path === file.path ? st.composer : null;
      const uncapped = st.uncapped.get(file.path);
      const loadError = st.errors.get(file.path);
      const forceFull = st.full.has(file.path);
      const deps = [eff, collapsed, fi === 0, split, fc, uncapped, composer, loadError, forceFull, canLoad];
      const hit = rowCache.get(file.path);
      const fr =
        hit && sameDeps(hit.deps, deps)
          ? hit.rows
          : buildFileRows({ file, eff, collapsed, first: fi === 0, split, fc, uncapped, composer, loadError, forceFull, canLoad });
      next.set(file.path, { deps, rows: fr });
      for (let j = 0; j < fr.length; j++) out.push(fr[j]);
    }
    rowCache = next;
    return out;
  });

  // ── Variable-height window ───────────────────────────────────────────────
  /** Measured row heights by row key (survive row-model rebuilds). */
  let measured = new Map<string, number>();
  let measuredFor: DiffResp | null = null;
  let measuredMobile = false;
  /** Bumped whenever a measurement patches `layout` in place. */
  let measureVersion = $state(0);
  const layout = $derived.by(() => {
    const rs = rows;
    const mob = isMobile;
    if (measuredFor !== diff || measuredMobile !== mob) {
      measured = new Map();
      measuredFor = diff;
      measuredMobile = mob;
    }
    const n = rs.length;
    const heights = new Float64Array(n);
    const offsets = new Float64Array(n + 1);
    for (let i = 0; i < n; i++) heights[i] = measured.get(rs[i].key) ?? estimateRow(rs[i], mob);
    resum(heights, offsets, 0);
    return { heights, offsets, n };
  });

  /** Visible band of the diff body, in body coordinates. */
  let view = $state({ top: 0, h: 1200 });
  /** Rendered beyond the visible band on each side (px). */
  const OVERSCAN_PX = 800;
  // A string so an unchanged window doesn't invalidate the slice below.
  const winRange = $derived.by(() => {
    void measureVersion;
    const { offsets, n } = layout;
    if (n === 0) return '0:0';
    const start = rowAt(offsets, n, view.top - OVERSCAN_PX);
    const end = Math.min(n, rowAt(offsets, n, view.top + view.h + OVERSCAN_PX) + 1);
    return `${start}:${end}`;
  });
  function parseRange(r: string): [number, number] {
    const c = r.indexOf(':');
    return [Number(r.slice(0, c)), Number(r.slice(c + 1))];
  }
  const pads = $derived.by(() => {
    void measureVersion;
    const [s, e] = parseRange(winRange);
    const { offsets, n } = layout;
    return { top: offsets[s] ?? 0, bottom: (offsets[n] ?? 0) - (offsets[e] ?? 0) };
  });

  /** Window rows grouped for the DOM: consecutive in-hunk rows share one
   *  `.dtable` block keyed by hunk (stable while scrolling inside it). */
  interface Seg {
    key: string;
    group: boolean;
    split: boolean;
    items: { r: Row; i: number }[];
  }
  const segs: Seg[] = $derived.by(() => {
    const [s, e] = parseRange(winRange);
    const rs = rows;
    const out: Seg[] = [];
    let g: Seg | null = null;
    for (let i = s; i < e && i < rs.length; i++) {
      const r = rs[i];
      if (inHunk(r)) {
        const gk = `${r.file.path}\u0000g${r.hi}`;
        if (!g || g.key !== gk) {
          g = { key: gk, group: true, split: r.kind === 'split' || effMode === 'split', items: [] };
          out.push(g);
        }
        g.items.push({ r, i });
      } else {
        g = null;
        out.push({ key: r.key, group: false, split: false, items: [{ r, i }] });
      }
    }
    return out;
  });

  // Comment threads, the composer and file-level comments are OVERLAY rows:
  // always mounted (there are O(comments) of them, not O(lines)), absolutely
  // positioned at their row offset over an in-flow spacer. Virtualization can
  // therefore never unmount a reply draft or the focused composer — the
  // instance is the same whether the row is on screen or not.
  const isOverlay = (r: Row) => r.kind === 'comment' || r.kind === 'composer' || r.kind === 'fcomments';
  const overlayIdx = $derived.by(() => {
    const out: number[] = [];
    if (!prMode) return out;
    const rs = rows;
    for (let i = 0; i < rs.length; i++) if (isOverlay(rs[i])) out.push(i);
    return out;
  });
  const overlays = $derived.by(() => {
    void measureVersion;
    const { offsets } = layout;
    return overlayIdx.map((i) => ({ r: rows[i], i, top: offsets[i] }));
  });
  /** In-flow placeholder height for an overlay row. */
  function spacerH(i: number): number {
    void measureVersion;
    return layout.heights[i] ?? 0;
  }

  // ── Viewport tracking: intersect the body with every clipping ancestor ────
  // (works whether the host pane, the page, or `.diff` itself on mobile
  // scrolls, and for hidden hosts: a 0-height band renders only overscan).
  let rootEl: HTMLDivElement | undefined = $state();
  let bodyEl: HTMLDivElement | undefined = $state();
  let clippers: HTMLElement[] = [];
  let viewFrame = 0;
  function readView(): void {
    viewFrame = 0;
    const b = bodyEl;
    if (!b) return;
    const br = b.getBoundingClientRect();
    let top = 0;
    let bottom = window.innerHeight;
    for (const c of clippers) {
      const r = c.getBoundingClientRect();
      if (r.top > top) top = r.top;
      if (r.bottom < bottom) bottom = r.bottom;
    }
    const next = { top: Math.round(top - br.top), h: Math.max(0, Math.round(bottom - top)) };
    if (next.top !== view.top || next.h !== view.h) view = next;
    captureAnchor(next.top);
  }

  // ── Scroll anchoring across row-model changes ────────────────────────────
  // The row at the top of the view, and how far into it the view starts —
  // recaptured on every view read. A row-model change ABOVE it (a file above
  // loading its hunks, a chunked Expand all, a comment arriving) re-derives
  // the layout, and without compensation the content under the user slides
  // away: a nav jump to a far file lost its target as the files above it
  // loaded, so the target never re-entered the window and never loaded.
  // (Measured-height changes are compensated separately, in `onMeasure`.)
  let scrollAnchor: { key: string; i: number; delta: number } | null = null;
  function captureAnchor(top: number): void {
    const { offsets, n } = layout;
    if (n === 0) {
      scrollAnchor = null;
      return;
    }
    const i = rowAt(offsets, n, top);
    const r = rows[i];
    scrollAnchor = r ? { key: r.key, i, delta: top - offsets[i] } : null;
  }
  $effect(() => {
    const l = layout;
    untrack(() => {
      const a = scrollAnchor;
      if (!a || !bodyEl) return;
      const rs = rows;
      let idx = rs[a.i]?.key === a.key ? a.i : -1;
      if (idx < 0) for (let j = 0; j < rs.length; j++) if (rs[j].key === a.key) { idx = j; break; }
      if (idx < 0) {
        // The anchor row itself went away (its file collapsed / filtered).
        captureAnchor(view.top);
        return;
      }
      const d = l.offsets[idx] + a.delta - view.top;
      scrollAnchor = { key: a.key, i: idx, delta: a.delta };
      if (Math.abs(d) < 1) return;
      const sc = activeScroller();
      if (sc) sc.scrollTop += d;
      else window.scrollBy(0, d);
      // Render the compensated window now, not a frame later.
      view = { top: view.top + d, h: view.h };
    });
  });
  function scheduleView(): void {
    if (!viewFrame) viewFrame = requestAnimationFrame(readView);
  }
  $effect(() => {
    void isMobile;
    const b = bodyEl;
    if (!b) return;
    const list: HTMLElement[] = [];
    for (let p = b.parentElement; p && p !== document.documentElement; p = p.parentElement) {
      const oy = getComputedStyle(p).overflowY;
      if (oy !== 'visible') list.push(p);
    }
    clippers = list;
    for (const c of list) c.addEventListener('scroll', scheduleView, { passive: true });
    window.addEventListener('scroll', scheduleView, { passive: true });
    window.addEventListener('resize', scheduleView);
    const ro = new ResizeObserver(scheduleView);
    ro.observe(b);
    for (const c of list) ro.observe(c);
    scheduleView();
    return () => {
      for (const c of list) c.removeEventListener('scroll', scheduleView);
      window.removeEventListener('scroll', scheduleView);
      window.removeEventListener('resize', scheduleView);
      ro.disconnect();
      if (viewFrame) cancelAnimationFrame(viewFrame);
      viewFrame = 0;
    };
  });

  /** The ancestor that actually scrolls right now (null ⇒ the page). */
  function activeScroller(): HTMLElement | null {
    const b = bodyEl;
    if (!b) return null;
    for (let p = findScroller(b); p; p = findScroller(p)) {
      if (p.scrollHeight > p.clientHeight + 1) return p;
    }
    return null;
  }

  // ── Row measurement ─────────────────────────────────────────────────────
  const rowMeta = new WeakMap<Element, { key: string; i: number }>();
  const rowRO =
    typeof ResizeObserver === 'undefined' ? null : new ResizeObserver((entries) => onMeasure(entries));
  $effect(() => () => rowRO?.disconnect());
  function onMeasure(entries: ResizeObserverEntry[]): void {
    const { heights, offsets, n } = layout;
    const rs = rows;
    const anchor = rowAt(offsets, n, view.top);
    let minI = Infinity;
    let aboveDelta = 0;
    for (const e of entries) {
      const m = rowMeta.get(e.target);
      if (!m) continue;
      const h = e.borderBoxSize?.[0]?.blockSize ?? (e.target as HTMLElement).offsetHeight;
      if (!h) continue; // hidden host (e.g. an inactive tab) — keep the old value
      measured.set(m.key, h);
      if (m.i >= n || rs[m.i]?.key !== m.key || heights[m.i] === h) continue;
      if (m.i < anchor) aboveDelta += h - heights[m.i];
      heights[m.i] = h;
      if (m.i < minI) minI = m.i;
    }
    if (minI === Infinity) return;
    resum(heights, offsets, minI);
    // Rows above the visible band changed height: shift the scroll by the same
    // amount so what the user is looking at doesn't jump.
    if (Math.abs(aboveDelta) >= 1) {
      const sc = activeScroller();
      if (sc) sc.scrollTop += aboveDelta;
      else window.scrollBy(0, aboveDelta);
    }
    // The re-render (spacer heights, newly windowed rows) waits for the next
    // frame: done here, inside the ResizeObserver delivery, it resized the
    // body and its overflow ancestors mid-loop — shallower observed boxes the
    // loop then had to skip, which is the "ResizeObserver loop completed with
    // undelivered notifications" error the huge-diff spec logged hundreds of
    // times a second (plus an extra layout pass per frame). The DOM already
    // shows the real heights and the scroll compensation above stays
    // synchronous, so the one-frame deferral moves nothing on screen.
    if (!measureFrame) {
      measureFrame = requestAnimationFrame(() => {
        measureFrame = 0;
        measureVersion++;
      });
    }
  }
  let measureFrame = 0;
  $effect(() => () => {
    if (measureFrame) cancelAnimationFrame(measureFrame);
    measureFrame = 0;
  });
  function measure(node: HTMLElement, p: [string, number]) {
    rowMeta.set(node, { key: p[0], i: p[1] });
    rowRO?.observe(node);
    return {
      update(q: [string, number]) {
        rowMeta.set(node, { key: q[0], i: q[1] });
      },
      destroy() {
        rowRO?.unobserve(node);
      },
    };
  }

  // ── Lazy per-file hunks ──────────────────────────────────────────────────
  const MAX_LOADS = 4;
  const inflight = new Map<string, AbortController>();
  // A new diff (or unmount) aborts every fetch/highlight made for the old one.
  $effect(() => {
    void diff;
    return () => {
      for (const c of inflight.values()) c.abort();
      inflight.clear();
      hl.clear();
      expandedLines.clear();
    };
  });
  // Fetch the pending files that are inside the rendered window.
  $effect(() => {
    const [s, e] = parseRange(winRange);
    const rs = rows;
    const lf = loadFile;
    if (!lf) return;
    untrack(() => {
      // Visible band (and below) first, the overscan above last: with only
      // MAX_LOADS in flight, the file the user is looking at — e.g. the target
      // of a nav jump — must not wait behind the ones above it.
      const { offsets, n } = layout;
      const vis = Math.min(Math.max(s, rowAt(offsets, n, view.top)), e);
      for (let i = vis; i < e && i < rs.length; i++) {
        const r = rs[i];
        if (r.kind === 'pending') startLoad(r.file, lf);
      }
      for (let i = s; i < vis && i < rs.length; i++) {
        const r = rs[i];
        if (r.kind === 'pending') startLoad(r.file, lf);
      }
    });
  });
  // Finished loads land here and are committed together once per frame: each
  // commit rebuilds the row model + layout, so four files arriving in one
  // frame cost one rebuild, not four.
  let landed: { d: DiffResp; loaded: Map<string, FileDiff>; errors: Map<string, string> } | null = null;
  let landFrame = 0;
  $effect(() => () => {
    if (landFrame) cancelAnimationFrame(landFrame);
    landFrame = 0;
    landed = null;
  });
  function land(d: DiffResp, path: string, fd: FileDiff | null, err: string | null): void {
    if (!landed || landed.d !== d) landed = { d, loaded: new Map(), errors: new Map() };
    if (err === null) landed.loaded.set(path, fd as FileDiff);
    else landed.errors.set(path, err);
    if (!landFrame) {
      landFrame = requestAnimationFrame(() => {
        landFrame = 0;
        const batch = landed;
        landed = null;
        if (!batch || batch.d !== diff) return;
        const patch: Partial<ViewState> = {};
        if (batch.loaded.size) patch.loaded = new Map([...vs.loaded, ...batch.loaded]);
        if (batch.errors.size) patch.errors = new Map([...vs.errors, ...batch.errors]);
        patchVs(patch, batch.d);
      });
    }
  }
  function startLoad(file: FileDiff, lf: DiffFileLoader): void {
    if (inflight.has(file.path) || inflight.size >= MAX_LOADS) return;
    // Landed but not committed yet (the next frame commits it).
    if (landed && landed.d === diff && (landed.loaded.has(file.path) || landed.errors.has(file.path))) return;
    const d = diff;
    const ctl = new AbortController();
    inflight.set(file.path, ctl);
    const done = () => {
      if (inflight.get(file.path) === ctl) inflight.delete(file.path);
    };
    lf(file, { full: vs.full.has(file.path), signal: ctl.signal }).then(
      (fd) => {
        done();
        if (ctl.signal.aborted || d !== diff) return;
        land(d, file.path, fd ?? { ...file, hunks: [], hunks_omitted: false, too_large: false }, null);
      },
      (e: unknown) => {
        done();
        if (isAbortError(e) || ctl.signal.aborted || d !== diff) return;
        land(d, file.path, null, e instanceof Error ? e.message : String(e));
      },
    );
  }
  function retryLoad(file: FileDiff): void {
    const errors = new Map(vs.errors);
    errors.delete(file.path);
    patchVs({ errors });
  }
  function loadAnyway(file: FileDiff): void {
    const full = new Set(vs.full);
    full.add(file.path);
    const loaded = new Map(vs.loaded);
    loaded.delete(file.path);
    patchVs({ full, loaded });
  }
  function showMore(file: FileDiff, hi: number): void {
    const uncapped = new Map(vs.uncapped);
    const s = new Set(uncapped.get(file.path));
    s.add(hi);
    uncapped.set(file.path, s);
    patchVs({ uncapped });
  }

  // ── PR composer ──────────────────────────────────────────────────────────
  // Keyed by the full (old_line, new_line) pair so a deleted row and an added
  // row that share a displayed line number (e.g. old 15 deleted + new 15
  // added) stay distinct. `line` is the number we post to.
  function gutterClick(path: string, line: DiffLine | null): void {
    if (!prMode || !onAddComment || !line) return;
    // A deleted row comments on the OLD side with its old number; every
    // other row on the new side (posting a deleted row's old number as a
    // new-side line landed on an unrelated line or 422'd).
    const a = lineAnchor(line);
    if (!a) return;
    const c = vs.composer;
    const same = c?.path === path && c.oldLine === line.old_line && c.newLine === line.new_line;
    patchVs({
      composer: same ? null : { path, oldLine: line.old_line, newLine: line.new_line, line: a.line, side: a.side },
    });
    composerText = '';
  }
  async function submitComment(): Promise<void> {
    const c = vs.composer;
    if (!c || !onAddComment || composerText.trim() === '') return;
    const d = diff;
    composerBusy = true;
    try {
      await onAddComment(c.path, c.line, composerText.trim(), { side: c.side, oldLine: c.oldLine });
      patchVs({ composer: null }, d);
      composerText = '';
    } finally {
      composerBusy = false;
    }
  }
  // When the comment composer gains focus the soft keyboard can cover it; pull it
  // into view so the textarea + actions stay visible while typing on a phone.
  function composerFocus(e: FocusEvent): void {
    const el = e.currentTarget as HTMLElement | null;
    if (!el) return;
    requestAnimationFrame(() => el.scrollIntoView({ block: 'center', behavior: 'smooth' }));
  }

  // ── Hunk / line staging (WIP mode) ─────────────────────────────────────────
  // One selection at a time, scoped to a single hunk: a patch is rebuilt from
  // ONE hunk server-side, so a cross-hunk selection has nowhere to go. Indices
  // are positions in `hunk.lines`, which is exactly how the server indexes the
  // raw diff body (markers excluded). Scoped to the diff it was made on.
  type Sel = { path: string; hunk: number; lines: Set<number>; anchor: number };
  let selRaw = $state.raw<{ for: DiffResp | null; s: Sel | null }>({ for: null, s: null });
  const sel = $derived(selRaw.for === diff ? selRaw.s : null);
  let applying = $state(false);

  function isSelected(path: string, hi: number, li: number): boolean {
    return sel !== null && sel.path === path && sel.hunk === hi && sel.lines.has(li);
  }

  /** Click = toggle one line; shift-click = the range from the last anchor. */
  function selectLine(e: MouseEvent, path: string, hi: number, li: number, line: DiffLine): void {
    if (!wip || line.origin === 'context') return;
    const cur = sel !== null && sel.path === path && sel.hunk === hi ? sel : null;
    if (e.shiftKey && cur) {
      const [a, b] = cur.anchor <= li ? [cur.anchor, li] : [li, cur.anchor];
      const lines = new Set(cur.lines);
      for (let i = a; i <= b; i++) lines.add(i);
      selRaw = { for: diff, s: { path, hunk: hi, lines, anchor: cur.anchor } };
      return;
    }
    const lines = cur ? new Set(cur.lines) : new Set<number>();
    if (lines.has(li)) lines.delete(li);
    else lines.add(li);
    selRaw = { for: diff, s: lines.size === 0 ? null : { path, hunk: hi, lines, anchor: li } };
  }

  /** "Stage hunk" → "Stage 4 lines" once lines inside THIS hunk are picked. */
  function selLabel(verb: string, path: string, hi: number): string {
    const n = sel !== null && sel.path === path && sel.hunk === hi ? sel.lines.size : 0;
    return n > 0 ? `${verb} ${n} line${n === 1 ? '' : 's'}` : `${verb} hunk`;
  }

  async function applyHunk(file: FileDiff, hi: number, hunk: Hunk, op: HunkOp): Promise<void> {
    if (!wip || !repoId || applying) return;
    if (op === 'discard') {
      const ok = await confirmer.ask(
        'Discard these changes? This rewrites the working file and cannot be undone from the file — a backup stash `otto: backup before hunk discard` is kept.',
        { title: 'Discard hunk', confirmLabel: 'Discard' },
      );
      if (!ok) return;
    }
    const picked =
      sel !== null && sel.path === file.path && sel.hunk === hi && sel.lines.size > 0
        ? [...sel.lines].sort((a, b) => a - b)
        : undefined;
    applying = true;
    try {
      const r = await api.post<StageHunkResp>(`/repos/${repoId}/stage-hunk`, {
        path: file.path,
        hunk_index: hi,
        hunk_header: hunk.header,
        fingerprint: file.fingerprint,
        lines: picked,
        op,
        confirm: op === 'discard' ? true : undefined,
      });
      selRaw = { for: null, s: null };
      wip.onapplied(r);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const status = e instanceof ApiError ? e.status : 0;
      if (status === 400 && msg.includes('stage the whole file')) {
        toasts.error(
          'Partial staging unavailable',
          "This hunk can't be staged partially (renamed/binary file) — stage the whole file.",
        );
      } else if (status === 409 && msg.includes('changed since the diff')) {
        toasts.error(
          'Hunk out of date',
          'The hunk no longer applies — the file changed since the diff was shown; refresh and retry.',
        );
      } else {
        toasts.error('Stage failed', msg);
      }
    } finally {
      applying = false;
    }
  }

  /** File header ⋯ — the diff is the only place a path is at hand, so this is
   *  where History / Blame hang off. The panels live in RepoView; `gitBridge`
   *  carries the request there without prop-drilling the whole graph. */
  function fileToolsMenu(e: MouseEvent, file: FileDiff): void {
    if (!repoId) return;
    ctxMenu.show(e, [
      {
        label: 'History',
        icon: 'note',
        action: () => gitBridge.openFileTool({ kind: 'history', repoId, path: file.path }),
      },
      {
        label: 'Blame',
        icon: 'note',
        action: () => gitBridge.openFileTool({ kind: 'blame', repoId, path: file.path }),
      },
    ]);
  }

  // Nav: basename (the tree rows carry the directory context).
  function baseName(path: string): string {
    return path.split('/').pop() ?? path;
  }

  // ── ⌘F over the whole diff (lib/findProviders.ts) ──────────────────────
  // The body mounts only the window (+800 px), so find-in-page searched one
  // screen of a 100k-line diff. It now searches the row model — every file,
  // hunk header and line of the expanded files — and reveals a match by an
  // index jump (as scrollToFile does), then highlights it in the mounted row
  // (gutters and stats are `data-find-skip`).
  function findTextOf(r: Row): string {
    switch (r.kind) {
      case 'file':
        return r.file.old_path ? `${r.file.old_path} → ${r.file.path}` : r.file.path;
      case 'hunk':
        return r.hunk.header;
      case 'line':
        return r.line.content;
      case 'split':
        return `${r.sr.left?.content ?? ''}\n${r.sr.right?.content ?? ''}`;
      case 'note':
        return r.text;
      case 'error':
        return r.message;
      case 'comment':
      case 'fcomments':
        return r.comments.map((c) => c.body).join('\n');
      default:
        return '';
    }
  }
  $effect(() =>
    registerFindProvider({
      root: () => bodyEl ?? null,
      count: () => rows.length,
      text: (i) => (rows[i] ? findTextOf(rows[i]) : ''),
      reveal: async (i) => {
        const b = bodyEl;
        if (!b || i >= layout.n) return;
        const y = layout.offsets[i] + layout.heights[i] / 2;
        const sc = activeScroller();
        const br = b.getBoundingClientRect();
        if (sc) sc.scrollTop += br.top - sc.getBoundingClientRect().top + y - sc.clientHeight / 2;
        else window.scrollBy(0, br.top + y - window.innerHeight / 2);
        readView();
        await tick();
      },
      rowElement: (i) => {
        const r = rows[i];
        return r ? (bodyEl?.querySelector(`[data-rk="${CSS.escape(r.key)}"]`) ?? null) : null;
      },
    }),
  );

  /** Jump to a file: expand it, then an index jump (instant — a smooth scroll
   *  across 100k rows would mount every row on the way), then snap to the
   *  real header once it is mounted and measured. */
  async function scrollToFile(path: string): Promise<void> {
    const f = diff.files.find((x) => x.path === path);
    if (f && isCollapsed(f)) patchVs({ overrides: { ...vs.overrides, [path]: false } });
    await tick();
    const idx = rows.findIndex((r) => r.kind === 'file' && r.file.path === path);
    const b = bodyEl;
    if (idx < 0 || !b) return;
    const y = layout.offsets[idx];
    const sc = activeScroller();
    const br = b.getBoundingClientRect();
    if (sc) sc.scrollTop += br.top - sc.getBoundingClientRect().top + y;
    else window.scrollBy(0, br.top + y);
    readView();
    await tick();
    requestAnimationFrame(() => {
      const el = rootEl?.querySelector(`[data-rk="${CSS.escape(`${path}\u0000f`)}"]`);
      el?.scrollIntoView({ block: 'start' });
    });
  }

  function toggleViewed(path: string): void {
    const next = new Set(viewed);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    viewedRaw = { for: diff, v: next };
  }

  const viewedCount = $derived(viewed.size);
  const totalFiles = $derived(diff.files.length);
  const lazyCount = $derived(diff.files.reduce((n, f) => n + (f.hunks_omitted ? 1 : 0), 0));

  // --- Keyboard file navigation: ] / n = next file, [ / N = prev file --------
  // Tracks which file in the filtered list is the "keyboard cursor" so ][ nav
  // works predictably when search is active.
  let navFocusIdx = $state(-1);

  function navToFile(delta: number): void {
    const files = filteredFiles;
    if (files.length === 0) return;
    const next = Math.max(0, Math.min(files.length - 1, navFocusIdx + delta));
    navFocusIdx = next;
    void scrollToFile(files[next].path);
  }

  function onDiffKeydown(e: KeyboardEvent): void {
    // Don't steal keys while the user is typing in a text field.
    const tag = (e.target as HTMLElement).tagName.toLowerCase();
    if (tag === 'input' || tag === 'textarea' || tag === 'select') return;
    if (e.key === ']' || e.key === 'n') {
      e.preventDefault();
      navToFile(+1);
    } else if (e.key === '[' || e.key === 'N') {
      e.preventDefault();
      navToFile(-1);
    }
  }

  const langCache = new Map<string, string | null>();
  function langOf(path: string): string | null {
    if (!hlReady) return null;
    let l = langCache.get(path);
    if (l === undefined) {
      l = langFromPath(path);
      langCache.set(path, l);
    }
    return l;
  }
  const sign = (l: DiffLine) => (l.origin === 'add' ? '+' : l.origin === 'del' ? '−' : '');
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="diff-root"
  class:with-nav={showNav && prMode}
  bind:this={rootEl}
  onkeydown={onDiffKeydown}
  role="region"
  aria-label="Diff viewer"
  tabindex="-1"
>
  <!-- File Navigator Sidebar -->
  {#snippet navFileRow(file: FileDiff, depth: number)}
    {@const stats = fileStat(file)}
    {@const cCount = commentCounts.get(file.path) ?? 0}
    {@const isViewed = viewed.has(file.path)}
    <div
      class="nav-file"
      class:nav-file-viewed={isViewed}
      style="padding-inline-start: {8 + depth * 12}px"
      role="button"
      tabindex="0"
      onclick={() => void scrollToFile(file.path)}
      onkeydown={(e) => e.key === 'Enter' && void scrollToFile(file.path)}
      title={file.path}
    >
      <span class="nav-viewed-cb">
        <input
          type="checkbox"
          checked={isViewed}
          title="Mark as viewed"
          onclick={(e) => e.stopPropagation()}
          onchange={() => toggleViewed(file.path)}
          aria-label="Mark {file.path} as viewed"
        />
      </span>
      <span class="nav-file-path">
        <span class="nav-base">{baseName(file.path)}</span>
      </span>
      <span class="nav-file-stats">
        <span class="add">+{stats.add}</span>
        <span class="del">−{stats.del}</span>
      </span>
      {#if cCount > 0}
        <span class="nav-comment-badge" title="{cCount} comment{cCount === 1 ? '' : 's'}">
          💬{cCount}
        </span>
      {/if}
    </div>
  {/snippet}

  {#snippet navDirRow(sub: NavDir, depth: number)}
    <div
      class="nav-dir-row"
      style="padding-inline-start: {8 + depth * 12}px"
      role="button"
      tabindex="0"
      onclick={() => toggleNavDir(sub.path)}
      onkeydown={(e) => e.key === 'Enter' && toggleNavDir(sub.path)}
      title={sub.path}
    >
      <span class="nav-dir-chevron">
        <Icon name={navDirCollapsed[sub.path] ? 'chevronRight' : 'chevronDown'} size={10} />
      </span>
      <Icon name="folder" size={11} />
      <span class="nav-dir-label">{sub.label}</span>
    </div>
  {/snippet}

  {#if showNav && prMode}
    <aside
      class="diff-nav"
      class:nav-collapsed={navCollapsed}
      class:nav-resizing={navResizing}
      style={navCollapsed ? '' : `--navw:${navW}px`}
    >
      <div class="nav-header">
        {#if !navCollapsed}
          <span class="nav-title">
            {totalFiles} file{totalFiles === 1 ? '' : 's'}
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
          <input
            class="nav-search input"
            type="text"
            placeholder="Filter files…"
            bind:value={rawSearch}
            aria-label="Filter files by path or content"
          />
          {#if search}
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
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div
          class="nav-resize-handle"
          role="separator"
          tabindex="0"
          aria-label="Resize file sidebar"
          aria-orientation="vertical"
          title="Drag or use ←/→ to resize · double-click or Enter to reset"
          onmousedown={startNavResize}
          ondblclick={() => setNavW(240)}
          use:paneResizer={{ value: navW, min: 180, max: 520, onChange: setNavW, onReset: () => setNavW(240), text: (v) => `${Math.round(v)} pixels wide` }}
        ></div>
      {/if}
    </aside>
  {/if}

  {#snippet composerBox()}
    <div class="composer">
      <textarea
        class="input"
        rows="2"
        bind:value={composerText}
        onfocus={composerFocus}
        placeholder="Comment on {vs.composer?.side === 'old' ? 'old ' : ''}line {vs.composer?.line}…"
      ></textarea>
      <div class="composer-actions">
        <button class="btn small" onclick={() => patchVs({ composer: null })}>Cancel</button>
        <button
          class="btn small primary"
          disabled={composerBusy || composerText.trim() === ''}
          onclick={submitComment}
        >
          {composerBusy ? 'Posting…' : 'Comment'}
        </button>
      </div>
    </div>
  {/snippet}

  <!-- One row of the flattened diff. Every row root carries `use:measure`. -->
  <!-- No whitespace around the {#if}: it would render inside the code cell. -->
  {#snippet codeText(content: string, lang: string | null, key: string)}{#if content.length > LINE_CUT && !expandedLines.has(key)}{@html hl.html(content.slice(0, LINE_CUT), lang)}<button
        type="button"
        class="line-cut-btn"
        data-find-skip
        aria-label={`Expand line (${kb(content.length)})`}
        title={`Show the full line (${kb(content.length)})`}
        onclick={() => expandedLines.add(key)}>… expand line ({kb(content.length)})</button
      >{:else}{@html hl.html(content, lang)}{/if}{/snippet}

  {#snippet rowView(r: Row, i: number)}
    {#if r.kind === 'file'}
      {@const stats = fileStat(r.file)}
      {@const cCount = commentCounts.get(r.file.path) ?? 0}
      <div
        class="drow drow-file"
        class:first={r.first}
        class:open={!r.collapsed}
        id="dfile-{r.file.path}"
        data-rk={r.key}
        use:measure={[r.key, i]}
      >
        <!-- Row, not a single button: the ⋯ menu can't nest inside the collapse
             button (nested <button> is invalid HTML and swallows the click). -->
        <div class="dfile-headrow">
          <button class="dfile-head" aria-expanded={!r.collapsed} onclick={() => toggleCollapsed(r.file)}>
            <span class="dfile-chevron">
              <Icon name={r.collapsed ? 'chevronRight' : 'chevronDown'} size={11} />
            </span>
            <span class="dfile-path mono" dir="ltr">
              {#if r.file.old_path}{r.file.old_path}<span class="rename-arrow"> → </span>{/if}{r.file.path}
            </span>
            <span class="grow"></span>
            {#if prMode && cCount > 0}
              <span class="file-comment-badge" data-find-skip title="{cCount} comment{cCount === 1 ? '' : 's'}">
                💬 {cCount}
              </span>
            {/if}
            <span class="add" data-find-skip>+{stats.add}</span>
            <span class="del" data-find-skip>−{stats.del}</span>
          </button>
          {#if repoId}
            <button
              class="dfile-tools"
              title="File history / blame"
              aria-label="File tools for {r.file.path}"
              onclick={(e) => {
                e.stopPropagation();
                fileToolsMenu(e, r.file);
              }}
            ><Icon name="more" size={13} /></button>
          {/if}
        </div>
      </div>
    {:else if r.kind === 'hunk'}
      <div class="drow inf hunk-header mono" dir="ltr" data-rk={r.key} use:measure={[r.key, i]}>
        <span>{r.hunk.header}</span>
        {#if wip && repoId}
          {@const w = wip}
          <span class="grow"></span>
          <button
            class="hunk-btn"
            disabled={applying}
            onclick={() => void applyHunk(r.eff, r.hi, r.hunk, w.target === 'staged' ? 'unstage' : 'stage')}
          >{selLabel(w.target === 'staged' ? 'Unstage' : 'Stage', r.file.path, r.hi)}</button>
          {#if w.target === 'worktree'}
            <button
              class="hunk-btn danger"
              disabled={applying}
              onclick={() => void applyHunk(r.eff, r.hi, r.hunk, 'discard')}
            >{selLabel('Discard', r.file.path, r.hi)}</button>
          {/if}
        {/if}
      </div>
    {:else if r.kind === 'line'}
      {@const lang = langOf(r.file.path)}
      {@const pick = !!wip && r.line.origin !== 'context'}
      <div
        class="vrow dline {r.line.origin}"
        class:selected={isSelected(r.file.path, r.hi, r.li)}
        data-rk={r.key}
        use:measure={[r.key, i]}
      >
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span
          data-find-skip
          class="gut old"
          class:commentable={prMode}
          class:selectable={pick}
          onclick={(e) => (wip ? selectLine(e, r.file.path, r.hi, r.li, r.line) : gutterClick(r.file.path, r.line))}
        >{r.line.old_line ?? ''}</span>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span
          data-find-skip
          class="gut new"
          class:commentable={prMode}
          class:selectable={pick}
          onclick={(e) => (wip ? selectLine(e, r.file.path, r.hi, r.li, r.line) : gutterClick(r.file.path, r.line))}
        >{r.line.new_line ?? ''}</span>
        <span class="sign" data-find-skip>{sign(r.line)}</span>
        <span class="code mono">{@render codeText(r.line.content, lang, r.key)}</span>
      </div>
    {:else if r.kind === 'split'}
      {@const lang = langOf(r.file.path)}
      {@const L = r.sr.left}
      {@const R = r.sr.right}
      <div class="vrow split-vrow" data-rk={r.key} use:measure={[r.key, i]}>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span class="gut old" data-find-skip class:commentable={prMode} onclick={() => gutterClick(r.file.path, L)}
          >{L?.old_line ?? ''}</span>
        <span class="code mono half {L ? (L.origin === 'del' ? 'del' : '') : 'void'}"
          >{#if L}{@render codeText(L.content, lang, r.key + ':L')}{/if}</span>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span class="gut new" data-find-skip class:commentable={prMode} onclick={() => gutterClick(r.file.path, R)}
          >{R?.new_line ?? ''}</span>
        <span class="code mono half {R ? (R.origin === 'add' ? 'add' : '') : 'void'}"
          >{#if R}{@render codeText(R.content, lang, r.key + ':R')}{/if}</span>
      </div>
    {:else if r.kind === 'comment'}
      <div class="comment-row" data-rk={r.key} use:measure={[r.key, i]}>
        {#each r.comments as c (c.id)}
          <CommentThread comment={c} onreply={onReplyComment} onresolve={onResolveComment} />
        {/each}
      </div>
    {:else if r.kind === 'composer'}
      <div class="comment-row" data-rk={r.key} use:measure={[r.key, i]}>
        {@render composerBox()}
      </div>
    {:else if r.kind === 'fcomments'}
      <!-- File-level comments (no line anchor, or line not in diff) -->
      <div class="file-comments-block" data-rk={r.key} use:measure={[r.key, i]}>
        <div class="file-comments-label dim">File comments</div>
        {#each r.comments as c (c.id)}
          {#if c.outdated}
            <span class="chip outdated-chip" title="The code this comment was on has changed since">
              Outdated{c.line !== null ? ` · ${c.side === 'old' ? 'old ' : ''}line ${c.line}` : ''}
            </span>
          {/if}
          <CommentThread comment={c} onreply={onReplyComment} onresolve={onResolveComment} />
        {/each}
      </div>
    {:else if r.kind === 'more'}
      <div class="hunk-cap-cell" data-rk={r.key} use:measure={[r.key, i]}>
        <button class="btn small ghost hunk-cap-btn" onclick={() => showMore(r.file, r.hi)}>
          Show {r.remaining.toLocaleString()} more line{r.remaining === 1 ? '' : 's'}
        </button>
      </div>
    {:else if r.kind === 'note'}
      <div class="drow inf dfile-binary dim" data-rk={r.key} use:measure={[r.key, i]}>{r.text}</div>
    {:else if r.kind === 'large'}
      <div class="drow inf dfile-large" data-rk={r.key} use:measure={[r.key, i]}>
        <Icon name="file" size={13} />
        <span>Large file — {r.lines.toLocaleString()} changed lines</span>
        {#if r.canLoad}
          <span aria-hidden="true">·</span>
          <button class="btn small" onclick={() => loadAnyway(r.file)}>Load anyway</button>
        {/if}
      </div>
    {:else if r.kind === 'pending'}
      <div class="drow inf dfile-pending dim" role="status" data-rk={r.key} use:measure={[r.key, i]}>
        Loading diff…
      </div>
    {:else if r.kind === 'error'}
      <div class="drow inf dfile-error" role="alert" data-rk={r.key} use:measure={[r.key, i]}>
        <span>Couldn't load this file's diff: {r.message}</span>
        <button class="btn small" onclick={() => retryLoad(r.file)}>Retry</button>
      </div>
    {:else if r.kind === 'end'}
      <div class="drow drow-end" data-rk={r.key} use:measure={[r.key, i]}></div>
    {/if}
  {/snippet}

  <!-- Diff main area -->
  <div class="diff">
    <div class="diff-toolbar">
      <span class="diff-stats">
        {#if search}
          <span>{matchCount} / {diff.files.length} file{diff.files.length === 1 ? '' : 's'}</span>
        {:else}
          {diff.files.length} file{diff.files.length === 1 ? '' : 's'}
        {/if}
        <span class="add">+{totals.add}</span>
        <span class="del">−{totals.del}</span>
        {#if diff.truncated || (lazyCount > 0 && !loadFile)}
          <span class="diff-note" title="The diff is too large to send at once; open a file to load its lines.">
            Large diff — some files load on demand
          </span>
        {/if}
      </span>
      <span class="grow"></span>
      {#if !showNav || !prMode}
        <!-- Inline search bar when no nav sidebar -->
        <div class="toolbar-search-wrap">
          <input
            class="toolbar-search input"
            type="text"
            placeholder="Search files & diff…"
            bind:value={rawSearch}
            aria-label="Search files and diff content"
          />
          {#if search}
            <span class="toolbar-match-count">{matchCount} match{matchCount === 1 ? '' : 'es'}</span>
          {/if}
        </div>
      {/if}
      {#if diff.files.length > 1}
        <button
          class="btn small ghost"
          onclick={() => setAllCollapsed(!allCollapsed)}
          title={allCollapsed ? 'Expand every file (only the rows on screen render)' : 'Collapse every file'}
        >{allCollapsed ? 'Expand all' : 'Collapse all'}</button>
      {/if}
      {#if !isMobile}
        <!-- Side-by-side is desktop-only; ≤1024 always renders unified. -->
        <div class="segmented">
          <button class:active={mode === 'unified'} onclick={() => (mode = 'unified')}>Unified</button>
          <button class:active={mode === 'split'} onclick={() => (mode = 'split')}>Side by side</button>
        </div>
      {/if}
    </div>

    {#if filteredFiles.length === 0}
      <div class="dim diff-empty">
        {search ? 'No files match your search.' : 'No changes.'}
      </div>
    {:else}
      <!-- Windowed body: padding stands in for the rows above/below the window;
           overlay rows (threads, composer) are absolutely placed at their
           offsets and always mounted. -->
      <div class="diff-body" bind:this={bodyEl} style="padding-block: {pads.top}px {pads.bottom}px">
        {#each segs as seg (seg.key)}
          {#if seg.group}
            <div class="dtable" class:split={seg.split} dir="ltr">
              {#each seg.items as it (it.r.key)}
                {#if isOverlay(it.r)}
                  <div class="drow-spacer" style="height: {spacerH(it.i)}px"></div>
                {:else}
                  {@render rowView(it.r, it.i)}
                {/if}
              {/each}
            </div>
          {:else if isOverlay(seg.items[0].r)}
            <div class="drow-spacer" style="height: {spacerH(seg.items[0].i)}px"></div>
          {:else}
            {@render rowView(seg.items[0].r, seg.items[0].i)}
          {/if}
        {/each}
        {#each overlays as o (o.r.key)}
          <div class="drow-overlay" style="top: {o.top}px">
            {@render rowView(o.r, o.i)}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  /* Two-pane layout when nav is shown */
  .diff-root {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .diff-root.with-nav {
    flex-direction: row;
    align-items: flex-start;
    gap: 0;
  }

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
    max-height: 100vh;
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
    background: var(--surface-2);
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
    padding: 3px 8px;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
    white-space: nowrap;
  }
  .nav-dir-row:hover {
    background: var(--surface-2);
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
    background: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .nav-file {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 8px;
    padding-inline-start: 6px;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text);
    border-radius: 0;
    transition: background 80ms;
    min-width: 0;
  }
  .nav-file:hover {
    background: var(--surface-2);
  }
  .nav-file.nav-file-viewed {
    opacity: 0.45;
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

  /* ── Toolbar search (no-nav mode) ── */
  .toolbar-search-wrap {
    position: relative;
    display: flex;
    align-items: center;
  }
  .toolbar-search {
    font-size: var(--fs-xs);
    height: 26px;
    padding: 0 8px;
    width: 180px;
  }
  .toolbar-match-count {
    position: absolute;
    inset-inline-end: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    pointer-events: none;
    white-space: nowrap;
  }

  /* ── Diff main ── */
  .diff {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex: 1;
    min-width: 0;
  }
  .diff-root.with-nav .diff {
    padding-inline-start: 12px;
  }
  .diff-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .diff-stats {
    font-size: var(--fs-s);
    color: var(--text-dim);
    display: inline-flex;
    gap: 8px;
    align-items: center;
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
  .diff-empty {
    padding: 24px;
    text-align: center;
  }
  .diff-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* ── Windowed body. Each file still reads as one bordered card, drawn by
     its rows: the header row owns the top edge, in-file rows the side edges
     and the end row the bottom edge. */
  .diff-body {
    position: relative;
    min-width: 0;
  }
  .drow-file {
    padding-top: 10px;
  }
  .drow-file.first {
    padding-top: 0;
  }
  .inf,
  .dtable {
    border-inline: 1px solid var(--border);
    background: var(--surface);
  }
  .drow-end {
    height: 4px;
    border: 1px solid var(--border);
    border-top: none;
    border-end-start-radius: var(--radius-m);
    border-end-end-radius: var(--radius-m);
    background: var(--surface);
  }
  .drow-overlay {
    position: absolute;
    inset-inline: 0;
    top: 0;
  }
  .dfile-large,
  .dfile-error {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 14px;
    font-size: var(--fs-s);
  }
  .dfile-large {
    color: var(--text-dim);
  }
  .dfile-error {
    color: var(--danger);
  }
  .dfile-pending {
    padding: 12px 14px;
    font-size: var(--fs-s);
  }
  .dfile-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 12px;
    border: none;
    background: var(--surface-2);
    cursor: pointer;
    font-size: var(--fs-s);
    color: var(--text);
    text-align: start;
  }
  .dfile-path {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Direction-aware glyphs: the sidebar-collapse and file-collapse chevrons
     mirror under RTL via Icon's DIRECTIONAL set; the literal "→" of a
     "from → to" rename is text, so it is mirrored here. */
  .nav-collapse-ico,
  .dfile-chevron,
  .rename-arrow {
    display: inline-flex;
  }
  :global([dir='rtl']) .rename-arrow {
    transform: scaleX(-1);
  }
  .dfile-binary {
    padding: 14px;
    font-size: var(--fs-s);
  }
  .file-comment-badge {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .outdated-chip {
    display: inline-block;
    margin-block-end: 4px;
    font-size: var(--fs-xs);
    color: var(--warning);
    background: var(--warning-soft);
  }
  /* File-level unanchored comments */
  .file-comments-block {
    border-inline: 1px solid var(--border);
    padding: 8px 14px 10px;
    background: color-mix(in srgb, var(--accent) 5%, var(--bg));
    border-bottom: 1px solid var(--border);
  }
  .file-comments-label {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 4px;
  }

  .dfile-headrow {
    display: flex;
    align-items: stretch;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .drow-file.open .dfile-headrow {
    border-end-start-radius: 0;
    border-end-end-radius: 0;
  }
  .dfile-headrow .dfile-head {
    min-width: 0;
  }
  .dfile-tools {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    padding: 0 10px;
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: var(--fs-m);
    line-height: 1;
  }
  .dfile-tools:hover {
    color: var(--accent-text);
  }
  .hunk-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 12px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .dtable {
    font-size: var(--fs-xs);
    line-height: 1.55;
  }
  .gut {
    width: 42px;
    min-width: 42px;
    text-align: end;
    padding-block: 0; padding-inline: 4px 8px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    user-select: none;
    vertical-align: top;
    border-inline-end: 1px solid var(--border);
  }
  .gut.commentable,
  .gut.selectable {
    cursor: pointer;
  }
  .gut.commentable:hover,
  .gut.selectable:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--accent-text);
  }
  /* Small ghost actions in the sticky hunk header — accent, never louder than
     the code they sit above. */
  .hunk-btn {
    flex-shrink: 0;
    padding: 1px 7px;
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: var(--radius-s);
    background: none;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .hunk-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .hunk-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .hunk-btn.danger {
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--danger);
  }
  .hunk-btn.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
  }
  /* Line selection wins over the add/del row tints below it. */
  .vrow.dline.selected,
  .vrow.dline.selected .gut {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .sign {
    width: 16px;
    text-align: center;
    color: var(--text-dim);
    user-select: none;
    font-family: var(--font-mono);
  }
  .code {
    padding-block: 0; padding-inline: 4px 10px;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .code.half.add {
    background: color-mix(in srgb, var(--success) 11%, transparent);
  }
  .code.half.del {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  .code.half.void {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
  .comment-row {
    padding: 6px 12px;
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .composer {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 560px;
  }
  .composer-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  /* Responsive: collapse nav to a thin rail on small screens */
  @media (max-width: 640px) {
    .diff-nav {
      width: 32px;
      min-width: 32px;
      max-width: 32px;
    }
  }

  /* ── Mobile + tablet (≤1024px): readable diffs that fit the viewport width.
     Wrap long code lines (no horizontal page overflow), bump the tiny code +
     gutter + toolbar text up to a legible size, and let the file-navigator (PR
     mode) sit above the diff as a collapsible strip instead of a side rail.
     The breakpoint is 1024 so the tablet range (iPad portrait 834, real-phone
     landscape 932) gets the fits-the-width treatment — at those widths the
     240px nav rail + diff would otherwise clip the code off-screen right. */
  @media (max-width: 1024px) {
    .diff-body {
      max-width: 100%;
    }
    /* The diff is its own vertical scroll container on mobile (E2E invariant):
       hosts (PR/review/history) embed DiffViewer without always wrapping it in a
       scroll pane, so own it here. min-width:0 lets the flex child shrink so the
       wrapping .code below actually fits the viewport instead of pushing wider. */
    .diff {
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
      overscroll-behavior: contain;
      min-width: 0;
      max-width: 100%;
    }
    .diff-root {
      min-width: 0;
      max-width: 100%;
    }
    /* Stack nav over diff in PR mode so both fit the narrow viewport. */
    .diff-root.with-nav {
      flex-direction: column;
    }
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
    .diff-root.with-nav .diff {
      padding-inline-start: 0;
    }
    .nav-title { font-size: var(--fs-m); }
    .nav-file { font-size: var(--fs-m); padding: 8px; height: 38px; }
    .nav-base { font-size: var(--fs-m); }
    .nav-dir-row { font-size: var(--fs-m); padding: 6px 8px; height: 38px; }
    .nav-file-stats { font-size: var(--fs-s); }
    .nav-search { font-size: var(--fs-m); height: 32px; }
    /* No drag-resize on touch layouts — the sidebar is full-width there. */
    .nav-resize-handle { display: none; }

    .diff-toolbar { flex-wrap: wrap; gap: 8px; }
    .toolbar-search-wrap { flex: 1; min-width: 120px; }
    .diff-stats { font-size: var(--fs-m); }
    .add, .del { font-size: var(--fs-m); }
    .toolbar-search { font-size: var(--fs-m); height: 32px; width: 100%; }
    .dfile-head { font-size: var(--fs-m); padding: 9px 12px; min-height: 40px; }
    .dfile-path { font-size: var(--fs-m); }
    .hunk-header { font-size: var(--fs-s); padding: 4px 12px; }
    /* The fixed gutter+sign grid columns keep the code column from being
       stretched past the viewport by a single long token — it wraps within its
       remaining width (E2E seeds 140-char lines that must wrap, not clip). */
    .dtable { font-size: var(--fs-s); }
    .gut {
      font-size: var(--fs-xs);
      width: 30px;
      min-width: 30px;
      padding-block: 0; padding-inline: 3px 5px;
    }
    .sign { width: 13px; }
    /* Wrap code so long lines don't push the page wider than the screen.
       overflow-wrap:anywhere is the belt-and-suspenders for unbreakable tokens. */
    .code {
      font-size: var(--fs-s);
      white-space: pre-wrap;
      word-break: break-word;
      overflow-wrap: anywhere;
    }
    .composer { max-width: 100%; }
    /* Narrower mobile gutters; the code cell wraps. (Split-vrow is never
       reached on mobile — effMode forces unified.) */
    .dtable .vrow { grid-template-columns: 30px 30px 13px 1fr; }
    .vrow .code { white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere; }

    /* ── Touch targets (no hover on touch → affordances must be persistent). ── */
    /* "Mark as viewed" checkbox: 12px is below the touch minimum. */
    .nav-viewed-cb input[type='checkbox'] { width: 18px; height: 18px; }
    /* Sidebar collapse button. */
    .nav-collapse-btn { min-width: 36px; min-height: 36px; justify-content: center; }
    /* "Show N more lines" hunk-cap button. */
    .line-cut-btn {
    display: inline;
    margin-inline-start: 6px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .line-cut-btn:hover { background: var(--accent-soft); }
  .line-cut-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .hunk-cap-btn { font-size: var(--fs-s); min-height: 32px; padding: 6px 12px; }
    /* Comment composer Cancel/Comment buttons. */
    .composer-actions .btn { min-height: 36px; padding: 6px 14px; }
    /* PR inline-comment affordance: widen the gutter tap zone and surface a
       persistent "+" hint (the desktop hover hint is invisible on touch). */
    .gut.commentable {
      position: relative;
      min-width: 34px;
      width: 34px;
    }
    .gut.commentable::after {
      content: '+';
      position: absolute;
      inset-inline-start: 2px;
      top: 1px;
      font-size: var(--fs-xs);
      line-height: 1;
      color: color-mix(in srgb, var(--accent) 55%, transparent);
      pointer-events: none;
    }
  }

  /* Hunk line cap: "Show N more lines" affordance */
  .hunk-cap-cell {
    text-align: center;
    padding: 5px 8px;
    border-top: 1px dashed var(--border);
  }
  .hunk-cap-btn {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hunk-cap-btn:hover {
    color: var(--text);
  }

  /* ── Line rows: a fixed-column grid (gutter+gutter+sign+code). Code wraps
     (pre-wrap, like the old table), so rows vary in height — the window
     measures every mounted row instead of assuming a fixed height. */
  .vrow {
    display: grid;
    /* gut-old | gut-new | sign | code — mirrors the four table columns */
    grid-template-columns: 42px 42px 16px 1fr;
    align-items: start;
  }
  .vrow .gut {
    display: block;
    width: auto;
    min-width: 0;
    align-self: stretch;
  }
  .vrow.dline.add {
    background: color-mix(in srgb, var(--success) 11%, transparent);
  }
  .vrow.dline.del {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  /* Split side-by-side rows: gutter | code | gutter | code */
  .split-vrow {
    grid-template-columns: 42px 1fr 42px 1fr;
  }
  .split-vrow .code.half.add {
    background: color-mix(in srgb, var(--success) 11%, transparent);
  }
  .split-vrow .code.half.del {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  .split-vrow .code.half.void {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
</style>
