<script lang="ts">
  // FileTree: lazy browsable file tree + read-only syntax-highlighted viewer
  // for the Side panel's Files tab. With `navigation` it also gets the folder
  // picker's navigation (lib/components/folderNavigation.ts): Back / Forward /
  // Up, clickable breadcrumbs, a typed "go to" path, a Show hidden toggle, and
  // an "open this folder" action on every folder row.
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { onDestroy, tick, untrack } from 'svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { openFile as openFileSignal } from '../../lib/stores/openfile.svelte';
  import { api } from '../../lib/api/client';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { emptyHistory, folderCrumbs, historyTarget, parentFolder, resolveJump, settleFolder } from '../../lib/components/folderNavigation';
  import { lsGet, lsSet } from '../../lib/storage';
  import { ListWindow } from '../git/list-window.svelte';
  import { marked } from 'marked';
  import type { FsBrowse, FsEntry, FsRead } from '../../lib/api/types';

  interface Props {
    /** When provided, the tree is rooted here; otherwise falls back to
     *  ws.activeSession?.cwd or ws.current?.root_path. */
    root?: string;
    /** When provided, a ✕ button is shown in the header to close this section. */
    onClose?: () => void;
    /** When true, this section responds to programmatic open-file requests
     *  (terminal `file:line` clicks). Only one section should be primary. */
    primary?: boolean;
    /** Folder navigation bar (history, breadcrumbs, go-to path, hidden). */
    navigation?: boolean;
  }
  let { root = undefined, onClose = undefined, primary = false, navigation = false }: Props = $props();

  // ── tree state ────────────────────────────────────────────────────────────

  interface TreeNode {
    entry: FsEntry;
    depth: number;
    open: boolean;
    loaded: boolean;
    loading: boolean;
    children: TreeNode[];
  }

  // The effective root: prop takes precedence, then session cwd, then workspace root.
  const wsRoot = $derived(ws.activeSession?.cwd || ws.current?.root_path || '');
  // overriddenRoot holds folder-picker selections inside this component.
  let overriddenRoot = $state<string | undefined>(undefined);

  const effectiveRoot = $derived(overriddenRoot ?? root ?? wsRoot);

  // Top-level nodes (children of effectiveRoot).
  let rootNodes: TreeNode[] = $state([]);
  let rootLoading = $state(false);
  let rootError = $state('');
  let loadedRoot = $state('');
  let rootGeneration = 0;
  let rootRequest: AbortController | undefined;
  let fileGeneration = 0;
  let fileRequest: AbortController | undefined;

  // ── navigation state ──────────────────────────────────────────────────────
  // The folder actually listed (canonical, from the daemon) + its parent, the
  // Back/Forward history of listed folders, and the history entry a Back /
  // Forward step is walking to (settled when its listing lands).
  let listedPath = $state('');
  let listedParent = $state<string | null>(null);
  let history = $state(emptyHistory());
  let walkIndex: number | undefined;
  const backIndex = $derived(historyTarget(history, -1));
  const forwardIndex = $derived(historyTarget(history, 1));
  // Dotfiles are shown by default (the tree always listed them); the toggle
  // is remembered per device.
  const HIDDEN_KEY = 'otto_files_show_hidden';
  let showHidden = $state(lsGet(HIDDEN_KEY) !== '0');
  function toggleHidden(): void {
    showHidden = !showHidden;
    lsSet(HIDDEN_KEY, showHidden ? '1' : '0');
  }
  let editingPath = $state(false);
  let pathDraft = $state('');
  let pathInput = $state<HTMLInputElement | null>(null);
  let crumbsEl = $state<HTMLElement | null>(null);

  /** Show `path` as the tree's root (`index`: a Back/Forward step). */
  function navigate(path: string, index?: number): void {
    if (path === loadedRoot && !rootError) {
      // Already listed: only the history position moves.
      if (index !== undefined) history = { ...history, index };
      return;
    }
    walkIndex = index;
    overriddenRoot = path;
    if (path === loadedRoot) void loadRoot(path); // a failed listing: retry it
  }
  /** Follow the section's own folder again (the session's / the panel's). */
  function resetRoot(): void {
    walkIndex = undefined;
    overriddenRoot = undefined;
  }
  // The folder this section belongs to changed (another session focused, a
  // folder picked in the Side panel's folder bar, another workspace): follow
  // it — a folder browsed to under the OLD one would leave the tree and the
  // folder bar (and Git) disagreeing — and start a fresh Back/Forward history.
  let followedRoot = untrack(() => root);
  $effect(() => {
    const r = root;
    untrack(() => {
      if (r === followedRoot) return;
      followedRoot = r;
      resetRoot();
      history = emptyHistory();
    });
  });
  function walk(index: number | null): void {
    if (index !== null) navigate(history.paths[index], index);
  }
  async function startPathEdit(): Promise<void> {
    pathDraft = listedPath || effectiveRoot;
    editingPath = true;
    await tick();
    pathInput?.focus();
    pathInput?.select();
  }
  function commitPath(): void {
    const target = resolveJump(pathDraft, listedPath || effectiveRoot);
    editingPath = false;
    if (target) navigate(target);
  }
  function onPathKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitPath();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      editingPath = false;
    }
  }

  // ── viewer state ──────────────────────────────────────────────────────────

  let viewerFile: FsRead | null = $state(null);
  let viewerName = $state('');
  let viewerLoading = $state(false);
  let viewerError = $state('');
  // Line/col to reveal when the viewer opens from a terminal `file:line` click.
  let viewerGotoLine: number | null = $state(null);
  let viewerGotoCol: number | null = $state(null);

  // ── folder picker state ───────────────────────────────────────────────────

  let showPicker = $state(false);

  // ── effects ───────────────────────────────────────────────────────────────

  $effect(() => {
    const r = effectiveRoot;
    if (r && r !== loadedRoot) {
      // Reset tree state when root changes.
      rootNodes = [];
      void loadRoot(r);
    } else if (!r) {
      rootGeneration++; rootRequest?.abort();
      loadedRoot = ''; rootNodes = []; rootLoading = false; rootError = '';
    }
  });

  // Programmatic open: react to terminal `file:line` clicks. Only the primary
  // section handles them so a single, predictable viewer responds. The `n`
  // counter on the request makes each click fire exactly once.
  let lastOpenN = 0;
  $effect(() => {
    if (!primary) return;
    const req = openFileSignal.request;
    if (!req || req.n <= lastOpenN) return;
    lastOpenN = req.n;
    void openPath(req.path, req.line, req.col);
  });

  // ── helpers ───────────────────────────────────────────────────────────────

  async function loadRoot(r: string): Promise<void> {
    const generation = ++rootGeneration;
    rootRequest?.abort();
    const controller = new AbortController();
    rootRequest = controller;
    loadedRoot = r;
    rootLoading = true;
    rootError = '';
    const index = walkIndex;
    walkIndex = undefined;
    try {
      const data = await api.get<FsBrowse>(`/fs/browse?path=${encodeURIComponent(r)}&files=true`, controller.signal);
      if (generation !== rootGeneration) return;
      rootNodes = data.entries.map((e) => makeNode(e, 0));
      listedPath = data.path;
      listedParent = data.parent;
      history = settleFolder(history, data.path, index);
      await tick();
      if (generation === rootGeneration && crumbsEl) crumbsEl.scrollLeft = crumbsEl.scrollWidth;
    } catch (e) {
      if (generation === rootGeneration) rootError = loadErrorText(e);
    } finally {
      if (generation === rootGeneration) rootLoading = false;
    }
  }

  function makeNode(entry: FsEntry, depth: number): TreeNode {
    return { entry, depth, open: false, loaded: false, loading: false, children: [] };
  }

  async function toggleDir(node: TreeNode): Promise<void> {
    if (!node.entry.is_dir) return;
    const generation = rootGeneration;
    if (!node.open) {
      // Open: lazy-load children if not yet done.
      node.open = true;
      if (!node.loaded) {
        node.loading = true;
        try {
          const data = await api.get<FsBrowse>(
            `/fs/browse?path=${encodeURIComponent(node.entry.path)}&files=true`,
          );
          if (generation !== rootGeneration) return;
          node.children = data.entries.map((e) => makeNode(e, node.depth + 1));
          node.loaded = true;
        } catch (e) {
          if (generation !== rootGeneration) return;
          toastError(`Couldn’t open ${node.entry.name}`, e);
          node.open = false;
        } finally {
          node.loading = false;
        }
      }
    } else {
      node.open = false;
    }
    // Trigger reactivity by reassigning rootNodes.
    if (generation === rootGeneration) rootNodes = [...rootNodes];
  }

  function onTreeRowKey(e: KeyboardEvent, node: TreeNode): void {
    const btn = e.currentTarget as HTMLElement;
    const list = btn.closest('.tree-list');
    if (e.key === 'ArrowRight' && node.entry.is_dir && !node.open) {
      e.preventDefault();
      void toggleDir(node);
    } else if (e.key === 'ArrowLeft' && node.entry.is_dir && node.open) {
      e.preventDefault();
      void toggleDir(node);
    } else if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && list) {
      e.preventDefault();
      const rows = Array.from(list.querySelectorAll<HTMLButtonElement>('button.tree-row'));
      const i = rows.indexOf(btn as HTMLButtonElement);
      const next = rows[i + (e.key === 'ArrowDown' ? 1 : -1)];
      next?.focus();
    }
  }

  async function openFile(entry: FsEntry): Promise<void> {
    await readFile(entry.path, undefined, undefined, true, false);
  }

  const folderOf = (path: string): string => parentFolder(path) ?? '/';

  async function openPath(path: string, line?: number, col?: number): Promise<void> {
    await readFile(path, line, col, false, true);
  }

  async function readFile(path: string, line: number | undefined, col: number | undefined, preview: boolean, revealParent: boolean): Promise<void> {
    const generation = ++fileGeneration;
    fileRequest?.abort();
    const controller = new AbortController();
    fileRequest = controller;
    // Browsing location is independent from workspace/session identity. Listing
    // the parent is useful but never a prerequisite to showing a readable file.
    if (revealParent) overriddenRoot = folderOf(path);
    viewerGotoLine = line ?? null;
    viewerGotoCol = col ?? null;
    viewerLoading = true;
    viewerName = path.split('/').at(-1) ?? path;
    viewerFile = null;
    viewerError = '';
    previewMode = preview;
    try {
      const data = await api.get<FsRead>(`/fs/read?path=${encodeURIComponent(path)}`, controller.signal);
      if (generation !== fileGeneration) return;
      viewerFile = data;
      if (revealParent) overriddenRoot = folderOf(data.path);
    } catch (e) {
      if (generation !== fileGeneration) return;
      viewerError = `Cannot open ${path}: ${loadErrorText(e)}. If the terminal shortened the filename, use its full path.`;
    } finally {
      if (generation === fileGeneration) viewerLoading = false;
    }
  }

  function closeViewer(): void {
    fileGeneration++;
    fileRequest?.abort();
    viewerFile = null;
    viewerError = '';
    viewerLoading = false;
    viewerName = '';
    viewerGotoLine = null;
    viewerGotoCol = null;
  }

  onDestroy(() => {
    rootGeneration++; fileGeneration++;
    rootRequest?.abort(); fileRequest?.abort();
  });

  // ── Markdown / HTML preview ────────────────────────────────────────────────
  let previewMode = $state(true);
  const viewerExt = $derived((viewerName.split('.').pop() ?? '').toLowerCase());
  const canPreview = $derived(['md', 'markdown', 'mdx', 'html', 'htm'].includes(viewerExt));
  const isMarkdown = $derived(['md', 'markdown', 'mdx'].includes(viewerExt));
  // Rendered HTML wrapped for a sandboxed iframe (no scripts run).
  const previewSrcdoc = $derived.by(() => {
    if (!viewerFile || !canPreview) return '';
    const inner = isMarkdown ? renderMarkdown(viewerFile.content) : viewerFile.content;
    return `<!doctype html><html><head><meta charset="utf-8"><style>${PREVIEW_CSS}</style></head><body>${inner}</body></html>`;
  });
  function renderMarkdown(src: string): string {
    try {
      return marked.parse(src, { async: false, gfm: true, breaks: true }) as string;
    } catch {
      return `<pre>${src.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c] ?? c))}</pre>`;
    }
  }
  // The srcdoc is an isolated, script-less document: it can't see the app's
  // tokens, so it gets a palette per resolved scheme (it used to be dark-only
  // — light grey text on a hard-coded near-black box in the light theme).
  const PREVIEW_PALETTE = {
    dark: { text: '#dddddd', dim: '#aaaaaa', link: '#6ea8fe', line: '#ffffff22', fill: '#ffffff12' },
    light: { text: '#1d1d1f', dim: '#6e6e73', link: '#0a60d0', line: '#00000022', fill: '#0000000d' },
  } as const;
  const PREVIEW_CSS = $derived.by(() => {
    const p = PREVIEW_PALETTE[ui.resolvedScheme];
    return `
    :root { color-scheme: ${ui.resolvedScheme}; }
    body { font: 14px/1.6 -apple-system, system-ui, sans-serif; margin: 16px; color: ${p.text}; background: transparent; }
    h1,h2,h3 { line-height: 1.25; } h1,h2 { border-bottom: 1px solid ${p.line}; padding-bottom: .2em; }
    a { color: ${p.link}; } code { background: ${p.fill}; padding: .15em .35em; border-radius: 4px; font-family: ui-monospace, monospace; }
    pre { background: ${p.fill}; padding: 12px; border-radius: 6px; overflow: auto; } pre code { background: none; padding: 0; }
    table { border-collapse: collapse; } th,td { border: 1px solid ${p.line}; padding: 4px 8px; }
    blockquote { border-inline-start: 3px solid ${p.line}; margin: 0; padding-inline-start: 12px; color: ${p.dim}; }
    img { max-width: 100%; }
  `;
  });

  // Collect flattened visible nodes for rendering (DFS walk). Dotfiles (and
  // everything under a dot-folder) drop out while Show hidden is off.
  function flatten(nodes: TreeNode[], hidden: boolean): TreeNode[] {
    const out: TreeNode[] = [];
    for (const n of nodes) {
      if (!hidden && n.entry.name.startsWith('.')) continue;
      out.push(n);
      if (n.open && n.children.length > 0) {
        out.push(...flatten(n.children, hidden));
      }
    }
    return out;
  }

  const visibleNodes = $derived(flatten(rootNodes, showHidden || !navigation));
  // Windowed past 300 rows (ListWindow): opening node_modules or a build
  // output folder listed every entry as a button + icons — thousands of rows
  // mounted (and re-diffed on every expand) in a pane that shows ~40.
  const treeWin = new ListWindow('.tree-item');
  const treeRange = $derived(treeWin.range(visibleNodes.length));
  const hiddenCount = $derived(navigation && !showHidden ? rootNodes.filter((n) => n.entry.name.startsWith('.')).length : 0);

  // Basename of root for display.
  function basename(p: string): string {
    return p.split('/').filter(Boolean).pop() ?? p;
  }

  function onPickFolder(path: string): void {
    closeViewer();
    overriddenRoot = path;
    showPicker = false;
  }
</script>

{#if showPicker}
  <FolderPicker
    title="Choose folder"
    start={effectiveRoot}
    onpick={onPickFolder}
    onclose={() => (showPicker = false)}
  />
{/if}

  <div class="ft-wrap">
    <!-- Section header: root path + Change folder + optional close -->
    <div class="ft-header">
      <span class="ft-root-path dim" title={effectiveRoot}>
        <Icon name="folder" size={12} />
        <span class="ft-root-text">{basename(effectiveRoot)}</span>
      </span>
      {#if navigation && overriddenRoot !== undefined && (root ?? wsRoot) && overriddenRoot !== (root ?? wsRoot)}
        <button
          class="icon-btn ft-change-btn"
          title="Back to {basename(root ?? wsRoot)}"
          aria-label="Back to {basename(root ?? wsRoot)}"
          onclick={resetRoot}
        >
          <Icon name="home" size={12} />
        </button>
      {/if}
      {#if effectiveRoot}
        <!-- The tree is loaded once per root; files the agent writes afterwards
             only show up on a re-list, so give that a button. -->
        <button
          class="icon-btn ft-change-btn"
          title="Refresh folder"
          aria-label="Refresh folder"
          disabled={rootLoading}
          onclick={() => void loadRoot(effectiveRoot)}
        >
          <Icon name="refresh" size={12} />
        </button>
      {/if}
      <button
        class="icon-btn ft-change-btn"
        title="Change folder"
        aria-label="Change folder"
        onclick={() => (showPicker = true)}
      >
        <Icon name="folder" size={12} />
      </button>
      {#if onClose}
        <button
          class="icon-btn ft-close-btn"
          title="Close section"
          aria-label="Close section"
          onclick={onClose}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>

    {#if navigation && effectiveRoot}
      <!-- Folder navigation: the folder picker's history, breadcrumbs and
           go-to path, sized for the panel (icons + one scrolling crumb row). -->
      <div class="ft-nav">
        <button class="icon-btn ft-nav-btn" title="Back" aria-label="Back" disabled={backIndex === null || rootLoading} onclick={() => walk(backIndex)}>
          <Icon name="chevronLeft" size={12} />
        </button>
        <button class="icon-btn ft-nav-btn" title="Forward" aria-label="Forward" disabled={forwardIndex === null || rootLoading} onclick={() => walk(forwardIndex)}>
          <Icon name="chevronRight" size={12} />
        </button>
        <button class="icon-btn ft-nav-btn" title="Up one folder" aria-label="Up one folder" disabled={!listedParent || rootLoading} onclick={() => listedParent && navigate(listedParent)}>
          <Icon name="arrowUp" size={12} />
        </button>
        {#if editingPath}
          <input
            bind:this={pathInput}
            bind:value={pathDraft}
            class="input ft-path-input mono"
            dir="ltr"
            aria-label="Go to folder (absolute, ~ or relative path)"
            placeholder="/path, ~/path or sub/folder"
            spellcheck="false"
            autocomplete="off"
            onkeydown={onPathKey}
            onblur={() => (editingPath = false)}
          />
        {:else}
          <nav class="ft-crumbs mono" dir="ltr" aria-label="Folder path" bind:this={crumbsEl}>
            {#each folderCrumbs(listedPath || effectiveRoot) as part, i (part.path)}
              {#if i > 1}<span class="ft-crumb-sep" aria-hidden="true">/</span>{/if}
              <button
                class="ft-crumb"
                title={part.path}
                aria-current={part.path === (listedPath || effectiveRoot) ? 'location' : undefined}
                onclick={() => navigate(part.path)}
                ondblclick={() => void startPathEdit()}
              >{part.label}</button>
            {/each}
          </nav>
          <button class="icon-btn ft-nav-btn" title="Type a path to go to" aria-label="Type a path to go to" onclick={() => void startPathEdit()}>
            <Icon name="edit" size={12} />
          </button>
        {/if}
        <button
          class="icon-btn ft-nav-btn"
          class:on={showHidden}
          title={showHidden ? 'Hide hidden files' : 'Show hidden files'}
          aria-label="Show hidden files"
          aria-pressed={showHidden}
          onclick={toggleHidden}
        >
          <Icon name={showHidden ? 'eye' : 'eyeOff'} size={12} />
        </button>
      </div>
    {/if}

    {#if !effectiveRoot}
      <EmptyState
        icon="folder"
        title="No folder to browse"
        body="Choose a folder to browse, or open a file from a terminal link."
        actionLabel="Choose folder"
        actionIcon="folder"
        onaction={() => (showPicker = true)}
      />
    {:else if (rootLoading || rootError) && rootNodes.length === 0}
      {@const root = effectiveRoot}
      <!-- First load: skeleton; a failed load with nothing listed: inline error.
           A refresh keeps the tree on screen (a failed one shows the stale bar). -->
      <div class="load-pad">
        <LoadState what="the files in {basename(root)}" loading={rootLoading} error={rootError || null} empty onretry={() => void loadRoot(root)} />
      </div>
    {:else}
      {#if rootError}
        {@const root = effectiveRoot}
        <LoadState what="the files in {basename(root)}" error={rootError} empty={false} onretry={() => void loadRoot(root)} />
      {/if}
      <!-- Tree pane -->
      <div class="tree-pane" class:has-viewer={!!viewerFile}>
        <div class="tree-list" {@attach treeWin.attach}>
          {#if treeRange.top > 0}<div style="height:{treeRange.top}px" aria-hidden="true"></div>{/if}
          {#each visibleNodes.slice(treeRange.start, treeRange.end) as node (node.entry.path)}
            <div class="tree-item">
            <button
              class="tree-row"
              class:is-dir={node.entry.is_dir}
              class:is-open={node.open}
              class:is-file={!node.entry.is_dir}
              style="padding-inline-start: {8 + node.depth * 14}px"
              aria-expanded={node.entry.is_dir ? node.open : undefined}
              onclick={() => node.entry.is_dir ? toggleDir(node) : openFile(node.entry)}
              ondblclick={() => navigation && node.entry.is_dir && navigate(node.entry.path)}
              onkeydown={(e) => onTreeRowKey(e, node)}
              title={node.entry.path}
            >
              {#if node.entry.is_dir}
                <span class="chevron">
                  <Icon name={node.open ? 'chevronDown' : 'chevronRight'} size={12} />
                </span>
                {#if node.loading}
                  <span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span>
                {:else}
                  <Icon name="folder" size={12} />
                {/if}
              {:else}
                <span class="file-spacer"></span>
                <Icon name="file" size={12} />
              {/if}
              <span class="row-name">{node.entry.name}</span>
              {#if node.entry.is_git_repo}
                <span class="git-badge dim"><Icon name="branch" size={12} /></span>
              {/if}
            </button>
            {#if navigation && node.entry.is_dir}
              <button class="icon-btn enter-btn" title="Open folder {node.entry.name}" aria-label="Open folder {node.entry.name}" onclick={() => navigate(node.entry.path)}>
                <Icon name="arrow" size={12} />
              </button>
            {/if}
            </div>
          {/each}
          {#if treeRange.bottom > 0}<div style="height:{treeRange.bottom}px" aria-hidden="true"></div>{/if}
          {#if visibleNodes.length === 0}
            <div class="empty-dir dim">
              {#if hiddenCount > 0}Only hidden items here. <button class="btn small ghost" onclick={toggleHidden}>Show hidden</button>{:else}Empty folder{/if}
            </div>
          {/if}
        </div>
      </div>

    {/if}
      <!-- The viewer is independent from folder listing success. -->
      {#if viewerFile || viewerLoading || viewerError}
        <div class="viewer-pane">
          <div class="viewer-header">
            <span class="viewer-name" title={viewerFile?.path ?? viewerName}>
              <Icon name="file" size={12} />
              <!-- ellipsis only applies to a block box, not a bare text node in a flex row -->
              <span class="viewer-name-text">{viewerName}</span>
            </span>
            {#if viewerFile?.truncated}
              <span class="truncated-badge dim" title="File truncated at ~400 KB">truncated</span>
            {/if}
            {#if canPreview}
              <div class="preview-toggle">
                <button class="pv" class:active={!previewMode} aria-pressed={!previewMode} onclick={() => (previewMode = false)}>Source</button>
                <button class="pv" class:active={previewMode} aria-pressed={previewMode} onclick={() => (previewMode = true)}>Preview</button>
              </div>
            {/if}
            <button class="close-btn icon-btn" onclick={closeViewer} title="Close viewer" aria-label="Close viewer">
              <Icon name="x" size={12} />
            </button>
          </div>
          {#if viewerLoading}
            <div class="loading"><Skeleton rows={8} height={14} label={viewerName} /></div>
          {:else if viewerError}
            <div class="error-msg" role="alert">{viewerError}</div>
          {:else if viewerFile}
            {#if canPreview && previewMode}
              <iframe class="preview-frame" title="Preview" sandbox="allow-same-origin" srcdoc={previewSrcdoc}></iframe>
            {:else}
              <div class="code-scroll">
                <CodeEditor
                  path={viewerFile.path}
                  content={viewerFile.content}
                  root={effectiveRoot}
                  lsp
                  gotoLine={viewerGotoLine}
                  gotoCol={viewerGotoCol}
                  readOnly
                />
              </div>
            {/if}
          {/if}
        </div>
      {/if}
  </div>

<style>
  .ft-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  /* ── section header ───────────────────── */
  .ft-header {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: 2px; padding-inline: 8px 4px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    flex-shrink: 0;
    min-width: 0;
  }

  .ft-root-path {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-family: var(--font-mono);
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }

  .ft-root-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ft-change-btn {
    flex-shrink: 0;
    opacity: 0.55;
    padding: 2px 4px;
  }
  .ft-change-btn:hover:not(:disabled) {
    opacity: 1;
  }

  .ft-close-btn {
    flex-shrink: 0;
    opacity: 0.45;
    padding: 2px 4px;
  }
  .ft-close-btn:hover {
    opacity: 1;
    color: var(--danger);
  }

  /* ── navigation ───────────────────────── */
  .ft-nav {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 4px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .ft-nav-btn {
    flex-shrink: 0;
  }
  .ft-nav-btn.on {
    color: var(--accent-text);
  }
  .ft-crumbs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 1px;
    overflow-x: auto;
    white-space: nowrap;
    scrollbar-width: none;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ft-crumb {
    flex-shrink: 0;
    font: inherit;
    color: inherit;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    padding: 2px 4px;
    cursor: pointer;
  }
  .ft-crumb:hover {
    background: var(--hover);
    color: var(--text);
  }
  .ft-crumb[aria-current] {
    color: var(--text);
    font-weight: 600;
  }
  .ft-crumb-sep {
    flex-shrink: 0;
  }
  .ft-path-input {
    flex: 1;
    min-width: 0;
    height: 22px;
    font-size: var(--fs-xs);
  }

  /* ── tree ─────────────────────────────── */
  .tree-pane {
    flex: 1 1 0;
    overflow-y: auto;
    min-height: 60px;
  }
  .tree-pane.has-viewer {
    /* Sparse folders give their unused room to the document; large trees
       retain a bounded, independently scrollable navigation area. */
    flex: 0 1 auto;
    max-height: 45%;
    border-bottom: 1px solid var(--border);
  }

  .tree-list {
    padding: 2px 0;
  }

  .tree-item {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .tree-item:hover {
    background: var(--hover);
  }
  /* "Open this folder": shown on row hover / keyboard focus. */
  .enter-btn {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    margin-inline-end: 4px;
    opacity: 0;
  }
  .tree-item:hover .enter-btn,
  .enter-btn:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .enter-btn {
      opacity: 0.6;
    }
  }
  .tree-row {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
    white-space: nowrap;
    overflow: hidden;
    padding-inline-end: 6px;
    transition: background var(--dur-fast) ease-out;
  }
  .tree-row:hover {
    background: var(--hover);
  }
  .tree-row.is-dir {
    color: var(--text);
  }
  .tree-row.is-file {
    color: var(--text-dim);
  }
  .tree-row.is-file:hover {
    color: var(--text);
  }

  .chevron {
    display: flex;
    align-items: center;
    justify-content: center;
    /* Same width as .file-spacer so folder and file icons line up per depth. */
    width: 14px;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .file-spacer {
    width: 14px;
    flex-shrink: 0;
  }
  .row-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .git-badge {
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }


  .empty-dir {
    padding: 8px 12px;
    font-size: var(--fs-xs);
  }

  /* ── viewer ───────────────────────────── */
  .viewer-pane {
    flex: 1 1 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 80px;
  }

  .viewer-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    flex-shrink: 0;
    min-width: 0;
  }
  .viewer-name {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
    color: var(--text);
  }
  .viewer-name-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .truncated-badge {
    font-size: var(--fs-xs);
    padding: 1px 4px;
    border-radius: var(--radius-s);
    background: var(--accent-soft);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .close-btn {
    flex-shrink: 0;
  }

  .code-scroll {
    flex: 1;
    overflow: hidden;
    min-height: 0;
  }
  .preview-toggle {
    display: inline-flex;
    gap: 2px;
    margin-inline-start: auto;
  }
  .pv {
    height: 20px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .pv.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .preview-frame {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: none;
    /* The srcdoc body is transparent over this; its palette follows the scheme. */
    background: var(--bg);
  }

  /* ── misc ─────────────────────────────── */
  .loading {
    padding: 12px;
    font-size: var(--fs-s);
  }
  .load-pad {
    padding: 12px;
  }
  .error-msg {
    padding: 12px;
    font-size: var(--fs-s);
    color: var(--danger);
    word-break: break-all;
  }
</style>
