///<reference types="svelte" />
;
import { scrollBehavior } from '../../lib/motion';
import { loadErrorText } from '../../lib/loadError';
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
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
import type { DiffResp, FileDiff, DiffLine, Hunk, HunkOp, PrComment, StageHunkResp } from '../../lib/api/types';
import { refocusRow } from './diffRowFocus';
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
    ROW_KEY_SEP,
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
import { carryViewState, carryViewed, unchangedPaths } from './diff-viewstate';
import { registerFindProvider } from '../../lib/findProviders';
import { startMouseDrag } from '../../lib/dragCursor';
import { ListWindow } from '../../lib/listWindow.svelte';
import { DeferredHighlighter } from './diff-highlight.svelte';
import { SvelteSet } from 'svelte/reactivity';
import { paneResizer } from '../../lib/paneResizer';
;

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
    /** Stable identity of what this diff shows (e.g. `repo#pr`). When set, a
     *  re-fetched diff with the same key keeps viewed marks, expansions and
     *  the open composer for paths that still exist (a push re-fetches). */
    stateKey?: string;
  };;

  // ── Per-diff view state ─────────────────────────────────────────────────
  // Everything the user toggles belongs to ONE diff object: the state records
  // which diff it was made for, and a different `diff` prop reads as a fresh
  // state synchronously (no reset effect ⇒ no render with stale state first).
  // Split by what it feeds: `vs` changes the row model; `viewed` /
  // `sel` only restyle rows, so toggling them never rebuilds rows.
  interface ViewState {
    for: DiffResp | null;
    /** The `stateKey` this state was made under (carry-over across reloads). */
    key: string | undefined;
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
  };;

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
  };;
  // The nav tree as the flat list of VISIBLE rows (folded dirs and search misses
  // skipped), windowed against `.nav-files` (`ListWindow`, with ⌘F over every
  // row): dir and file rows share one fixed pitch (CSS) so the math is exact.
  type NavRow =
    | { kind: 'dir'; d: NavDir; depth: number; key: string }
    | { kind: 'file'; f: FileDiff; depth: number; key: string };;;

  /** Window rows grouped for the DOM: consecutive in-hunk rows share one
   *  `.dtable` block keyed by hunk (stable while scrolling inside it). */
  interface Seg {
    key: string;
    group: boolean;
    split: boolean;
    items: { r: Row; i: number }[];
  };;

  // ── Hunk / line staging (WIP mode) ─────────────────────────────────────────
  // One selection at a time, scoped to a single hunk: a patch is rebuilt from
  // ONE hunk server-side, so a cross-hunk selection has nowhere to go. Indices
  // are positions in `hunk.lines`, which is exactly how the server indexes the
  // raw diff body (markers excluded). Scoped to the diff it was made on.
  type Sel = { path: string; hunk: number; lines: Set<number>; anchor: number };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
    stateKey,
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

  const freshState = (d: DiffResp | null): ViewState => ({
    for: d,
    key: stateKey,
    overrides: {},
    composer: null,
    uncapped: new Map(),
    loaded: new Map(),
    errors: new Map(),
    full: new Set(),
  });
  let vsRaw = $state.raw<ViewState>(freshState(null));
  const diffPaths = $derived(new Set(diff.files.map((f) => f.path)));
  /** Same identity, new diff object (a reload): carry, don't reset. */
  const sameKey = (k: string | undefined): boolean => stateKey !== undefined && k === stateKey;
  // Carried only for files whose content didn't change (S15-304).
  const vs: ViewState = $derived(
    vsRaw.for === diff
      ? vsRaw
      : sameKey(vsRaw.key)
        ? { ...freshState(diff), ...carryViewState(vsRaw, unchangedPaths(vsRaw.for, diff)) }
        : freshState(diff),
  );
  // The open composer's file changed under it: its anchor was dropped above.
  // Keep the typed text for the next composer and say why it closed.
  let keepComposerText = false;
  let reanchorToasted: ComposerAt | null = null;
  $effect(() => {
    const prev = vsRaw;
    if (prev.for === diff || !prev.composer || vs.composer || !sameKey(prev.key)) return;
    if (!diffPaths.has(prev.composer.path) || reanchorToasted === prev.composer) return;
    if (untrack(() => composerText).trim() !== '') {
      reanchorToasted = prev.composer;
      keepComposerText = true;
      toasts.info('File changed — re-anchor your comment', `${prev.composer.path} changed with the new push. Your text is kept; open the comment on the line you mean.`);
    }
  });
  /** Patch the view state of diff `d` — a no-op once `d` is no longer shown. */
  function patchVs(p: Partial<ViewState>, d: DiffResp = diff): void {
    if (d !== diff) return;
    vsRaw = { ...vs, ...p, for: d, key: stateKey };
  }

  let viewedRaw = $state.raw<{ for: DiffResp | null; key: string | undefined; v: Set<string> }>({ for: null, key: undefined, v: new Set() });
  const viewed = $derived(
    viewedRaw.for === diff
      ? viewedRaw.v
      : sameKey(viewedRaw.key)
        ? carryViewed(viewedRaw.v, unchangedPaths(viewedRaw.for, diff))
        : new Set<string>(),
  );

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
  $effect(() => navWin.findRows(() => null, () => navRows, (r) => (r.kind === 'dir' ? r.d.label : (r.f.path.split('/').pop() ?? ''))));
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

  const segs: Seg[] = $derived.by(() => {
    const [s, e] = parseRange(winRange);
    const rs = rows;
    const out: Seg[] = [];
    let g: Seg | null = null;
    for (let i = s; i < e && i < rs.length; i++) {
      const r = rs[i];
      if (inHunk(r)) {
        const gk = `${r.file.path}${ROW_KEY_SEP}g${r.hi}`;
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
        // Nothing came back for this path: `hunks_omitted` keeps it reading
        // as "No diff available", not as an empty (looks-unchanged) body.
        land(d, file.path, fd ?? { ...file, hunks: [], hunks_omitted: true, too_large: false }, null);
      },
      (e: unknown) => {
        done();
        if (isAbortError(e) || ctl.signal.aborted || d !== diff) return;
        land(d, file.path, null, loadErrorText(e));
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
    if (!keepComposerText) composerText = '';
    keepComposerText = false;
  }
  async function submitComment(): Promise<void> {
    const c = vs.composer;
    if (!c || !onAddComment || composerText.trim() === '') return;
    composerBusy = true;
    try {
      await onAddComment(c.path, c.line, composerText.trim(), { side: c.side, oldLine: c.oldLine });
      // Only close the composer that was posted: the user may have opened
      // another line's composer (and typed into it) while this one posted.
      if (vs.composer === c) {
        patchVs({ composer: null });
        composerText = '';
      }
    } finally {
      composerBusy = false;
    }
  }
  // When the comment composer gains focus the soft keyboard can cover it; pull it
  // into view so the textarea + actions stay visible while typing on a phone.
  function composerFocus(e: FocusEvent): void {
    const el = e.currentTarget as HTMLElement | null;
    if (!el) return;
    requestAnimationFrame(() => el.scrollIntoView({ block: 'center', behavior: scrollBehavior() }));
  }

  let selRaw = $state.raw<{ for: DiffResp | null; s: Sel | null }>({ for: null, s: null });
  const sel = $derived(selRaw.for === diff ? selRaw.s : null);
  let applying = $state(false);

  function isSelected(path: string, hi: number, li: number): boolean {
    return sel !== null && sel.path === path && sel.hunk === hi && sel.lines.has(li);
  }

  /** Click = toggle one line; shift-click = the range from the last anchor.
   *  A range picks only the CHANGED lines in it — context lines can't be staged,
   *  and counting them made the button read "Stage 6 lines" for 3 changes. */
  function selectLine(e: MouseEvent, path: string, hi: number, li: number, line: DiffLine, hunkLines?: DiffLine[]): void {
    if (!wip || line.origin === 'context') return;
    const cur = sel !== null && sel.path === path && sel.hunk === hi ? sel : null;
    if (e.shiftKey && cur) {
      const [a, b] = cur.anchor <= li ? [cur.anchor, li] : [li, cur.anchor];
      const lines = new Set(cur.lines);
      for (let i = a; i <= b; i++) if (!hunkLines || hunkLines[i]?.origin !== 'context') lines.add(i);
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
    return n > 0 ? `${verb} ${plural(n, 'line')}` : `${verb} hunk`;
  }

  async function applyHunk(file: FileDiff, hi: number, hunk: Hunk, op: HunkOp): Promise<void> {
    if (!wip || !repoId || applying) return;
    // Capture the line pick BEFORE any await: a re-fetch while the confirm is
    // open swaps `diff`, which drops `sel` — reading it afterwards silently
    // widened "Discard 2 lines" into a whole-hunk discard.
    const shown = diff;
    const picked =
      sel !== null && sel.path === file.path && sel.hunk === hi && sel.lines.size > 0
        ? [...sel.lines].sort((a, b) => a - b)
        : undefined;
    if (op === 'discard') {
      const what = picked ? plural(picked.length, 'changed line') : 'this whole hunk';
      const ok = await confirmer.ask(
        `Discard ${what}? This rewrites the working file and cannot be undone from the file — a backup stash \`otto: backup before hunk discard\` is kept.`,
        { title: picked ? 'Discard lines' : 'Discard hunk', confirmLabel: 'Discard', danger: true },
      );
      if (!ok) return;
      // The diff changed under the dialog: the pick and the hunk index may no
      // longer mean what the user confirmed. Re-pick instead of guessing.
      if (diff !== shown) {
        toasts.info('Diff changed', 'The file changed while you were confirming — nothing was discarded. Pick the lines again.');
        return;
      }
    }
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
          "This hunk can’t be staged partially (renamed/binary file) — stage the whole file.",
        );
      } else if (status === 409 && msg.includes('changed since the diff')) {
        toasts.error(
          'Hunk out of date',
          'The hunk no longer applies — the file changed since the diff was shown; refresh and retry.',
        );
      } else {
        toasts.error('Couldn’t stage', loadErrorText(e));
      }
    } finally {
      applying = false;
    }
  }

  /** File header ⋯ — the diff is the only place a path is at hand, so this is
   *  where History / Blame hang off. The panels live in RepoView; `gitBridge`
   *  carries the request there without prop-drilling the whole graph.
   *
   *  Offered only with `repoId`, i.e. the WORKING-TREE diff (WipPanel), where
   *  the local HEAD is the right revision — so no `rev` is sent. A commit
   *  diff in the graph opens Blame at its own sha (GraphView's file menu); a
   *  PR diff has no drawer to open into and passes no `repoId` (S15-309: a
   *  `rev` prop that no caller set was removed rather than left as dead
   *  plumbing). */
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
      const el = rootEl?.querySelector(`[data-rk="${CSS.escape(`${path}${ROW_KEY_SEP}f`)}"]`);
      el?.scrollIntoView({ block: 'start' });
    });
  }

  function toggleViewed(path: string): void {
    const next = new Set(viewed);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    viewedRaw = { for: diff, key: stateKey, v: next };
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
    // Bare keys only: ⌘[ / ⌘] (back/forward), Ctrl+N and friends are not ours.
    if (e.metaKey || e.ctrlKey || e.altKey) return;
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
;
async () => {


  { const $$_div0 = svelteHTML.createElement("div", {            "class":`diff-root`,"onkeydown":onDiffKeydown,"role":`region`,"aria-label":`Diff viewer`,"tabindex":-1,});showNav && prMode;rootEl = $$_div0;
   const navFileRow/*Ωignore_positionΩ*/ = (file: FileDiff, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
    const stats = fileStat(file);
    const cCount = commentCounts.get(file.path) ?? 0;
    const isViewed = viewed.has(file.path);
    
     { svelteHTML.createElement("div", {        "class":`nav-file`,"style":`padding-inline-start: ${8 + depth * 12}px`,"title":file.path,});isViewed;
       { svelteHTML.createElement("span", { "class":`nav-viewed-cb`,});
         { svelteHTML.createElement("input", {           "type":`checkbox`,"checked":isViewed,"title":`Mark as viewed`,"onchange":() => toggleViewed(file.path),"aria-label":`Mark ${file.path} as viewed`,});}
       }
       { svelteHTML.createElement("button", {     "class":`nav-file-open`,"onclick":() => void scrollToFile(file.path),"aria-label":`Go to ${file.path}`,});
         { svelteHTML.createElement("span", { "class":`nav-file-path`,});
           { svelteHTML.createElement("span", { "class":`nav-base`,});baseName(file.path); }
         }
         { svelteHTML.createElement("span", { "class":`nav-file-stats`,});
           { svelteHTML.createElement("span", { "class":`add`,}); stats.add; }
           { svelteHTML.createElement("span", { "class":`del`,}); stats.del; }
         }
        if(cCount > 0){
            { svelteHTML.createElement("span", {   "class":`nav-comment-badge`,"title":plural(cCount, 'comment'),});
            cCount;
           }
        }
       }
     }
  };return __sveltets_2_any(0)}; const gut/*Ωignore_positionΩ*/ = (side: 'old' | 'new', n: number | null, other: number | null, act: ((e: MouseEvent) => void) | null, kind: 'comment' | 'select', pressed?: boolean)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
    if(act && n != null){
       { svelteHTML.createElement("button", {                "type":`button`,...__sveltets_2_empty({"data-find-skip":true}),"class":`gut ${side}`,"tabindex":side === 'new' || other == null ? 0 : -1,"aria-label":`${kind === 'comment' ? 'Comment on' : 'Select'} line ${n}`,"aria-pressed":kind === 'select' ? pressed === true : undefined,"onclick":act,});kind === 'comment';kind === 'select';n; }
    }else{
      
       { svelteHTML.createElement("span", {     ...__sveltets_2_empty({"data-find-skip":true}),"class":`gut ${side}`,"onclick":act ?? undefined,});!!act && kind === 'comment';!!act && kind === 'select';n ?? ''; }
    }
  };return __sveltets_2_any(0)}; const navDirRow/*Ωignore_positionΩ*/ = (sub: NavDir, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {           "class":`nav-dir-row`,"style":`padding-inline-start: ${8 + depth * 12}px`,"aria-expanded":!navDirCollapsed[sub.path],"onclick":() => toggleNavDir(sub.path),"title":sub.path,});
       { svelteHTML.createElement("span", { "class":`nav-dir-chevron`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":navDirCollapsed[sub.path] ? 'chevronRight' : 'chevronDown',"size":12,}});}
       }
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
       { svelteHTML.createElement("span", { "class":`nav-dir-label`,});sub.label; }
     }
  };return __sveltets_2_any(0)};  const composerBox/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", { "class":`composer`,});
       { svelteHTML.createElement("textarea", {              "dir":`auto`,"aria-label":`Line comment`,"class":`input`,"rows":2,"bind:value":composerText,"onfocus":composerFocus,"placeholder":`Comment on ${vs.composer?.side === 'old' ? 'old ' : ''}line ${vs.composer?.line}…`,});/*Ωignore_startΩ*/() => composerText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("div", { "class":`composer-actions`,});
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => patchVs({ composer: null }),});  }
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":composerBusy || composerText.trim() === '',"onclick":submitComment,});
          composerBusy ? 'Posting…' : 'Comment';
         }
       }
     }
  };return __sveltets_2_any(0)}; const codeText/*Ωignore_positionΩ*/ = (content: string, lang: string | null, key: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {if(content.length > LINE_CUT && !expandedLines.has(key)){ hl.html(content.slice(0, LINE_CUT), lang); { svelteHTML.createElement("button", {           "type":`button`,"class":`line-cut-btn`,...__sveltets_2_empty({"data-find-skip":true}),"aria-label":`Expand line (${kb(content.length)})`,"title":`Show the full line (${kb(content.length)})`,"onclick":(e) => { expandedLines.add(key); void refocusRow(e.currentTarget, rootEl); },});   kb(content.length);  }}else{ hl.html(content, lang);}};return __sveltets_2_any(0)}; const rowView/*Ωignore_positionΩ*/ = (r: Row, i: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
    if(r.kind === 'file'){
      const stats = fileStat(r.file);
      const cCount = commentCounts.get(r.file.path) ?? 0;
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`drow drow-file`,"id":`dfile-${r.file.path}`,...__sveltets_2_empty({"data-rk":r.key}),});r.first;!r.collapsed;
        
         { svelteHTML.createElement("div", { "class":`dfile-headrow`,});
           { svelteHTML.createElement("button", {     "class":`dfile-head`,"aria-expanded":!r.collapsed,"onclick":() => toggleCollapsed(r.file),});
             { svelteHTML.createElement("span", { "class":`dfile-chevron`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":r.collapsed ? 'chevronRight' : 'chevronDown',"size":12,}});}
             }
             { svelteHTML.createElement("span", {   "class":`dfile-path mono`,"dir":`ltr`,});
              if(r.file.old_path){r.file.old_path; { svelteHTML.createElement("span", { "class":`rename-arrow`,});   }}r.file.path;
             }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(prMode && cCount > 0){
                { svelteHTML.createElement("span", {   "class":`file-comment-badge`,...__sveltets_2_empty({"data-find-skip":true}),"title":plural(cCount, 'comment'),});
                 cCount;
               }
            }
             { svelteHTML.createElement("span", {  "class":`add`,...__sveltets_2_empty({"data-find-skip":true}),}); stats.add; }
             { svelteHTML.createElement("span", {  "class":`del`,...__sveltets_2_empty({"data-find-skip":true}),}); stats.del; }
           }
          if(repoId){
             { svelteHTML.createElement("button", {         "class":`dfile-tools`,"title":`File history / blame`,"aria-label":`File tools for ${r.file.path}`,"onclick":(e) => {
                e.stopPropagation();
                fileToolsMenu(e, r.file);
              },}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
          }
         }
       }}
    } else if (r.kind === 'hunk'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {       "class":`drow inf hunk-header mono`,"dir":`ltr`,...__sveltets_2_empty({"data-rk":r.key}),});
         { svelteHTML.createElement("span", {});r.hunk.header; }
        if(wip && repoId){
          const w = wip;
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {       "class":`hunk-btn`,"disabled":applying,"onclick":() => void applyHunk(r.eff, r.hi, r.hunk, w.target === 'staged' ? 'unstage' : 'stage'),});selLabel(w.target === 'staged' ? 'Unstage' : 'Stage', r.file.path, r.hi); }
          if(w.target === 'worktree'){
             { svelteHTML.createElement("button", {       "class":`hunk-btn danger`,"disabled":applying,"onclick":() => void applyHunk(r.eff, r.hi, r.hunk, 'discard'),});selLabel('Discard', r.file.path, r.hi); }
          }
        }
       }}
    } else if (r.kind === 'line'){
      const lang = langOf(r.file.path);
      const pick = !!wip && r.line.origin !== 'context';
      const act = wip ? (pick ? (e: MouseEvent) => selectLine(e, r.file.path, r.hi, r.li, r.line, (vs.loaded.get(r.file.path) ?? r.file).hunks[r.hi]?.lines) : null) : prMode && onAddComment ? () => gutterClick(r.file.path, r.line) : null;
      const picked = pick ? isSelected(r.file.path, r.hi, r.li) : undefined;
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "class":`vrow dline ${r.line.origin}`,...__sveltets_2_empty({"data-rk":r.key}),});picked === true;
        ;__sveltets_2_ensureSnippet(gut('old', r.line.old_line, r.line.new_line, act, wip ? 'select' : 'comment', picked));
        ;__sveltets_2_ensureSnippet(gut('new', r.line.new_line, r.line.old_line, act, wip ? 'select' : 'comment', picked));
         { svelteHTML.createElement("span", {  "class":`sign`,...__sveltets_2_empty({"data-find-skip":true}),});sign(r.line); }
         { svelteHTML.createElement("span", { "class":`code mono`,});;__sveltets_2_ensureSnippet(codeText(r.line.content, lang, r.key)); }
       }}
    } else if (r.kind === 'split'){
      const lang = langOf(r.file.path);
      const L = r.sr.left;
      const R = r.sr.right;
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`vrow split-vrow`,...__sveltets_2_empty({"data-rk":r.key}),});
        ;__sveltets_2_ensureSnippet(gut('old', L?.old_line ?? null, null, prMode && onAddComment && L ? () => gutterClick(r.file.path, L) : null, 'comment'));
         { svelteHTML.createElement("span", {  "class":`code mono half ${L ? (L.origin === 'del' ? 'del' : '') : 'void'}`,});if(L){;__sveltets_2_ensureSnippet(codeText(L.content, lang, r.key + ':L'));} }
        ;__sveltets_2_ensureSnippet(gut('new', R?.new_line ?? null, null, prMode && onAddComment && R ? () => gutterClick(r.file.path, R) : null, 'comment'));
         { svelteHTML.createElement("span", {  "class":`code mono half ${R ? (R.origin === 'add' ? 'add' : '') : 'void'}`,});if(R){;__sveltets_2_ensureSnippet(codeText(R.content, lang, r.key + ':R'));} }
       }}
    } else if (r.kind === 'comment'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`comment-row`,...__sveltets_2_empty({"data-rk":r.key}),});
           for(let c of __sveltets_2_ensureArray(r.comments)){c.id;
           { const $$_daerhTtnemmoC1C = __sveltets_2_ensureComponent(CommentThread); new $$_daerhTtnemmoC1C({ target: __sveltets_2_any(), props: {      "comment":c,"onreply":onReplyComment,"onresolve":onResolveComment,}});}
        }
       }}
    } else if (r.kind === 'composer'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`comment-row`,...__sveltets_2_empty({"data-rk":r.key}),});
        ;__sveltets_2_ensureSnippet(composerBox());
       }}
    } else if (r.kind === 'fcomments'){
      
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`file-comments-block`,...__sveltets_2_empty({"data-rk":r.key}),});
         { svelteHTML.createElement("div", { "class":`file-comments-label dim`,});  }
           for(let c of __sveltets_2_ensureArray(r.comments)){c.id;
          if(c.outdated){
             { svelteHTML.createElement("span", {   "class":`chip outdated-chip`,"title":`The code this comment was on has changed since`,});
              c.line !== null ? ` · ${c.side === 'old' ? 'old ' : ''}line ${c.line}` : '';
             }
          }
           { const $$_daerhTtnemmoC1C = __sveltets_2_ensureComponent(CommentThread); new $$_daerhTtnemmoC1C({ target: __sveltets_2_any(), props: {      "comment":c,"onreply":onReplyComment,"onresolve":onResolveComment,}});}
        }
       }}
    } else if (r.kind === 'more'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`hunk-cap-cell`,...__sveltets_2_empty({"data-rk":r.key}),});
         { svelteHTML.createElement("button", {   "class":`btn small ghost hunk-cap-btn`,"onclick":() => showMore(r.file, r.hi),});
           r.remaining.toLocaleString();  r.remaining === 1 ? 'line' : 'lines';
         }
       }}
    } else if (r.kind === 'note'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`drow inf dfile-binary dim`,...__sveltets_2_empty({"data-rk":r.key}),});r.text; }}
    } else if (r.kind === 'large'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`drow inf dfile-large`,...__sveltets_2_empty({"data-rk":r.key}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":13,}});}
         { svelteHTML.createElement("span", {});   r.lines.toLocaleString();   }
        if(r.canLoad){
           { svelteHTML.createElement("span", { "aria-hidden":`true`,});  }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => loadAnyway(r.file),});  }
        }
       }}
    } else if (r.kind === 'pending'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`drow inf dfile-pending`,...__sveltets_2_empty({"data-rk":r.key}),});
         { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {        "rows":1,"height":14,"label":`this file’s diff`,"grace":false,}});}
       }}
    } else if (r.kind === 'error'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {       "class":`drow inf dfile-error`,"role":`alert`,...__sveltets_2_empty({"data-rk":r.key}),});
         { svelteHTML.createElement("span", {});     r.message; }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => retryLoad(r.file),});  }
       }}
    } else if (r.kind === 'end'){
       {const $$action_0 = __sveltets_2_ensureAction(measure(svelteHTML.mapElementTag('div'),([r.key, i])));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`drow drow-end`,...__sveltets_2_empty({"data-rk":r.key}),}); }}
    }
  };return __sveltets_2_any(0)};
  

  
  

  

  if(showNav && prMode){
     { svelteHTML.createElement("aside", {       "class":`diff-nav`,"style":`--nav-max:${Math.max(160, view.h)}px;${navCollapsed ? '' : `--navw:${navW}px`}`,});navCollapsed;navResizing;
       { svelteHTML.createElement("div", { "class":`nav-header`,});
        if(!navCollapsed){
           { svelteHTML.createElement("span", { "class":`nav-title`,});
            plural(totalFiles, 'file');
             { svelteHTML.createElement("span", { "class":`nav-viewed-count`,}); viewedCount; totalFiles;  }
           }
        }
         { svelteHTML.createElement("button", {         "class":`nav-collapse-btn`,"onclick":() => (navCollapsed = !navCollapsed),"title":navCollapsed ? 'Expand sidebar' : 'Collapse sidebar',"aria-label":navCollapsed ? 'Expand sidebar' : 'Collapse sidebar',});
          
           { svelteHTML.createElement("span", { "class":`nav-collapse-ico`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":navCollapsed ? 'chevronRight' : 'chevronLeft',"size":12,}});}
           }
         }
       }

      if(!navCollapsed){
        
         { svelteHTML.createElement("div", { "class":`nav-search-wrap`,});
           { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`nav-search input`,"type":`text`,"placeholder":`Filter files…`,"bind:value":rawSearch,"aria-label":`Filter files by path or content`,});/*Ωignore_startΩ*/() => rawSearch = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(search){
             { svelteHTML.createElement("span", { "class":`nav-match-count`,});matchCount; }
          }
         }

         { svelteHTML.createElement("div", { "class":`nav-files`,});
           { svelteHTML.createElement("div", {  "class":`nav-rows`,[Symbol("@attach")]:navWin.attach,});
            if(navRange.top > 0){ { svelteHTML.createElement("div", {   "style":`height:${navRange.top}px`,"aria-hidden":`true`,}); }}
               for(let r of __sveltets_2_ensureArray((navRows.slice(navRange.start, navRange.end)))){r.key;
              if(r.kind === 'dir'){
                ;__sveltets_2_ensureSnippet(navDirRow(r.d, r.depth));
              }else{
                ;__sveltets_2_ensureSnippet(navFileRow(r.f, r.depth));
              }
            }
            if(navRange.bottom > 0){ { svelteHTML.createElement("div", {   "style":`height:${navRange.bottom}px`,"aria-hidden":`true`,}); }}
           }
         }
        
        
         {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: navW, min: 180, max: 520, onChange: setNavW, onReset: () => setNavW(240), onDragStart: startNavResize, text: (v) => `${Math.round(v)} pixels wide` })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`nav-resize-handle`,"role":`separator`,"aria-label":`Resize file sidebar`,"title":`Drag or use ←/→ to resize · double-click or Enter to reset`,}); }}
      }
     }
  }

  

  
  
  

  

  
   { svelteHTML.createElement("div", { "class":`diff`,});
     { svelteHTML.createElement("div", { "class":`diff-toolbar`,});
       { svelteHTML.createElement("span", { "class":`diff-stats`,});
        if(search){
           { svelteHTML.createElement("span", {});matchCount;  plural(diff.files.length, 'file'); }
        }else{
          plural(diff.files.length, 'file');
        }
         { svelteHTML.createElement("span", { "class":`add`,}); totals.add; }
         { svelteHTML.createElement("span", { "class":`del`,}); totals.del; }
        if(diff.truncated || (lazyCount > 0 && !loadFile)){
           { svelteHTML.createElement("span", {   "class":`diff-note`,"title":`The diff is too large to send at once; open a file to load its lines.`,});
                   
           }
        }
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(!showNav || !prMode){
        
         { svelteHTML.createElement("div", { "class":`toolbar-search-wrap`,});
           { svelteHTML.createElement("input", {            "dir":`auto`,"class":`toolbar-search input`,"type":`text`,"placeholder":`Search files & diff…`,"bind:value":rawSearch,"aria-label":`Search files and diff content`,});/*Ωignore_startΩ*/() => rawSearch = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(search){
             { svelteHTML.createElement("span", { "class":`toolbar-match-count`,});matchCount; matchCount === 1 ? '' : 'es'; }
          }
         }
      }
      if(diff.files.length > 1){
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => setAllCollapsed(!allCollapsed),"title":allCollapsed ? 'Expand every file (only the rows on screen render)' : 'Collapse every file',});allCollapsed ? 'Expand all' : 'Collapse all'; }
      }
      if(!isMobile){
        
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Diff layout`,});
           { svelteHTML.createElement("button", {     "aria-pressed":mode === 'unified',"onclick":() => (mode = 'unified'),});mode === 'unified';  }
           { svelteHTML.createElement("button", {     "aria-pressed":mode === 'split',"onclick":() => (mode = 'split'),});mode === 'split';   }
         }
      }
     }

    if(filteredFiles.length === 0){
       { svelteHTML.createElement("div", { "class":`dim diff-empty`,});
        search ? 'No files match your search.' : 'No changes.';
       }
    }else{
      
       { const $$_div2 = svelteHTML.createElement("div", {    "class":`diff-body`,"style":`padding-block: ${pads.top}px ${pads.bottom}px`,});bodyEl = $$_div2;
           for(let seg of __sveltets_2_ensureArray(segs)){seg.key;
          if(seg.group){
             { svelteHTML.createElement("div", {    "class":`dtable`,"dir":`ltr`,});seg.split;
                 for(let it of __sveltets_2_ensureArray(seg.items)){it.r.key;
                if(isOverlay(it.r)){
                   { svelteHTML.createElement("div", {   "class":`drow-spacer`,"style":`height: ${spacerH(it.i)}px`,}); }
                }else{
                  ;__sveltets_2_ensureSnippet(rowView(it.r, it.i));
                }
              }
             }
          } else if (isOverlay(seg.items[0].r)){
             { svelteHTML.createElement("div", {   "class":`drow-spacer`,"style":`height: ${spacerH(seg.items[0].i)}px`,}); }
          }else{
            ;__sveltets_2_ensureSnippet(rowView(seg.items[0].r, seg.items[0].i));
          }
        }
           for(let o of __sveltets_2_ensureArray(overlays)){o.r.key;
           { svelteHTML.createElement("div", {   "class":`drow-overlay`,"style":`top: ${o.top}px`,});
            ;__sveltets_2_ensureSnippet(rowView(o.r, o.i));
           }
        }
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DiffViewer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DiffViewer__SvelteComponent_ = ReturnType<typeof DiffViewer__SvelteComponent_>;
/*Ωignore_endΩ*/export default DiffViewer__SvelteComponent_;