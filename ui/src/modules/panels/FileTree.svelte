<script lang="ts">
  // FileTree: lazy browsable file tree + read-only syntax-highlighted viewer
  // for the right-panel Files tab.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { marked } from 'marked';
  import type { FsBrowse, FsEntry, FsRead } from '../../lib/api/types';

  interface Props {
    /** When provided, the tree is rooted here; otherwise falls back to
     *  ws.activeSession?.cwd or ws.current?.root_path. */
    root?: string;
    /** When provided, a ✕ button is shown in the header to close this section. */
    onClose?: () => void;
  }
  let { root = undefined, onClose = undefined }: Props = $props();

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

  // ── viewer state ──────────────────────────────────────────────────────────

  let viewerFile: FsRead | null = $state(null);
  let viewerName = $state('');
  let viewerLoading = $state(false);

  // ── folder picker state ───────────────────────────────────────────────────

  let showPicker = $state(false);

  // ── effects ───────────────────────────────────────────────────────────────

  $effect(() => {
    const r = effectiveRoot;
    if (r && r !== loadedRoot) {
      // Reset tree state when root changes.
      rootNodes = [];
      viewerFile = null;
      viewerName = '';
      loadRoot(r);
    }
  });

  // ── helpers ───────────────────────────────────────────────────────────────

  async function loadRoot(r: string): Promise<void> {
    rootLoading = true;
    rootError = '';
    try {
      const data = await api.get<FsBrowse>(`/fs/browse?path=${encodeURIComponent(r)}&files=true`);
      rootNodes = data.entries.map((e) => makeNode(e, 0));
      loadedRoot = r;
    } catch (e) {
      rootError = e instanceof Error ? e.message : String(e);
    } finally {
      rootLoading = false;
    }
  }

  function makeNode(entry: FsEntry, depth: number): TreeNode {
    return { entry, depth, open: false, loaded: false, loading: false, children: [] };
  }

  async function toggleDir(node: TreeNode): Promise<void> {
    if (!node.entry.is_dir) return;
    if (!node.open) {
      // Open: lazy-load children if not yet done.
      node.open = true;
      if (!node.loaded) {
        node.loading = true;
        try {
          const data = await api.get<FsBrowse>(
            `/fs/browse?path=${encodeURIComponent(node.entry.path)}&files=true`,
          );
          node.children = data.entries.map((e) => makeNode(e, node.depth + 1));
          node.loaded = true;
        } catch (e) {
          toasts.error('Cannot open folder', e instanceof Error ? e.message : String(e));
          node.open = false;
        } finally {
          node.loading = false;
        }
      }
    } else {
      node.open = false;
    }
    // Trigger reactivity by reassigning rootNodes.
    rootNodes = [...rootNodes];
  }

  async function openFile(entry: FsEntry): Promise<void> {
    viewerLoading = true;
    viewerName = entry.name;
    viewerFile = null;
    previewMode = true; // previewable files default to rendered view
    try {
      const data = await api.get<FsRead>(`/fs/read?path=${encodeURIComponent(entry.path)}`);
      viewerFile = data;
    } catch (e) {
      toasts.error('Cannot read file', e instanceof Error ? e.message : String(e));
      viewerLoading = false;
      return;
    } finally {
      viewerLoading = false;
    }
  }

  function closeViewer(): void {
    viewerFile = null;
    viewerName = '';
  }

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
  const PREVIEW_CSS = `
    :root { color-scheme: light dark; }
    body { font: 14px/1.6 -apple-system, system-ui, sans-serif; margin: 16px; color: #ddd; background: transparent; }
    h1,h2,h3 { line-height: 1.25; } h1,h2 { border-bottom: 1px solid #ffffff22; padding-bottom: .2em; }
    a { color: #6ea8fe; } code { background: #ffffff14; padding: .15em .35em; border-radius: 4px; font-family: ui-monospace, monospace; }
    pre { background: #ffffff10; padding: 12px; border-radius: 6px; overflow: auto; } pre code { background: none; padding: 0; }
    table { border-collapse: collapse; } th,td { border: 1px solid #ffffff22; padding: 4px 8px; }
    blockquote { border-left: 3px solid #ffffff33; margin: 0; padding-left: 12px; color: #aaa; }
    img { max-width: 100%; }
  `;

  // Collect flattened visible nodes for rendering (DFS walk).
  function flatten(nodes: TreeNode[]): TreeNode[] {
    const out: TreeNode[] = [];
    for (const n of nodes) {
      out.push(n);
      if (n.open && n.children.length > 0) {
        out.push(...flatten(n.children));
      }
    }
    return out;
  }

  const visibleNodes = $derived(flatten(rootNodes));

  // Basename of root for display.
  function basename(p: string): string {
    return p.split('/').filter(Boolean).pop() ?? p;
  }

  function onPickFolder(path: string): void {
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

{#if !effectiveRoot}
  <EmptyState icon="folder" title="No workspace" body="Open a workspace to browse its files." />
{:else}
  <div class="ft-wrap">
    <!-- Section header: root path + Change folder + optional close -->
    <div class="ft-header">
      <span class="ft-root-path dim" title={effectiveRoot}>
        <Icon name="folder" size={11} />
        <span class="ft-root-text">{basename(effectiveRoot)}</span>
      </span>
      <button
        class="icon-btn ft-change-btn"
        title="Change folder…"
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

    {#if rootLoading}
      <div class="loading dim">Loading…</div>
    {:else if rootError}
      <div class="error-msg">{rootError}</div>
    {:else}
      <!-- Tree pane -->
      <div class="tree-pane" class:has-viewer={!!viewerFile}>
        <div class="tree-list">
          {#each visibleNodes as node (node.entry.path)}
            <button
              class="tree-row"
              class:is-dir={node.entry.is_dir}
              class:is-open={node.open}
              class:is-file={!node.entry.is_dir}
              style="padding-left: {8 + node.depth * 14}px"
              onclick={() => node.entry.is_dir ? toggleDir(node) : openFile(node.entry)}
              title={node.entry.path}
            >
              {#if node.entry.is_dir}
                <span class="chevron">
                  <Icon name={node.open ? 'chevronDown' : 'chevronRight'} size={10} />
                </span>
                {#if node.loading}
                  <span class="spin"><Icon name="refresh" size={12} /></span>
                {:else}
                  <Icon name="folder" size={12} />
                {/if}
              {:else}
                <span class="file-spacer"></span>
                <Icon name="file" size={12} />
              {/if}
              <span class="row-name">{node.entry.name}</span>
              {#if node.entry.is_git_repo}
                <span class="git-badge dim"><Icon name="branch" size={10} /></span>
              {/if}
            </button>
          {/each}
          {#if visibleNodes.length === 0}
            <div class="empty-dir dim">Empty folder</div>
          {/if}
        </div>
      </div>

      <!-- Viewer pane -->
      {#if viewerFile || viewerLoading}
        <div class="viewer-pane">
          <div class="viewer-header">
            <span class="viewer-name">
              <Icon name="file" size={11} />
              {viewerName}
            </span>
            {#if viewerFile?.truncated}
              <span class="truncated-badge dim" title="File truncated at ~400 KB">truncated</span>
            {/if}
            {#if canPreview}
              <div class="preview-toggle">
                <button class="pv" class:active={!previewMode} onclick={() => (previewMode = false)}>Source</button>
                <button class="pv" class:active={previewMode} onclick={() => (previewMode = true)}>Preview</button>
              </div>
            {/if}
            <button class="close-btn icon-btn" onclick={closeViewer} title="Close viewer" aria-label="Close viewer">
              <Icon name="x" size={12} />
            </button>
          </div>
          {#if viewerLoading}
            <div class="loading dim">Loading…</div>
          {:else if viewerFile}
            {#if canPreview && previewMode}
              <iframe class="preview-frame" title="Preview" sandbox="allow-same-origin" srcdoc={previewSrcdoc}></iframe>
            {:else}
              <div class="code-scroll">
                <CodeEditor
                  path={viewerFile.path}
                  content={viewerFile.content}
                  root={effectiveRoot}
                  readOnly
                />
              </div>
            {/if}
          {/if}
        </div>
      {/if}
    {/if}
  </div>
{/if}

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
    padding: 3px 4px 3px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    flex-shrink: 0;
    min-width: 0;
  }

  .ft-root-path {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
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
  .ft-change-btn:hover {
    opacity: 1;
  }

  .ft-close-btn {
    flex-shrink: 0;
    opacity: 0.45;
    padding: 2px 4px;
  }
  .ft-close-btn:hover {
    opacity: 1;
    color: var(--danger, #e05252);
  }

  /* ── tree ─────────────────────────────── */
  .tree-pane {
    flex: 1 1 0;
    overflow-y: auto;
    min-height: 60px;
  }
  .tree-pane.has-viewer {
    flex: 0 0 45%;
    border-bottom: 1px solid var(--border);
  }

  .tree-list {
    padding: 2px 0;
  }

  .tree-row {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    height: 22px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    padding-right: 6px;
    transition: background 80ms ease-out;
  }
  .tree-row:hover {
    background: var(--surface-2);
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

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
  .spin {
    display: flex;
    align-items: center;
    animation: spin 0.8s linear infinite;
  }

  .empty-dir {
    padding: 8px 12px;
    font-size: 11.5px;
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
    font-size: 11.5px;
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    color: var(--text);
  }
  .truncated-badge {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
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
    margin-left: auto;
  }
  .pv {
    height: 20px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .pv.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .preview-frame {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: none;
    background: var(--surface-1, #1a1a1a);
  }

  /* ── misc ─────────────────────────────── */
  .loading {
    padding: 12px;
    font-size: 12px;
  }
  .error-msg {
    padding: 12px;
    font-size: 12px;
    color: var(--danger, #e05252);
    word-break: break-all;
  }
</style>
