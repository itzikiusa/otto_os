<script lang="ts">
  // Shared diff renderer (Changes / commit / PR views): unified or
  // side-by-side, per-file collapse (files >400 changed lines start
  // collapsed — only expanded files render, which keeps huge diffs cheap),
  // syntax highlight, and (PR mode) line-gutter comment affordance.
  // PR mode adds: inline comment rendering, file-navigator sidebar, search.
  import type { DiffResp, FileDiff, DiffLine, PrComment } from '../../lib/api/types';
  import { langFromPath, highlightLine, ensureHljs } from '../../lib/hl';
  import Icon from '../../lib/components/Icon.svelte';
  import CommentThread from './CommentThread.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';

  interface Props {
    diff: DiffResp;
    prMode?: boolean;
    showNav?: boolean;
    comments?: PrComment[];
    onAddComment?: (path: string, line: number, body: string) => Promise<void>;
  }
  let { diff, prMode = false, showNav = false, comments = [], onAddComment }: Props = $props();

  let mode: 'unified' | 'split' = $state('unified');
  let collapsed: Record<string, boolean> = $state({});
  let initializedFor: DiffResp | null = null;
  let hlReady = $state(false);

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
  const effMode = $derived(isMobile ? 'unified' : mode);

  // Nav sidebar state
  let navCollapsed = $state(false);
  let viewed = $state(new Set<string>());

  // Search state
  let rawSearch = $state('');
  let search = $state('');
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  function debounceSearch(val: string): void {
    if (searchTimer !== null) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      search = val.trim().toLowerCase();
    }, 200);
  }

  $effect(() => {
    debounceSearch(rawSearch);
  });

  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });

  // comment composer state (PR mode). Keyed by the full (old_line, new_line)
  // pair so a deleted row and an added row that share a displayed line number
  // (e.g. old 15 deleted + new 15 added) stay distinct — otherwise a single
  // click would open a composer on both rows. `line` is the number we post to.
  let composer: { path: string; oldLine: number | null; newLine: number | null; line: number } | null =
    $state(null);
  let composerText = $state('');
  let composerBusy = $state(false);

  function changedLines(f: FileDiff): { add: number; del: number } {
    let add = 0;
    let del = 0;
    for (const h of f.hunks) {
      for (const l of h.lines) {
        if (l.origin === 'add') add++;
        else if (l.origin === 'del') del++;
      }
    }
    return { add, del };
  }

  $effect(() => {
    if (initializedFor === diff) return;
    initializedFor = diff;
    const next: Record<string, boolean> = {};
    for (const f of diff.files) {
      const c = changedLines(f);
      next[f.path] = c.add + c.del > 400;
    }
    collapsed = next;
    composer = null;
    viewed = new Set<string>();
  });

  const totals = $derived.by(() => {
    let add = 0;
    let del = 0;
    for (const f of diff.files) {
      const c = changedLines(f);
      add += c.add;
      del += c.del;
    }
    return { add, del };
  });

  // All line numbers that are visible in a given file's hunks
  function visibleLineKeys(f: FileDiff): Set<string> {
    const s = new Set<string>();
    for (const h of f.hunks) {
      for (const l of h.lines) {
        if (l.new_line !== null) s.add(`new:${l.new_line}`);
        if (l.old_line !== null) s.add(`old:${l.old_line}`);
      }
    }
    return s;
  }

  // Comments for a specific file, split into: anchored (matched to a line) and unanchored
  function fileComments(path: string): { anchored: Map<string, PrComment[]>; unanchored: PrComment[] } {
    const anchored = new Map<string, PrComment[]>();
    const unanchored: PrComment[] = [];
    if (!prMode) return { anchored, unanchored };
    for (const c of comments) {
      if (c.path !== path) continue;
      if (c.line === null) {
        unanchored.push(c);
      } else {
        // Prefer new_line key; we store under both new_line and old_line so the
        // diff-row lookup is fast via a single key.
        const key = `line:${c.line}`;
        if (!anchored.has(key)) anchored.set(key, []);
        anchored.get(key)!.push(c);
      }
    }
    return { anchored, unanchored };
  }

  // Comments for a specific diff line — match line.new_line (preferred) then line.old_line
  function inlineCommentsForLine(
    anchored: Map<string, PrComment[]>,
    line: DiffLine
  ): PrComment[] {
    if (line.new_line !== null) {
      const r = anchored.get(`line:${line.new_line}`);
      if (r && r.length > 0) return r;
    }
    if (line.old_line !== null) {
      const r = anchored.get(`line:${line.old_line}`);
      if (r && r.length > 0) return r;
    }
    return [];
  }

  // Count total inline comments for a file
  function commentCountForFile(path: string): number {
    if (!prMode) return 0;
    return comments.filter((c) => c.path === path).length;
  }

  function gutterClick(path: string, line: DiffLine): void {
    if (!prMode || !onAddComment) return;
    const n = line.new_line ?? line.old_line;
    if (n === null) return;
    const same =
      composer?.path === path &&
      composer.oldLine === line.old_line &&
      composer.newLine === line.new_line;
    composer = same ? null : { path, oldLine: line.old_line, newLine: line.new_line, line: n };
    composerText = '';
  }

  async function submitComment(): Promise<void> {
    if (!composer || !onAddComment || composerText.trim() === '') return;
    composerBusy = true;
    try {
      await onAddComment(composer.path, composer.line, composerText.trim());
      composer = null;
      composerText = '';
    } finally {
      composerBusy = false;
    }
  }

  // side-by-side row pairing: context aligns, del-runs pair with add-runs
  interface SplitRow {
    left: DiffLine | null;
    right: DiffLine | null;
  }
  function computeSplitRows(lines: DiffLine[]): SplitRow[] {
    const rows: SplitRow[] = [];
    let i = 0;
    while (i < lines.length) {
      const l = lines[i];
      if (l.origin === 'context') {
        rows.push({ left: l, right: l });
        i++;
        continue;
      }
      const dels: DiffLine[] = [];
      const adds: DiffLine[] = [];
      while (i < lines.length && lines[i].origin === 'del') dels.push(lines[i++]);
      while (i < lines.length && lines[i].origin === 'add') adds.push(lines[i++]);
      const n = Math.max(dels.length, adds.length);
      for (let k = 0; k < n; k++) {
        rows.push({ left: dels[k] ?? null, right: adds[k] ?? null });
      }
      if (dels.length === 0 && adds.length === 0) i++; // safety
    }
    return rows;
  }

  // Memoised side-by-side rows: keyed `{filePath}::{hunkIndex}` so the pairing
  // is only recomputed when the underlying diff data actually changes.
  const splitRowsCache = $derived.by(() => {
    const m = new Map<string, SplitRow[]>();
    for (const f of diff.files) {
      for (let hi = 0; hi < f.hunks.length; hi++) {
        m.set(`${f.path}::${hi}`, computeSplitRows(f.hunks[hi].lines));
      }
    }
    return m;
  });

  function splitRows(filePath: string, hunkIdx: number, lines: DiffLine[]): SplitRow[] {
    return splitRowsCache.get(`${filePath}::${hunkIdx}`) ?? computeSplitRows(lines);
  }

  // Line height estimate for VirtualList (mono code line, 1.55 line-height, 11.5px).
  // At zoom=1 this is ~18 px; add a little buffer for safety.
  const VLIST_ROW_H = 20;

  // Large-hunk threshold: hunks with more lines than this use VirtualList
  // instead of a full DOM table; below it the table renders in full (fast for
  // small hunks and required to support comments/composer rows).
  const VLIST_THRESHOLD = 200;

  // Search helpers
  function fileMatchesSearch(f: FileDiff): boolean {
    if (!search) return true;
    if (f.path.toLowerCase().includes(search)) return true;
    // Check changed-line content
    for (const h of f.hunks) {
      for (const l of h.lines) {
        if (l.origin !== 'context' && l.content.toLowerCase().includes(search)) return true;
      }
    }
    return false;
  }

  const filteredFiles = $derived.by(() => {
    if (!search) return diff.files;
    return diff.files.filter(fileMatchesSearch);
  });

  const matchCount = $derived(filteredFiles.length);

  // Nav: basename + directory parts
  function baseName(path: string): string {
    return path.split('/').pop() ?? path;
  }
  function dirName(path: string): string {
    const parts = path.split('/');
    if (parts.length <= 1) return '';
    return parts.slice(0, -1).join('/') + '/';
  }

  function scrollToFile(path: string): void {
    // Expand first so the target has full height, then scroll on the next frame.
    // NOTE: getElementById takes a LITERAL id — do NOT CSS.escape it (the id
    // attribute is the raw path).
    if (collapsed[path]) {
      collapsed = { ...collapsed, [path]: false };
    }
    const find = () => document.getElementById(`dfile-${path}`);
    requestAnimationFrame(() => {
      const el = find();
      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
    });
  }

  function toggleViewed(path: string): void {
    const next = new Set(viewed);
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    viewed = next;
  }

  const viewedCount = $derived(viewed.size);
  const totalFiles = $derived(diff.files.length);

  // --- Keyboard file navigation: ] / n = next file, [ / N = prev file --------
  // Tracks which file in the filtered list is the "keyboard cursor" so ][ nav
  // works predictably when search is active.
  let navFocusIdx = $state(-1);

  function navToFile(delta: number): void {
    const files = filteredFiles;
    if (files.length === 0) return;
    const next = Math.max(0, Math.min(files.length - 1, navFocusIdx + delta));
    navFocusIdx = next;
    scrollToFile(files[next].path);
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

  // --- Per-hunk line cap: render first N lines; "Show more" expands ------------
  // Keeps large hunks from pushing every line into the DOM at once.
  const HUNK_LINE_CAP = 500;
  let expandedHunks: Set<string> = $state(new Set());

  function hunkKey(filePath: string, hunkIdx: number): string {
    return `${filePath}::${hunkIdx}`;
  }

  function isHunkExpanded(filePath: string, hunkIdx: number): boolean {
    return expandedHunks.has(hunkKey(filePath, hunkIdx));
  }

  function expandHunk(filePath: string, hunkIdx: number): void {
    const next = new Set(expandedHunks);
    next.add(hunkKey(filePath, hunkIdx));
    expandedHunks = next;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="diff-root"
  class:with-nav={showNav && prMode}
  onkeydown={onDiffKeydown}
  role="region"
  aria-label="Diff viewer"
  tabindex="-1"
>
  <!-- File Navigator Sidebar -->
  {#if showNav && prMode}
    <aside class="diff-nav" class:nav-collapsed={navCollapsed}>
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
          <Icon name={navCollapsed ? 'chevronRight' : 'chevronLeft'} size={12} />
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
          {#each diff.files as file (file.path)}
            {@const stats = changedLines(file)}
            {@const cCount = commentCountForFile(file.path)}
            {@const isViewed = viewed.has(file.path)}
            {@const matches = fileMatchesSearch(file)}
            <div
              class="nav-file"
              class:nav-file-viewed={isViewed}
              class:nav-file-hidden={!matches}
              role="button"
              tabindex="0"
              onclick={() => scrollToFile(file.path)}
              onkeydown={(e) => e.key === 'Enter' && scrollToFile(file.path)}
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
                {#if dirName(file.path)}
                  <span class="nav-dir">{dirName(file.path)}</span>
                {/if}
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
          {/each}
        </div>
      {/if}
    </aside>
  {/if}

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
      {#if !isMobile}
        <!-- Side-by-side is desktop-only; ≤1024 always renders unified. -->
        <div class="segmented">
          <button class:active={mode === 'unified'} onclick={() => (mode = 'unified')}>Unified</button>
          <button class:active={mode === 'split'} onclick={() => (mode = 'split')}>Side by side</button>
        </div>
      {/if}
    </div>

    {#each filteredFiles as file (file.path)}
      {@const stats = changedLines(file)}
      {@const lang = hlReady ? langFromPath(file.path) : null}
      {@const fc = fileComments(file.path)}
      {@const cCount = commentCountForFile(file.path)}
      <section class="dfile" id="dfile-{file.path}">
        <button
          class="dfile-head"
          onclick={() => (collapsed = { ...collapsed, [file.path]: !collapsed[file.path] })}
        >
          <Icon name={collapsed[file.path] ? 'chevronRight' : 'chevronDown'} size={11} />
          <span class="dfile-path mono">
            {#if file.old_path}{file.old_path} → {/if}{file.path}
          </span>
          <span class="grow"></span>
          {#if prMode && cCount > 0}
            <span class="file-comment-badge" title="{cCount} comment{cCount === 1 ? '' : 's'}">
              💬 {cCount}
            </span>
          {/if}
          <span class="add">+{stats.add}</span>
          <span class="del">−{stats.del}</span>
        </button>

        {#if !collapsed[file.path]}
          {#if file.is_binary}
            <div class="dfile-binary dim">Binary file — no text diff.</div>
          {:else}
            <!-- File-level comments (no line anchor, or line not in diff) -->
            {#if prMode && fc.unanchored.length > 0}
              <div class="file-comments-block">
                <div class="file-comments-label dim">File comments</div>
                {#each fc.unanchored as c (c.id)}
                  <CommentThread comment={c} />
                {/each}
              </div>
            {/if}

            {#each file.hunks as hunk, hi (hi)}
              <div class="hunk-header mono">{hunk.header}</div>

              {#if effMode === 'unified'}
                {#if hunk.lines.length > VLIST_THRESHOLD && !isHunkExpanded(file.path, hi)}
                  <!-- Large hunk: virtualised rendering for smooth scroll.
                       Comments and the composer are deliberately suppressed here —
                       the "Load anyway" guard below lets the user fall back to the
                       full table when they need to comment. -->
                  <VirtualList
                    items={hunk.lines}
                    estimateHeight={VLIST_ROW_H}
                    class="vlist-hunk"
                  >
                    {#snippet row(line: DiffLine, _i: number)}
                      <div class="vrow dline {line.origin}">
                        <span class="gut old">{line.old_line ?? ''}</span>
                        <span class="gut new">{line.new_line ?? ''}</span>
                        <span class="sign">{line.origin === 'add' ? '+' : line.origin === 'del' ? '−' : ''}</span>
                        <span class="code mono">{@html highlightLine(line.content, lang)}</span>
                      </div>
                    {/snippet}
                  </VirtualList>
                  <div class="hunk-cap-cell">
                    <button
                      class="btn small ghost hunk-cap-btn"
                      onclick={() => expandHunk(file.path, hi)}
                    >
                      Load all {hunk.lines.length} lines (comments + composer available after loading)
                    </button>
                  </div>
                {:else}
                  {@const hunkCapped = !isHunkExpanded(file.path, hi) && hunk.lines.length > HUNK_LINE_CAP}
                  {@const visibleLines = hunkCapped ? hunk.lines.slice(0, HUNK_LINE_CAP) : hunk.lines}
                  <table class="dtable">
                    <tbody>
                      {#each visibleLines as line, li (li)}
                        <tr class="dline {line.origin}">
                          <td
                            class="gut old"
                            class:commentable={prMode}
                            onclick={() => gutterClick(file.path, line)}
                            >{line.old_line ?? ''}</td
                          >
                          <td
                            class="gut new"
                            class:commentable={prMode}
                            onclick={() => gutterClick(file.path, line)}
                            >{line.new_line ?? ''}</td
                          >
                          <td class="sign">{line.origin === 'add' ? '+' : line.origin === 'del' ? '−' : ''}</td>
                          <td class="code mono">{@html highlightLine(line.content, lang)}</td>
                        </tr>
                        {#each inlineCommentsForLine(fc.anchored, line) as c (c.id)}
                          <tr class="comment-row">
                            <td colspan="4"><CommentThread comment={c} /></td>
                          </tr>
                        {/each}
                        {#if composer && composer.path === file.path && composer.oldLine === line.old_line && composer.newLine === line.new_line}
                          <tr class="comment-row">
                            <td colspan="4">
                              <div class="composer">
                                <textarea
                                  class="input"
                                  rows="2"
                                  bind:value={composerText}
                                  placeholder="Comment on line {composer.line}…"
                                ></textarea>
                                <div class="composer-actions">
                                  <button class="btn small" onclick={() => (composer = null)}>Cancel</button>
                                  <button
                                    class="btn small primary"
                                    disabled={composerBusy || composerText.trim() === ''}
                                    onclick={submitComment}
                                  >
                                    {composerBusy ? 'Posting…' : 'Comment'}
                                  </button>
                                </div>
                              </div>
                            </td>
                          </tr>
                        {/if}
                      {/each}
                      {#if hunkCapped}
                        <tr class="hunk-cap-row">
                          <td colspan="4" class="hunk-cap-cell">
                            <button
                              class="btn small ghost hunk-cap-btn"
                              onclick={() => expandHunk(file.path, hi)}
                            >
                              Show {hunk.lines.length - HUNK_LINE_CAP} more lines
                            </button>
                          </td>
                        </tr>
                      {/if}
                    </tbody>
                  </table>
                {/if}
              {:else}
                {#if hunk.lines.length > VLIST_THRESHOLD && !isHunkExpanded(file.path, hi)}
                  <!-- Large split-hunk: virtualised. Comments suppressed (same guard above). -->
                  {@const srows = splitRows(file.path, hi, hunk.lines)}
                  <VirtualList
                    items={srows}
                    estimateHeight={VLIST_ROW_H}
                    class="vlist-hunk"
                  >
                    {#snippet row(sr: SplitRow, _i: number)}
                      <div class="vrow split-vrow">
                        <span class="gut old">{sr.left?.old_line ?? ''}</span>
                        <span class="code mono half {sr.left ? (sr.left.origin === 'del' ? 'del' : '') : 'void'}">{@html sr.left ? highlightLine(sr.left.content, lang) : ''}</span>
                        <span class="gut new">{sr.right?.new_line ?? ''}</span>
                        <span class="code mono half {sr.right ? (sr.right.origin === 'add' ? 'add' : '') : 'void'}">{@html sr.right ? highlightLine(sr.right.content, lang) : ''}</span>
                      </div>
                    {/snippet}
                  </VirtualList>
                  <div class="hunk-cap-cell">
                    <button
                      class="btn small ghost hunk-cap-btn"
                      onclick={() => expandHunk(file.path, hi)}
                    >
                      Load all {hunk.lines.length} lines (comments + composer available after loading)
                    </button>
                  </div>
                {:else}
                  {@const hunkSplitCapped = !isHunkExpanded(file.path, hi) && hunk.lines.length > HUNK_LINE_CAP}
                  {@const splitLines = hunkSplitCapped ? hunk.lines.slice(0, HUNK_LINE_CAP) : hunk.lines}
                  <table class="dtable split">
                    <tbody>
                      {#each splitRows(file.path, hi, splitLines) as srow, ri (ri)}
                        {@const leftComments = srow.left ? inlineCommentsForLine(fc.anchored, srow.left) : []}
                        {@const rightComments = srow.right ? inlineCommentsForLine(fc.anchored, srow.right) : []}
                        {@const rowComments = leftComments.length > 0 ? leftComments : rightComments}
                        <tr>
                          <td
                            class="gut old"
                            class:commentable={prMode}
                            onclick={() => srow.left && gutterClick(file.path, srow.left)}
                            >{srow.left?.old_line ?? ''}</td
                          >
                          <td class="code mono half {srow.left ? (srow.left.origin === 'del' ? 'del' : '') : 'void'}">
                            {#if srow.left}{@html highlightLine(srow.left.content, lang)}{/if}
                          </td>
                          <td
                            class="gut new"
                            class:commentable={prMode}
                            onclick={() => srow.right && gutterClick(file.path, srow.right)}
                            >{srow.right?.new_line ?? ''}</td
                          >
                          <td class="code mono half {srow.right ? (srow.right.origin === 'add' ? 'add' : '') : 'void'}">
                            {#if srow.right}{@html highlightLine(srow.right.content, lang)}{/if}
                          </td>
                        </tr>
                        {#if rowComments.length > 0}
                          <tr class="comment-row">
                            <td colspan="4">
                              {#each rowComments as c (c.id)}
                                <CommentThread comment={c} />
                              {/each}
                            </td>
                          </tr>
                        {/if}
                        {#if composer && composer.path === file.path && ((srow.left && composer.oldLine === srow.left.old_line && composer.newLine === srow.left.new_line) || (srow.right && composer.oldLine === srow.right.old_line && composer.newLine === srow.right.new_line))}
                          <tr class="comment-row">
                            <td colspan="4">
                              <div class="composer">
                                <textarea
                                  class="input"
                                  rows="2"
                                  bind:value={composerText}
                                  placeholder="Comment on line {composer.line}…"
                                ></textarea>
                                <div class="composer-actions">
                                  <button class="btn small" onclick={() => (composer = null)}>Cancel</button>
                                  <button
                                    class="btn small primary"
                                    disabled={composerBusy || composerText.trim() === ''}
                                    onclick={submitComment}
                                  >
                                    {composerBusy ? 'Posting…' : 'Comment'}
                                  </button>
                                </div>
                              </div>
                            </td>
                          </tr>
                        {/if}
                      {/each}
                      {#if hunkSplitCapped}
                        <tr class="hunk-cap-row">
                          <td colspan="4" class="hunk-cap-cell">
                            <button
                              class="btn small ghost hunk-cap-btn"
                              onclick={() => expandHunk(file.path, hi)}
                            >
                              Show {hunk.lines.length - HUNK_LINE_CAP} more lines
                            </button>
                          </td>
                        </tr>
                      {/if}
                    </tbody>
                  </table>
                {/if}
              {/if}
            {/each}
          {/if}
        {/if}
      </section>
    {:else}
      <div class="dim" style="padding: 24px; text-align: center">
        {search ? 'No files match your search.' : 'No changes.'}
      </div>
    {/each}
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
    width: 240px;
    min-width: 240px;
    max-width: 240px;
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
    padding: 8px 8px 6px 10px;
    border-bottom: 1px solid var(--border);
    gap: 6px;
    flex-shrink: 0;
  }
  .nav-title {
    font-size: 11px;
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
    border-radius: var(--radius-s, 3px);
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
    font-size: 11.5px;
    height: 26px;
    padding: 0 6px;
    box-sizing: border-box;
  }
  .nav-match-count {
    position: absolute;
    inset-inline-end: 14px;
    top: 50%;
    transform: translateY(-50%);
    font-size: 10px;
    color: var(--text-dim);
    pointer-events: none;
  }
  .nav-files {
    overflow-y: auto;
    flex: 1;
    padding: 2px 0 8px;
  }
  .nav-file {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 8px 4px 6px;
    cursor: pointer;
    font-size: 11px;
    color: var(--text);
    border-radius: 0;
    transition: background 80ms;
    min-width: 0;
  }
  .nav-file:hover {
    background: var(--hover);
  }
  .nav-file.nav-file-viewed {
    opacity: 0.45;
  }
  .nav-file.nav-file-hidden {
    display: none;
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
  .nav-dir {
    color: var(--text-dim);
    font-size: 10px;
  }
  .nav-base {
    font-weight: 600;
    font-size: 11px;
  }
  .nav-file-stats {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
    font-size: 10px;
  }
  .nav-comment-badge {
    flex-shrink: 0;
    font-size: 9.5px;
    color: var(--accent);
    white-space: nowrap;
  }

  /* ── Toolbar search (no-nav mode) ── */
  .toolbar-search-wrap {
    position: relative;
    display: flex;
    align-items: center;
  }
  .toolbar-search {
    font-size: 11.5px;
    height: 26px;
    padding: 0 8px;
    width: 180px;
  }
  .toolbar-match-count {
    position: absolute;
    inset-inline-end: 8px;
    font-size: 10px;
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
    font-size: 12px;
    color: var(--text-dim);
    display: inline-flex;
    gap: 8px;
    align-items: center;
  }
  .add {
    color: var(--status-working);
    font-weight: 600;
    font-size: 11.5px;
  }
  .del {
    color: var(--status-exited);
    font-weight: 600;
    font-size: 11.5px;
  }
  .dfile {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface);
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
    font-size: 12px;
    color: var(--text);
    text-align: start;
  }
  .dfile-path {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dfile-binary {
    padding: 14px;
    font-size: 12px;
  }
  .file-comment-badge {
    font-size: 10.5px;
    color: var(--accent);
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* File-level unanchored comments */
  .file-comments-block {
    padding: 8px 14px 10px;
    background: color-mix(in srgb, var(--accent) 5%, var(--bg));
    border-bottom: 1px solid var(--border);
  }
  .file-comments-label {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 4px;
  }

  .hunk-header {
    position: sticky;
    top: 0;
    padding: 3px 12px;
    font-size: 10.5px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .dtable {
    width: 100%;
    border-collapse: collapse;
    font-size: 11.5px;
    line-height: 1.55;
  }
  .gut {
    width: 42px;
    min-width: 42px;
    text-align: end;
    padding: 0 8px 0 4px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    user-select: none;
    vertical-align: top;
    border-inline-end: 1px solid var(--border);
  }
  .gut.commentable {
    cursor: pointer;
  }
  .gut.commentable:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--accent);
  }
  .sign {
    width: 16px;
    text-align: center;
    color: var(--text-dim);
    user-select: none;
    font-family: var(--font-mono);
  }
  .code {
    padding: 0 10px 0 4px;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  tr.dline.add {
    background: color-mix(in srgb, var(--status-working) 11%, transparent);
  }
  tr.dline.del {
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
  }
  .code.half.add {
    background: color-mix(in srgb, var(--status-working) 11%, transparent);
  }
  .code.half.del {
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
  }
  .code.half.void {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
  .comment-row td {
    padding: 6px 12px;
    background: var(--bg);
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
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
  @media (max-width: 700px) {
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
    .dfile {
      max-width: 100%;
    }
    /* The diff is its own vertical scroll container on mobile (E2E invariant):
       hosts (PR/review/history) embed DiffViewer without always wrapping it in a
       scroll pane, so own it here. min-width:0 lets the flex child shrink so the
       wrapping .code below actually fits the viewport instead of pushing wider. */
    .diff {
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
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
    .nav-title { font-size: 13px; }
    .nav-file { font-size: 13px; padding: 8px; }
    .nav-base { font-size: 13px; }
    .nav-dir { font-size: 11px; }
    .nav-file-stats { font-size: 12px; }
    .nav-search { font-size: 13px; height: 32px; }

    .diff-toolbar { flex-wrap: wrap; gap: 8px; }
    .toolbar-search-wrap { flex: 1; min-width: 120px; }
    .diff-stats { font-size: 13px; }
    .add, .del { font-size: 13px; }
    .toolbar-search { font-size: 13px; height: 32px; width: 100%; }
    .dfile-head { font-size: 13px; padding: 9px 12px; min-height: 40px; }
    .dfile-path { font-size: 13px; }
    .hunk-header { font-size: 12px; padding: 4px 12px; }
    /* table-layout:fixed pins the gutter+sign columns so the code column can't
       be stretched past the viewport by a single long token — it wraps within
       its remaining width (E2E seeds 140-char lines that must wrap, not clip). */
    .dtable { font-size: 12.5px; table-layout: fixed; width: 100%; }
    .gut {
      font-size: 11px;
      width: 30px;
      min-width: 30px;
      padding: 0 5px 0 3px;
    }
    .sign { width: 13px; }
    /* Wrap code so long lines don't push the page wider than the screen.
       overflow-wrap:anywhere is the belt-and-suspenders for unbreakable tokens. */
    .code {
      font-size: 12.5px;
      white-space: pre-wrap;
      word-break: break-word;
      overflow-wrap: anywhere;
    }
    .composer { max-width: 100%; }
    /* Virtualised unified rows: match the table's narrower mobile gutters and
       let the code cell wrap. (Split-vrow is never reached on mobile — effMode
       forces unified — but stays aligned for safety.) */
    .vrow { grid-template-columns: 30px 30px 13px 1fr; }
    .vrow .code { white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere; }
  }

  /* Hunk line cap: "Show N more lines" affordance */
  .hunk-cap-row {
    background: var(--surface);
  }
  .hunk-cap-cell {
    text-align: center;
    padding: 5px 8px;
    border-top: 1px dashed var(--border);
  }
  .hunk-cap-btn {
    font-size: 11px;
    color: var(--text-dim);
  }
  .hunk-cap-btn:hover {
    color: var(--text);
  }

  /* ── VirtualList hunk container ─────────────────────────────────────────
     Capped at 400 px so the viewport doesn't snap to a giant empty box; the
     user scrolls inside it. The vrow divs mirror the .dtable tr layout using
     a fixed-column grid (gutter+gutter+sign+code) so lines align correctly. */
  :global(.vlist-hunk) {
    max-height: 400px;
    border-top: 1px solid var(--border);
    font-size: 11.5px;
    line-height: 1.55;
  }
  .vrow {
    display: grid;
    /* gut-old | gut-new | sign | code — mirrors the four table columns */
    grid-template-columns: 42px 42px 16px 1fr;
    align-items: start;
    min-height: 20px;
  }
  .vrow .gut {
    /* override the td width rule from the table layout — already set inline */
    display: block;
  }
  .vrow.dline.add {
    background: color-mix(in srgb, var(--status-working) 11%, transparent);
  }
  .vrow.dline.del {
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
  }
  /* Split side-by-side rows in VirtualList: 4-col grid matching split table */
  .split-vrow {
    grid-template-columns: 42px 1fr 42px 1fr;
  }
  .split-vrow .code.half.add {
    background: color-mix(in srgb, var(--status-working) 11%, transparent);
  }
  .split-vrow .code.half.del {
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
  }
  .split-vrow .code.half.void {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
</style>
