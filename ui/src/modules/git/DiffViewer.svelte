<script lang="ts">
  import { scrollBehavior } from '../../lib/motion';
  import { loadErrorText } from '../../lib/loadError';
  import { plural } from '../../lib/plural';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  // Shared diff renderer (Changes / commit / PR views): unified or side-by-side, collapse,
  // highlight; PR mode adds threads, composer and a file tree + toolbar (lazy chunks).
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
  import { tick, untrack, type Snippet } from 'svelte';
  import { isTestPath } from './testFiles';
  import { loadViewed, saveViewed } from './diff-viewed-store';
  import { publishPrFiles } from './prFilesBridge.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { PREF_HIDE_TESTS, PREF_TREE, PREF_WRAP, buildNavTree, fileStatus, readPref, writePref } from './diff-pr';
  const DiffPrToolbarLazy = lazyComponent(() => import('./DiffPrToolbar.svelte'));
  const DiffNavLazy = lazyComponent(() => import('./DiffNav.svelte'));
  import './diff-pr.css';
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
  import { DeferredHighlighter } from './diff-highlight.svelte';
  import { SvelteSet } from 'svelte/reactivity';

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
    /** PR mode: rendered at the very start of the diff toolbar (e.g. the
     *  host's "All commits ▾" picker). */
    toolbarLead?: Snippet;
    /** PR mode: persist "Viewed" marks in localStorage under this key. A mark
     *  is pinned to the file's status + line counts, so a push that changes
     *  the file drops it; an untouched file stays viewed across reloads. */
    viewedStorageKey?: string;
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
    stateKey,
    toolbarLead,
    viewedStorageKey,
  }: Props = $props();

  // ── PR-mode view preferences (persisted; helpers in diff-pr.ts) ──────────
  /** File tree (the nav sidebar) shown — PR mode, hidden by default. */
  let treeOpen = $state(readPref(PREF_TREE, false));
  /** Test files filtered out of rows, tree and counts — PR mode. */
  let hideTests = $state(readPref(PREF_HIDE_TESTS, false));
  /** Soft-wrap long code lines (desktop; ≤1024 always wraps). */
  let wrapLines = $state(readPref(PREF_WRAP, true));
  function toggleTree(): void { treeOpen = !treeOpen; writePref(PREF_TREE, treeOpen); }
  function toggleHideTests(): void { hideTests = !hideTests; writePref(PREF_HIDE_TESTS, hideTests); }
  function toggleWrap(): void { wrapLines = !wrapLines; writePref(PREF_WRAP, wrapLines); }
  /** The nav sidebar is on screen (host asked for it AND, in PR mode, the tree toggle is on). */
  const navShown = $derived(showNav && prMode && treeOpen);
  /** The files this view shows: everything, minus test files when hidden (PR mode). */
  const visibleFiles = $derived(prMode && hideTests ? diff.files.filter((f) => !isTestPath(f.path)) : diff.files);
  const hiddenTests = $derived(diff.files.length - visibleFiles.length);

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
  }
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
        : prMode && viewedStorageKey
          ? loadViewed(viewedStorageKey, diff.files)
          : new Set<string>(),
  );

  let composerText = $state('');
  let composerBusy = $state(false);

  // ── Nav tree (diff-pr.ts): directories collapsible, files keep the viewed
  // checkbox / stats / comment badge.
  const navTree = $derived(buildNavTree(visibleFiles));
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
    for (const f of visibleFiles) if (fileMatches(vs.loaded.get(f.path) ?? f, search)) s.add(f.path);
    return s;
  });
  const filteredFiles = $derived(matchSet ? visibleFiles.filter((f) => matchSet.has(f.path)) : visibleFiles);
  const matchCount = $derived(filteredFiles.length);

  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });

  // ── Stats + collapse defaults (derived — never an after-render effect) ────
  const totals = $derived.by(() => {
    if (hiddenTests === 0 && diff.total_added != null && diff.total_deleted != null) {
      return { add: diff.total_added, del: diff.total_deleted };
    }
    let add = 0;
    let del = 0;
    for (const f of visibleFiles) {
      const s = fileStat(f);
      add += s.add;
      del += s.del;
    }
    return { add, del };
  });
  const collapseAll = $derived(
    diff.files.length > COLLAPSE_ALL_FILES || totals.add + totals.del > COLLAPSE_ALL_LINES,
  );
  /** PR mode opens on a collapsed file list (T3/GitHub style) past this many files. */
  const PR_COLLAPSED_FILES = 5;
  function defaultCollapsed(f: FileDiff): boolean {
    if (collapseAll || isGenerated(f.path)) return true;
    if (prMode && (diff.files.length > PR_COLLAPSED_FILES || viewed.has(f.path))) return true;
    const s = fileStat(f);
    return s.add + s.del > FILE_COLLAPSE_LINES;
  }
  function isCollapsed(f: FileDiff): boolean {
    return vs.overrides[f.path] ?? defaultCollapsed(f);
  }
  function toggleCollapsed(f: FileDiff): void {
    patchVs({ overrides: { ...vs.overrides, [f.path]: !isCollapsed(f) } });
  }
  const allCollapsed = $derived(visibleFiles.every((f) => isCollapsed(f)));
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
    const files = visibleFiles;
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
  // Touch: the soft keyboard can cover the composer — centre it. Mouse: only bring the whole box (buttons
  // too) into view, at once; a smooth re-centre on each focus slid Comment away under the pointer.
  function composerFocus(e: FocusEvent): void {
    const el = e.currentTarget as HTMLElement | null;
    if (!el) return;
    const touch = matchMedia('(pointer: coarse)').matches;
    const box = touch ? el : (el.closest('.composer') ?? el);
    requestAnimationFrame(() => box.scrollIntoView(touch ? { block: 'center', behavior: scrollBehavior() } : { block: 'nearest' }));
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
    const nowViewed = !next.has(path);
    if (nowViewed) next.add(path);
    else next.delete(path);
    viewedRaw = { for: diff, key: stateKey, v: next };
    // GitHub-style: ticking "Viewed" folds the file away; unticking reopens it.
    if (prMode) patchVs({ overrides: { ...vs.overrides, [path]: nowViewed } });
    if (prMode && viewedStorageKey) saveViewed(viewedStorageKey, diff.files, next);
  }

  // The PR workspace's Files panel mirrors these files, marks and jumps (prFilesBridge).
  publishPrFiles(() => (prMode && viewedStorageKey ? { key: viewedStorageKey, files: visibleFiles, viewed, commentCounts, current: rows[rowAt(layout.offsets, layout.n, view.top + 1)]?.file.path ?? null, jump: scrollToFile, toggleViewed } : null));
  const viewedCount = $derived(visibleFiles.reduce((n, f) => n + (viewed.has(f.path) ? 1 : 0), 0));
  const totalFiles = $derived(visibleFiles.length);
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
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="diff-root"
  class:with-nav={navShown}
  class:pr={prMode}
  class:nowrap={prMode && !wrapLines}
  bind:this={rootEl}
  onkeydown={onDiffKeydown}
  role="region"
  aria-label="Diff viewer"
  tabindex="-1"
>
  <!-- A line-number gutter. When it can act (comment on the line / select it for
       staging) it is a real <button> with a spoken label; of a row's two gutters
       only one is a tab stop (the other stays mouse-clickable) so a diff does not
       double its tab order. A cell with no number is a plain, inert box. -->
  {#snippet gut(side: 'old' | 'new', n: number | null, other: number | null, act: ((e: MouseEvent) => void) | null, kind: 'comment' | 'select', pressed?: boolean)}
    {#if act && n != null}
      <button
        type="button"
        data-find-skip
        class="gut {side}"
        class:commentable={kind === 'comment'}
        class:selectable={kind === 'select'}
        tabindex={side === 'new' || other == null ? 0 : -1}
        aria-label="{kind === 'comment' ? 'Comment on' : 'Select'} line {n}"
        aria-pressed={kind === 'select' ? pressed === true : undefined}
        onclick={act}
      >{n}</button>
    {:else}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <span data-find-skip class="gut {side}" class:commentable={!!act && kind === 'comment'} class:selectable={!!act && kind === 'select'} onclick={act ?? undefined}>{n ?? ''}</span>
    {/if}
  {/snippet}

  {#if navShown && DiffNavLazy.component}{@const DiffNav = DiffNavLazy.component}
    <DiffNav
      tree={navTree}
      {matchSet}
      {viewed}
      {commentCounts}
      {totalFiles}
      {viewedCount}
      {matchCount}
      searching={!!search}
      maxH={view.h}
      bind:rawSearch
      onjump={(path) => void scrollToFile(path)}
      ontoggleviewed={toggleViewed}
    />
  {/if}

  {#snippet composerBox()}
    <div class="composer">
      <textarea dir="auto" aria-label="Line comment"
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
        onclick={(e) => { expandedLines.add(key); void refocusRow(e.currentTarget, rootEl); }}>… expand line ({kb(content.length)})</button
      >{:else}{@html hl.html(content, lang)}{/if}{/snippet}

  {#snippet rowView(r: Row, i: number)}
    {#if r.kind === 'file'}
      {@const stats = fileStat(r.file)}
      {@const cCount = commentCounts.get(r.file.path) ?? 0}
      {@const isViewed = prMode && viewed.has(r.file.path)}
      {@const st = prMode ? fileStatus(r.file) : null}
      <div
        class="drow drow-file"
        class:first={r.first}
        class:open={!r.collapsed}
        class:viewed={isViewed}
        id="dfile-{r.file.path}"
        data-rk={r.key}
        use:measure={[r.key, i]}
      >
        <!-- Row, not a single button: the ⋯ menu can't nest inside the collapse
             button (nested <button> is invalid HTML and swallows the click). -->
        <div class="dfile-headrow">
          <button class="dfile-head" aria-expanded={!r.collapsed} onclick={() => toggleCollapsed(r.file)}>
            <span class="dfile-chevron">
              <Icon name={r.collapsed ? 'chevronRight' : 'chevronDown'} size={12} />
            </span>
            {#if st}
              <span class="fstat fstat-{st.kind}" data-find-skip role="img" aria-label={st.label} title={st.label}>{st.glyph}</span>
            {/if}
            <span class="dfile-path mono" dir="ltr">
              {#if r.file.old_path}{r.file.old_path}<span class="rename-arrow"> → </span>{/if}{r.file.path}
            </span>
            <span class="grow"></span>
            {#if prMode && cCount > 0}
              <span class="file-comment-badge" data-find-skip title="{plural(cCount, 'comment')}">
                💬 {cCount}
              </span>
            {/if}
            <span class="add" data-find-skip>+{stats.add}</span>
            <span class="del" data-find-skip>−{stats.del}</span>
          </button>
          {#if prMode}
            <!-- Sibling of the collapse button, never nested inside it. -->
            <label class="dfile-viewed" data-find-skip title={isViewed ? 'Viewed — untick to reopen' : 'Mark as viewed (folds the file away)'}>
              <input
                type="checkbox"
                checked={isViewed}
                onchange={() => toggleViewed(r.file.path)}
                aria-label="Mark {r.file.path} as viewed"
              />
              <span>Viewed</span>
            </label>
          {/if}
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
      {@const act = wip ? (pick ? (e: MouseEvent) => selectLine(e, r.file.path, r.hi, r.li, r.line, (vs.loaded.get(r.file.path) ?? r.file).hunks[r.hi]?.lines) : null) : prMode && onAddComment ? () => gutterClick(r.file.path, r.line) : null}
      {@const picked = pick ? isSelected(r.file.path, r.hi, r.li) : undefined}
      <div
        class="vrow dline {r.line.origin}"
        class:selected={picked === true}
        data-rk={r.key}
        use:measure={[r.key, i]}
      >
        {@render gut('old', r.line.old_line, r.line.new_line, act, wip ? 'select' : 'comment', picked)}
        {@render gut('new', r.line.new_line, r.line.old_line, act, wip ? 'select' : 'comment', picked)}
        <span class="sign" data-find-skip>{sign(r.line)}</span>
        <span class="code mono">{@render codeText(r.line.content, lang, r.key)}</span>
      </div>
    {:else if r.kind === 'split'}
      {@const lang = langOf(r.file.path)}
      {@const L = r.sr.left}
      {@const R = r.sr.right}
      <div class="vrow split-vrow" data-rk={r.key} use:measure={[r.key, i]}>
        {@render gut('old', L?.old_line ?? null, null, prMode && onAddComment && L ? () => gutterClick(r.file.path, L) : null, 'comment')}
        <span class="code mono half {L ? (L.origin === 'del' ? 'del' : '') : 'void'}"
          >{#if L}{@render codeText(L.content, lang, r.key + ':L')}{/if}</span>
        {@render gut('new', R?.new_line ?? null, null, prMode && onAddComment && R ? () => gutterClick(r.file.path, R) : null, 'comment')}
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
          Show {r.remaining.toLocaleString()} more {r.remaining === 1 ? 'line' : 'lines'}
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
      <div class="drow inf dfile-pending" data-rk={r.key} use:measure={[r.key, i]}>
        <Skeleton rows={1} height={14} label="this file’s diff" grace={false} />
      </div>
    {:else if r.kind === 'error'}
      <div class="drow inf dfile-error" role="alert" data-rk={r.key} use:measure={[r.key, i]}>
        <span>Couldn’t load this file’s diff: {r.message}</span>
        <button class="btn small" onclick={() => retryLoad(r.file)}>Retry</button>
      </div>
    {:else if r.kind === 'end'}
      <div class="drow drow-end" data-rk={r.key} use:measure={[r.key, i]}></div>
    {/if}
  {/snippet}

  <!-- Diff main area -->
  <div class="diff">
    {#if prMode}
      {@const DiffPrToolbar = DiffPrToolbarLazy.component}{#if DiffPrToolbar}<DiffPrToolbar
        lead={toolbarLead}
        {totalFiles}
        {matchCount}
        searching={!!search}
        {viewedCount}
        {hideTests}
        {hiddenTests}
        add={totals.add}
        del={totals.del}
        largeNote={!!diff.truncated || (lazyCount > 0 && !loadFile)}
        bind:rawSearch
        showSearch={!navShown}
        canTree={showNav}
        {treeOpen}
        {isMobile}
        bind:mode
        {wrapLines}
        canFold={visibleFiles.length > 1}
        {allCollapsed}
        ontoggletree={toggleTree}
        ontogglewrap={toggleWrap}
        ontoggletests={toggleHideTests}
        onfold={() => setAllCollapsed(!allCollapsed)}
      />{:else}<div class="diff-toolbar" aria-hidden="true"></div>{/if}
    {:else}
    <div class="diff-toolbar">
      <span class="diff-stats">
        {#if search}
          <span>{matchCount} / {plural(diff.files.length, 'file')}</span>
        {:else}
          {plural(diff.files.length, 'file')}
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
          <input dir="auto"
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
        <div class="segmented" role="group" aria-label="Diff layout">
          <button class:active={mode === 'unified'} aria-pressed={mode === 'unified'} onclick={() => (mode = 'unified')}>Unified</button>
          <button class:active={mode === 'split'} aria-pressed={mode === 'split'} onclick={() => (mode = 'split')}>Side by side</button>
        </div>
      {/if}
    </div>

    {/if}

    {#if filteredFiles.length === 0}
      <div class="dim diff-empty">
        {search ? 'No files match your search.' : hiddenTests > 0 ? 'Only test files changed — they’re hidden.' : 'No changes.'}
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
    padding: 6px 12px;
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
    padding: 2px 12px;
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
  /* The actionable gutters are real buttons: reset the UA button chrome. */
  button.gut {
    appearance: none;
    margin: 0;
    background: none;
    border-block: 0;
    border-inline-start: 0;
    line-height: inherit;
    border-radius: 0;
  }
  button.gut:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .gut.commentable,
  .gut.selectable {
    cursor: pointer;
  }
  .gut.commentable:hover,
  .gut.selectable:hover {
    background: var(--accent-soft-strong);
    color: var(--accent-text);
  }
  /* Small ghost actions in the sticky hunk header — accent, never louder than
     the code they sit above. */
  .hunk-btn {
    flex-shrink: 0;
    padding: 1px 6px;
    border: 1px solid var(--accent-line);
    border-radius: var(--radius-s);
    background: none;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .hunk-btn:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .hunk-btn:disabled {
    opacity: var(--disabled-opacity);
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
    background: var(--accent-soft);
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
    .diff-root.with-nav .diff {
      padding-inline-start: 0;
    }

    .diff-toolbar { flex-wrap: wrap; gap: 8px; }
    .toolbar-search-wrap { flex: 1; min-width: 120px; }
    .diff-stats { font-size: var(--fs-m); }
    .add, .del { font-size: var(--fs-m); }
    .toolbar-search { font-size: var(--fs-m); height: 32px; width: 100%; }
    .dfile-head { font-size: var(--fs-m); padding: 8px 12px; min-height: 40px; }
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
      padding-block: 0; padding-inline: 2px 4px;
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
    /* "Show N more lines" hunk-cap button. */
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
      color: var(--accent-line-strong);
      pointer-events: none;
    }
  }

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

  /* Hunk line cap: "Show N more lines" affordance */
  .hunk-cap-cell {
    text-align: center;
    padding: 4px 8px;
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
