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
  function splitRows(lines: DiffLine[]): SplitRow[] {
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
</script>

<div class="diff-root" class:with-nav={showNav && prMode}>
  <!-- File Navigator Sidebar -->
  {#if showNav && prMode}
    <aside class="diff-nav" class:nav-collapsed={navCollapsed}>
      <div class="nav-header">
        {#if !navCollapsed}
          <span class="nav-title">
            {totalFiles} file{totalFiles === 1 ? '' : 's'}
            {#if viewedCount > 0}
              <span class="nav-viewed-count">· {viewedCount}/{totalFiles} viewed</span>
            {/if}
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
      <div class="segmented">
        <button class:active={mode === 'unified'} onclick={() => (mode = 'unified')}>Unified</button>
        <button class:active={mode === 'split'} onclick={() => (mode = 'split')}>Side by side</button>
      </div>
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

              {#if mode === 'unified'}
                <table class="dtable">
                  <tbody>
                    {#each hunk.lines as line, li (li)}
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
                  </tbody>
                </table>
              {:else}
                <table class="dtable split">
                  <tbody>
                    {#each splitRows(hunk.lines) as row, ri (ri)}
                      {@const leftComments = row.left ? inlineCommentsForLine(fc.anchored, row.left) : []}
                      {@const rightComments = row.right ? inlineCommentsForLine(fc.anchored, row.right) : []}
                      {@const rowComments = leftComments.length > 0 ? leftComments : rightComments}
                      <tr>
                        <td
                          class="gut old"
                          class:commentable={prMode}
                          onclick={() => row.left && gutterClick(file.path, row.left)}
                          >{row.left?.old_line ?? ''}</td
                        >
                        <td class="code mono half {row.left ? (row.left.origin === 'del' ? 'del' : '') : 'void'}">
                          {#if row.left}{@html highlightLine(row.left.content, lang)}{/if}
                        </td>
                        <td
                          class="gut new"
                          class:commentable={prMode}
                          onclick={() => row.right && gutterClick(file.path, row.right)}
                          >{row.right?.new_line ?? ''}</td
                        >
                        <td class="code mono half {row.right ? (row.right.origin === 'add' ? 'add' : '') : 'void'}">
                          {#if row.right}{@html highlightLine(row.right.content, lang)}{/if}
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
                      {#if composer && composer.path === file.path && ((row.left && composer.oldLine === row.left.old_line && composer.newLine === row.left.new_line) || (row.right && composer.oldLine === row.right.old_line && composer.newLine === row.right.new_line))}
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
                  </tbody>
                </table>
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
    border-right: 1px solid var(--border);
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    position: sticky;
    top: 0;
    max-height: 100vh;
    overflow: hidden;
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
    right: 14px;
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
    right: 8px;
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
    padding-left: 12px;
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
    text-align: left;
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
    text-align: right;
    padding: 0 8px 0 4px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    user-select: none;
    vertical-align: top;
    border-right: 1px solid var(--border);
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
</style>
