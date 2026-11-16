<script lang="ts">
  // Shared windowed diff viewer (unified and side-by-side) for local changes,
  // commits, and history comparisons across IDE and Git modules.
  import { tick, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { isAbortError } from '../api/client';
  import type { DiffLine, DiffResp, FileDiff, Hunk, HunkOp, StageHunkResp } from '../api/types';
  import { ctxMenu } from '../contextmenu.svelte';
  import { registerFindProvider } from '../findProviders';
  import { ensureHljs, langFromPath } from '../hl';
  import { loadErrorText } from '../loadError';
  import { plural } from '../plural';
  import DiffRowView from './DiffRowView.svelte';
  import DiffToolbar from './DiffToolbar.svelte';
  import { fileNavDelta } from './diff-dom';
  import { DeferredHighlighter } from './diff-highlight.svelte';
  import type { DiffFileLoader } from './diff-load';
  import {
    COLLAPSE_ALL_FILES,
    COLLAPSE_ALL_LINES,
    FILE_COLLAPSE_LINES,
    ROW_KEY_SEP,
    estimateRow,
    fileMatches,
    fileStat,
    findTextOf,
    isGenerated,
    type ComposerAt,
    type Row,
  } from './diff-model';
  import {
    activeDiffScroller,
    buildCachedDiffRows,
    buildDiffSegments,
    computeDiffTotals,
    parseRange,
    readSplitPref,
    runStageOrDiscardHunk,
    toggleDiffLineSelection,
    writeSplitPref,
    type DiffLineSelection,
    type DiffRowCache,
    type DiffSegment,
  } from './diff-viewer-helpers';
  import { carryViewState, unchangedPaths } from './diff-viewstate';
  import { resum, rowAt } from './diff-virtual';
  import { gitBridge } from './gitBridge.svelte';

  interface Props {
    diff: DiffResp;
    repoId?: string;
    wip?: {
      target: 'worktree' | 'staged';
      onapplied: (r: StageHunkResp) => void;
    };
    loadFile?: DiffFileLoader;
    stateKey?: string;
  }

  let { diff, repoId, wip, loadFile, stateKey }: Props = $props();

  let mode = $state<'unified' | 'split'>(readSplitPref() ? 'split' : 'unified');
  $effect(() => writeSplitPref(mode === 'split'));

  let hlReady = $state(false);
  const hl = new DeferredHighlighter();
  const expandedLines = new SvelteSet<string>();

  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  const effMode = $derived<'unified' | 'split'>(isMobile ? 'unified' : mode);

  interface ViewState {
    for: DiffResp | null;
    key: string | undefined;
    overrides: Record<string, boolean>;
    composer: ComposerAt | null;
    uncapped: Map<string, Set<number>>;
    loaded: Map<string, FileDiff>;
    errors: Map<string, string>;
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
  const sameKey = (k: string | undefined): boolean => stateKey !== undefined && k === stateKey;
  const vs: ViewState = $derived(
    vsRaw.for === diff
      ? vsRaw
      : sameKey(vsRaw.key)
        ? { ...freshState(diff), ...carryViewState(vsRaw, unchangedPaths(vsRaw.for, diff)) }
        : freshState(diff),
  );
  function patchVs(p: Partial<ViewState>, d: DiffResp = diff): void {
    if (d !== diff) return;
    vsRaw = { ...vs, ...p, for: d, key: stateKey };
  }

  let rawSearch = $state('');
  let search = $state('');
  $effect(() => {
    const val = rawSearch;
    const t = setTimeout(() => (search = val.trim().toLowerCase()), 200);
    return () => clearTimeout(t);
  });
  const filteredFiles = $derived.by(() => {
    if (!search) return diff.files;
    return diff.files.filter((f) => fileMatches(vs.loaded.get(f.path) ?? f, search));
  });
  const matchCount = $derived(filteredFiles.length);

  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });

  const totals = $derived(computeDiffTotals(diff));
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
  const EXPAND_CHUNK = 150;
  let expandFrame = 0;
  $effect(() => () => {
    if (expandFrame) cancelAnimationFrame(expandFrame);
  });
  function setAllCollapsed(v: boolean): void {
    if (expandFrame) cancelAnimationFrame(expandFrame);
    expandFrame = 0;
    const d = diff;
    const files = diff.files;
    if (files.length <= EXPAND_CHUNK) {
      const next: Record<string, boolean> = {};
      for (const f of files) next[f.path] = v;
      patchVs({ overrides: next });
      return;
    }
    const topRow = rows[rowAt(layout.offsets, layout.n, view.top)];
    const at = Math.max(0, topRow ? files.indexOf(topRow.file) : 0);
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

  let rowCache: DiffRowCache = new Map();
  let rowCacheFor: DiffResp | null = null;
  const rows: Row[] = $derived.by(() => {
    if (rowCacheFor !== diff) {
      rowCache = new Map();
      rowCacheFor = diff;
    }
    const built = buildCachedDiffRows({
      files: filteredFiles,
      overrides: vs.overrides,
      defaultCollapsed,
      uncappedMap: vs.uncapped,
      loadedMap: vs.loaded,
      errorsMap: vs.errors,
      fullSet: vs.full,
      split: effMode === 'split',
      canLoad: !!loadFile,
      prevCache: rowCache,
    });
    rowCache = built.nextCache;
    return built.rows;
  });

  let measured = new Map<string, number>();
  let measuredFor: DiffResp | null = null;
  let measuredMobile = false;
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

  let view = $state({ top: 0, h: 1200 });
  const OVERSCAN_PX = 800;
  const winRange = $derived.by(() => {
    void measureVersion;
    const { offsets, n } = layout;
    if (n === 0) return '0:0';
    const start = rowAt(offsets, n, view.top - OVERSCAN_PX);
    const end = Math.min(n, rowAt(offsets, n, view.top + view.h + OVERSCAN_PX) + 1);
    return `${start}:${end}`;
  });
  const pads = $derived.by(() => {
    void measureVersion;
    const [s, e] = parseRange(winRange);
    const { offsets, n } = layout;
    return { top: offsets[s] ?? 0, bottom: (offsets[n] ?? 0) - (offsets[e] ?? 0) };
  });
  const segs: DiffSegment[] = $derived.by(() => {
    const [s, e] = parseRange(winRange);
    return buildDiffSegments(rows, s, e, effMode === 'split');
  });

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
      if (idx < 0) {
        for (let j = 0; j < rs.length; j++) {
          if (rs[j].key === a.key) {
            idx = j;
            break;
          }
        }
      }
      if (idx < 0) {
        captureAnchor(view.top);
        return;
      }
      const d = l.offsets[idx] + a.delta - view.top;
      scrollAnchor = { key: a.key, i: idx, delta: a.delta };
      if (Math.abs(d) < 1) return;
      const sc = activeDiffScroller(bodyEl);
      if (sc) sc.scrollTop += d;
      else window.scrollBy(0, d);
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
      if (getComputedStyle(p).overflowY !== 'visible') list.push(p);
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

  const rowMeta = new WeakMap<Element, { key: string; i: number }>();
  const rowRO = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver((entries) => onMeasure(entries));
  $effect(() => () => rowRO?.disconnect());
  let measureFrame = 0;
  $effect(() => () => {
    if (measureFrame) cancelAnimationFrame(measureFrame);
    measureFrame = 0;
  });
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
      if (!h) continue;
      measured.set(m.key, h);
      if (m.i >= n || rs[m.i]?.key !== m.key || heights[m.i] === h) continue;
      if (m.i < anchor) aboveDelta += h - heights[m.i];
      heights[m.i] = h;
      if (m.i < minI) minI = m.i;
    }
    if (minI === Infinity) return;
    resum(heights, offsets, minI);
    if (Math.abs(aboveDelta) >= 1) {
      const sc = activeDiffScroller(bodyEl);
      if (sc) sc.scrollTop += aboveDelta;
      else window.scrollBy(0, aboveDelta);
    }
    if (!measureFrame) {
      measureFrame = requestAnimationFrame(() => {
        measureFrame = 0;
        measureVersion++;
      });
    }
  }
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

  const MAX_LOADS = 4;
  const inflight = new Map<string, AbortController>();
  $effect(() => {
    void diff;
    return () => {
      for (const c of inflight.values()) c.abort();
      inflight.clear();
      hl.clear();
      expandedLines.clear();
    };
  });
  $effect(() => {
    const [s, e] = parseRange(winRange);
    const rs = rows;
    const lf = loadFile;
    if (!lf) return;
    untrack(() => {
      const { offsets, n } = layout;
      const vis = Math.min(Math.max(s, rowAt(offsets, n, view.top)), e);
      for (let i = vis; i < e && i < rs.length; i++) {
        if (rs[i].kind === 'pending') startLoad(rs[i].file, lf);
      }
      for (let i = s; i < vis && i < rs.length; i++) {
        if (rs[i].kind === 'pending') startLoad(rs[i].file, lf);
      }
    });
  });

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

  let selRaw = $state.raw<{ for: DiffResp | null; s: DiffLineSelection | null }>({ for: null, s: null });
  const sel = $derived(selRaw.for === diff ? selRaw.s : null);
  let applying = $state(false);

  function isSelected(path: string, hi: number, li: number): boolean {
    return sel !== null && sel.path === path && sel.hunk === hi && sel.lines.has(li);
  }
  function selectLine(
    e: MouseEvent,
    path: string,
    hi: number,
    li: number,
    line: DiffLine,
    hunkLines?: DiffLine[],
  ): void {
    if (!wip) return;
    selRaw = {
      for: diff,
      s: toggleDiffLineSelection(sel, e.shiftKey, path, hi, li, line, hunkLines),
    };
  }
  function selLabel(verb: string, path: string, hi: number): string {
    const n = sel !== null && sel.path === path && sel.hunk === hi ? sel.lines.size : 0;
    return n > 0 ? `${verb} ${plural(n, 'line')}` : `${verb} hunk`;
  }

  async function applyHunk(file: FileDiff, hi: number, hunk: Hunk, op: HunkOp): Promise<void> {
    if (!wip || !repoId || applying) return;
    const shown = diff;
    applying = true;
    try {
      const r = await runStageOrDiscardHunk({
        repoId,
        file,
        hi,
        hunk,
        op,
        sel,
        isStale: () => diff !== shown,
      });
      if (r) {
        selRaw = { for: null, s: null };
        wip.onapplied(r);
      }
    } finally {
      applying = false;
    }
  }

  function fileToolsMenu(e: MouseEvent, file: FileDiff): void {
    if (!repoId) return;
    ctxMenu.show(e, [
      { label: 'History', icon: 'note', action: () => gitBridge.openFileTool({ kind: 'history', repoId, path: file.path }) },
      { label: 'Blame', icon: 'note', action: () => gitBridge.openFileTool({ kind: 'blame', repoId, path: file.path }) },
    ]);
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
        const sc = activeDiffScroller(b);
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

  async function scrollToFile(path: string): Promise<void> {
    const f = diff.files.find((x) => x.path === path);
    if (f && isCollapsed(f)) patchVs({ overrides: { ...vs.overrides, [path]: false } });
    await tick();
    const idx = rows.findIndex((r) => r.kind === 'file' && r.file.path === path);
    const b = bodyEl;
    if (idx < 0 || !b) return;
    const y = layout.offsets[idx];
    const sc = activeDiffScroller(b);
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

  const lazyCount = $derived(diff.files.reduce((n, f) => n + (f.hunks_omitted ? 1 : 0), 0));
  let navFocusIdx = $state(-1);
  function navToFile(delta: number): void {
    const files = filteredFiles;
    if (files.length === 0) return;
    const next = Math.max(0, Math.min(files.length - 1, navFocusIdx + delta));
    navFocusIdx = next;
    void scrollToFile(files[next].path);
  }
  function onDiffKeydown(e: KeyboardEvent): void {
    const d = fileNavDelta(e);
    if (d === 0) return;
    e.preventDefault();
    navToFile(d);
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
</script>

<div
  class="diff-root"
  bind:this={rootEl}
  role="region"
  aria-label="Diff viewer"
  tabindex="-1"
  {@attach (el) => {
    el.addEventListener('keydown', onDiffKeydown);
    return () => el.removeEventListener('keydown', onDiffKeydown);
  }}
>
  <div class="diff">
    <DiffToolbar
      fileCount={diff.files.length}
      {matchCount}
      totalAdded={totals.add}
      totalDeleted={totals.del}
      showLargeDiffNote={Boolean(diff.truncated || (lazyCount > 0 && !loadFile))}
      {rawSearch}
      {search}
      {allCollapsed}
      {isMobile}
      {mode}
      onsearchchange={(next) => (rawSearch = next)}
      ontoggleallcollapsed={() => setAllCollapsed(!allCollapsed)}
      onmodechange={(next) => (mode = next)}
    />

    {#if filteredFiles.length === 0}
      <div class="dim diff-empty">{search ? 'No files match your search.' : 'No changes.'}</div>
    {:else}
      <div class="diff-body" bind:this={bodyEl} style="padding-block: {pads.top}px {pads.bottom}px">
        {#each segs as seg (seg.key)}
          {#if seg.group}
            <div class="dtable" class:split={seg.split} dir="ltr">
              {#each seg.items as it (it.r.key)}
                <DiffRowView
                  r={it.r}
                  i={it.i}
                  {repoId}
                  {wip}
                  {applying}
                  {hl}
                  {expandedLines}
                  loadedFiles={vs.loaded}
                  {rootEl}
                  {measure}
                  {langOf}
                  {isSelected}
                  {selLabel}
                  ontogglefile={toggleCollapsed}
                  onfiletools={fileToolsMenu}
                  onapplyhunk={(file, hi, hunk, op) => void applyHunk(file, hi, hunk, op)}
                  onselectline={selectLine}
                  onshowmore={showMore}
                  onloadanyway={loadAnyway}
                  onretryload={retryLoad}
                />
              {/each}
            </div>
          {:else}
            <DiffRowView
              r={seg.items[0].r}
              i={seg.items[0].i}
              {repoId}
              {wip}
              {applying}
              {hl}
              {expandedLines}
              loadedFiles={vs.loaded}
              {rootEl}
              {measure}
              {langOf}
              {isSelected}
              {selLabel}
              ontogglefile={toggleCollapsed}
              onfiletools={fileToolsMenu}
              onapplyhunk={(file, hi, hunk, op) => void applyHunk(file, hi, hunk, op)}
              onselectline={selectLine}
              onshowmore={showMore}
              onloadanyway={loadAnyway}
              onretryload={retryLoad}
            />
          {/if}
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .diff-root {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
  }
  .diff {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex: 1;
    min-inline-size: 0;
  }
  .diff-empty {
    padding: 24px;
    text-align: center;
  }
  .diff-body {
    position: relative;
    min-inline-size: 0;
  }
  .dtable {
    border-inline: 1px solid var(--border);
    background: var(--surface);
    font-size: var(--fs-xs);
    line-height: 1.55;
  }
  @media (max-width: 1024px) {
    .diff-body {
      max-inline-size: 100%;
    }
    .diff {
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
      overscroll-behavior: contain;
      min-inline-size: 0;
      max-inline-size: 100%;
    }
    .diff-root {
      min-inline-size: 0;
      max-inline-size: 100%;
    }
    .dtable {
      font-size: var(--fs-s);
    }
  }
</style>
