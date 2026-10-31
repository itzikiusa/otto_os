<script lang="ts">
  import { api, baseUrl, getToken, isAbortError } from '../../lib/api/client';
  import { copyWithToast } from '../../lib/clipboard';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { renderMarkdownGfm } from '../../lib/md';
  import { onTabKey } from '../../lib/tabKeys';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { FsBrowse, FsEntry, FsRead } from '../../lib/api/types';
  import {
    baseName,
    compactPath,
    parseDelimitedTable,
    previewKindForPath,
    relToRoot,
  } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    rootPath: string;
    wsKey: string;
  }
  let { rootPath, wsKey }: Props = $props();

  let treeOpen = $state(true);
  let showHidden = $state(false);
  let browseDir = $state('');
  let parentDir = $state<string | null>(null);
  let entries = $state<FsEntry[]>([]);
  let treeLoading = $state(false);
  let treeError = $state<string | null>(null);
  let treeRetry = $state(0);

  let fileRead = $state<FsRead | null>(null);
  let fileLoading = $state(false);
  let fileError = $state<string | null>(null);
  let fileRetry = $state(0);
  let viewMode = $state<'preview' | 'source'>('source');

  const homePath = $derived(ws.scratch?.root_path ?? null);
  const tabState = $derived(threads.fileState(wsKey));
  const activeTab = $derived(tabState.tabs.find((t) => t.path === tabState.activeFilePath) ?? null);
  const previewKind = $derived(activeTab ? previewKindForPath(activeTab.path) : 'code');
  const hasRichPreview = $derived(previewKind !== 'code');

  // Reset browsed folder when workspace/session root changes.
  let prevRoot = '';
  $effect(() => {
    const r = rootPath;
    if (r && r !== prevRoot) {
      prevRoot = r;
      browseDir = r;
    }
  });

  // Load directory listing.
  $effect(() => {
    const dir = browseDir || rootPath;
    void treeRetry;
    if (!dir) {
      entries = [];
      return;
    }
    const ctl = new AbortController();
    treeLoading = true;
    treeError = null;
    void api
      .get<FsBrowse>(`/fs/browse?path=${encodeURIComponent(dir)}&files=true`, ctl.signal)
      .then((res) => {
        if (ctl.signal.aborted) return;
        browseDir = res.path;
        parentDir = res.parent;
        entries = res.entries;
      })
      .catch((e) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        treeError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) treeLoading = false;
      });
    return () => ctl.abort();
  });

  // Load active open file content.
  $effect(() => {
    const path = activeTab?.path ?? null;
    void fileRetry;
    if (!path) {
      fileRead = null;
      fileError = null;
      fileLoading = false;
      return;
    }
    const kind = previewKindForPath(path);
    viewMode = kind === 'code' ? 'source' : 'preview';
    if (kind === 'image') {
      fileRead = { path, content: '', language: 'image', truncated: false };
      fileError = null;
      fileLoading = false;
      return;
    }
    const ctl = new AbortController();
    fileLoading = true;
    fileError = null;
    void api
      .get<FsRead>(`/fs/read?path=${encodeURIComponent(path)}`, ctl.signal)
      .then((res) => {
        if (!ctl.signal.aborted) fileRead = res;
      })
      .catch((e) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        fileRead = null;
        fileError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) fileLoading = false;
      });
    return () => ctl.abort();
  });

  const visibleEntries = $derived(
    entries.filter((e) => showHidden || !e.name.startsWith('.')),
  );

  const formattedJson = $derived.by(() => {
    if (previewKind !== 'json' || !fileRead?.content) return null;
    try {
      return JSON.stringify(JSON.parse(fileRead.content), null, 2);
    } catch {
      return null;
    }
  });

  const parsedTable = $derived.by(() => {
    if (previewKind !== 'csv' || !fileRead?.content) return null;
    const delim = activeTab?.path.toLowerCase().endsWith('.tsv') ? '\t' : ',';
    return parseDelimitedTable(fileRead.content, delim);
  });

  function rawMediaSrc(path: string): string {
    const tok = getToken();
    const q = new URLSearchParams({ path });
    if (tok) q.set('token', tok);
    return `${baseUrl()}/api/v1/fs/raw?${q.toString()}`;
  }

  function insertIntoComposer(path: string): void {
    const rel = relToRoot(path, rootPath);
    const ref = `@${rel} `;
    const fid = threads.focusedSessionId;
    if (fid && typeof document !== 'undefined') {
      const paneEl = document.querySelector(`[data-testid="thread-pane-${fid}"]`);
      const ta = paneEl?.querySelector<HTMLTextAreaElement>('textarea:not([disabled])');
      if (ta) {
        const start = ta.selectionStart ?? ta.value.length;
        const end = ta.selectionEnd ?? ta.value.length;
        const prefix = start > 0 && !/\s$/.test(ta.value.slice(0, start)) ? ' ' : '';
        ta.value = `${ta.value.slice(0, start)}${prefix}${ref}${ta.value.slice(end)}`;
        const caret = start + prefix.length + ref.length;
        ta.selectionStart = caret;
        ta.selectionEnd = caret;
        ta.dispatchEvent(new Event('input', { bubbles: true }));
        ta.focus();
        toasts.info('Inserted into composer', `@${rel}`);
        return;
      }
    }
    void copyWithToast(`@${rel}`, 'Copied @file reference');
  }
</script>

<div class="tf-wrap" data-testid="inspector-files-tab">
  <div class="tf-explorer" class:collapsed={!treeOpen}>
    <div class="tf-exp-bar">
      <button
        type="button"
        class="tf-exp-toggle"
        aria-expanded={treeOpen}
        onclick={() => (treeOpen = !treeOpen)}
      >
        <Icon name={treeOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        <Icon name="folder" size={13} />
        <span class="mono ellipsis" dir="ltr" title={browseDir || rootPath}>
          {compactPath(browseDir || rootPath, homePath)}
        </span>
      </button>
      <div class="tf-exp-btns">
        {#if parentDir}
          <button
            type="button"
            class="icon-btn"
            aria-label="Up to parent directory"
            title="Parent folder"
            onclick={() => {
              if (parentDir) browseDir = parentDir;
            }}
          >
            <Icon name="arrowUp" size={12} />
          </button>
        {/if}
        {#if browseDir && rootPath && browseDir !== rootPath}
          <button
            type="button"
            class="icon-btn"
            aria-label="Reset to project root"
            title="Project root"
            onclick={() => (browseDir = rootPath)}
          >
            <Icon name="home" size={12} />
          </button>
        {/if}
        <button
          type="button"
          class="icon-btn"
          class:active={showHidden}
          aria-pressed={showHidden}
          aria-label={showHidden ? 'Hide dotfiles' : 'Show dotfiles'}
          title={showHidden ? 'Hide dotfiles' : 'Show dotfiles'}
          onclick={() => (showHidden = !showHidden)}
        >
          <Icon name={showHidden ? 'eye' : 'eyeOff'} size={12} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Refresh directory"
          title="Refresh files"
          onclick={() => treeRetry++}
        >
          <Icon name="refresh" size={12} />
        </button>
      </div>
    </div>

    {#if treeOpen}
      <div class="tf-tree-list">
        <LoadState
          what="project files"
          variant="compact"
          loading={treeLoading}
          error={treeError}
          empty={visibleEntries.length === 0}
          onretry={() => treeRetry++}
        >
          {#snippet emptyView()}
            <p class="tf-empty-note">Folder is empty.</p>
          {/snippet}
          {#each visibleEntries as entry (entry.path)}
            <button
              type="button"
              class="tf-entry"
              class:active={activeTab?.path === entry.path}
              dir="ltr"
              onclick={() => {
                if (entry.is_dir) browseDir = entry.path;
                else threads.openFileTab(wsKey, entry.path, { pin: false });
              }}
              ondblclick={() => {
                if (!entry.is_dir) threads.openFileTab(wsKey, entry.path, { pin: true });
              }}
            >
              <Icon name={entry.is_dir ? 'folder' : 'file'} size={13} />
              <span class="ellipsis">{entry.name}</span>
            </button>
          {/each}
        </LoadState>
      </div>
    {/if}
  </div>

  {#if tabState.tabs.length > 0}
    <div class="tf-tabstrip" role="toolbar" aria-label="Open files">
      {#each tabState.tabs as tab (tab.path)}
        {@const active = tab.path === tabState.activeFilePath}
        <div class="tf-tab" class:active class:preview={!tab.pinned}>
          <button
            type="button"
            aria-pressed={active}
            class="tf-tab-btn"
            title={tab.path}
            onkeydown={onTabKey}
            onclick={() => threads.setActiveFile(wsKey, tab.path)}
            ondblclick={() => threads.pinFileTab(wsKey, tab.path)}
          >
            <Icon name="file" size={12} />
            <span class="ellipsis" dir="ltr">{baseName(tab.path)}</span>
          </button>
          <button
            type="button"
            class="icon-btn tf-tab-close"
            aria-label="Close {baseName(tab.path)}"
            title="Close file tab"
            onclick={() => threads.closeFileTab(wsKey, tab.path)}
          >
            <Icon name="x" size={12} />
          </button>
        </div>
      {/each}
    </div>
  {/if}

  <div class="tf-viewer">
    {#if !activeTab}
      <EmptyState
        variant="panel"
        icon="file"
        title="Open a file to inspect"
        body="Click any file above — single-click previews, double-click pins its tab. Markdown, JSON, CSV, SVG and code render inline."
      />
    {:else}
      <div class="tf-file-toolbar">
        {#if hasRichPreview}
          <div class="segmented" role="group" aria-label="File view mode">
            <button
              type="button"
              class:active={viewMode === 'preview'}
              aria-pressed={viewMode === 'preview'}
              onkeydown={onTabKey}
              onclick={() => (viewMode = 'preview')}
            >
              Preview
            </button>
            {#if previewKind !== 'image'}
              <button
                type="button"
                class:active={viewMode === 'source'}
                aria-pressed={viewMode === 'source'}
                onkeydown={onTabKey}
                onclick={() => (viewMode = 'source')}
              >
                Source
              </button>
            {/if}
          </div>
        {:else}
          <span class="tf-path-hint mono ellipsis" dir="ltr" title={activeTab.path}>
            {relToRoot(activeTab.path, rootPath)}
          </span>
        {/if}

        <div class="tf-file-actions">
          <button
            type="button"
            class="btn small ghost"
            title="Copy relative file path"
            onclick={() => void copyWithToast(relToRoot(activeTab.path, rootPath), 'Copied file path')}
          >
            <Icon name="copy" size={12} />
            Path
          </button>
          <button
            type="button"
            class="btn small"
            title="Insert @file reference into focused thread composer"
            onclick={() => insertIntoComposer(activeTab.path)}
          >
            <Icon name="send" size={12} />
            @file
          </button>
        </div>
      </div>

      <div class="tf-file-body">
        <LoadState
          what="file contents"
          variant="panel"
          loading={fileLoading}
          error={fileError}
          empty={!fileRead}
          onretry={() => fileRetry++}
        >
          {#if fileRead}
            {#if viewMode === 'preview' && previewKind === 'markdown'}
              <div class="tf-md md-body" dir="auto">
                {@html renderMarkdownGfm(fileRead.content)}
              </div>
            {:else if viewMode === 'preview' && previewKind === 'json' && formattedJson}
              <div class="tf-editor-host">
                <CodeEditor
                  path={fileRead.path}
                  content={formattedJson}
                  root={rootPath}
                  language="json"
                  readOnly
                />
              </div>
            {:else if viewMode === 'preview' && previewKind === 'csv' && parsedTable}
              <div class="tf-table-wrap">
                <table class="tf-table">
                  <thead>
                    <tr>
                      {#each parsedTable.headers as h, idx (idx)}
                        <th>{h}</th>
                      {/each}
                    </tr>
                  </thead>
                  <tbody>
                    {#each parsedTable.rows as row, rIdx (rIdx)}
                      <tr>
                        {#each parsedTable.headers as _, cIdx (cIdx)}
                          <td>{row[cIdx] ?? ''}</td>
                        {/each}
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {:else if viewMode === 'preview' && (previewKind === 'image' || previewKind === 'svg')}
              <div class="tf-image-wrap">
                <img src={rawMediaSrc(activeTab.path)} alt={baseName(activeTab.path)} />
              </div>
            {:else}
              <div class="tf-editor-host">
                <CodeEditor
                  path={fileRead.path}
                  content={fileRead.content}
                  root={rootPath}
                  language={fileRead.language}
                  gotoLine={activeTab.line ?? null}
                  readOnly
                />
              </div>
            {/if}
          {/if}
        </LoadState>
      </div>
    {/if}
  </div>
</div>

<style>
  .tf-wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .tf-explorer {
    display: flex;
    flex-direction: column;
    max-block-size: 220px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tf-explorer.collapsed {
    max-block-size: none;
  }
  .tf-exp-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 6px 8px;
    background: var(--surface-2);
  }
  .tf-exp-toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    border: 0;
    padding: 2px 4px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 500;
    text-align: start;
    cursor: pointer;
  }
  .tf-exp-btns {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
  .tf-exp-btns :global(.icon-btn.active) {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .tf-tree-list {
    overflow-y: auto;
    padding: 4px 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tf-entry {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .tf-entry:hover {
    background: var(--hover);
  }
  .tf-entry.active {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 500;
  }
  .tf-empty-note {
    margin: 0;
    padding: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tf-tabstrip {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 6px;
    overflow-x: auto;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    scrollbar-width: none;
  }
  .tf-tabstrip::-webkit-scrollbar {
    display: none;
  }
  .tf-tab {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-inline: 6px 2px;
    padding-block: 2px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    flex-shrink: 0;
    max-width: 180px;
  }
  .tf-tab:hover {
    background: var(--hover);
  }
  .tf-tab.active {
    background: var(--surface-2);
    border-color: var(--border-strong);
  }
  .tf-tab.preview .tf-tab-btn {
    font-style: italic;
  }
  .tf-tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    border: 0;
    padding: 2px 4px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .tf-tab-close {
    inline-size: 20px;
    block-size: 20px;
  }
  .tf-viewer {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tf-file-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .tf-path-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
  }
  .tf-file-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .tf-file-body {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .tf-editor-host {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: auto;
  }
  .tf-md {
    flex: 1;
    overflow-y: auto;
    padding: 14px 16px;
    font-size: var(--fs-s);
  }
  .tf-table-wrap {
    flex: 1;
    overflow: auto;
  }
  .tf-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
  }
  .tf-table th,
  .tf-table td {
    padding: 6px 8px;
    border-block-end: 1px solid var(--border);
    text-align: start;
    white-space: nowrap;
  }
  .tf-table th {
    position: sticky;
    inset-block-start: 0;
    background: var(--surface-2);
    font-weight: 600;
    color: var(--text);
  }
  .tf-image-wrap {
    flex: 1;
    overflow: auto;
    display: grid;
    place-items: center;
    padding: 16px;
  }
  .tf-image-wrap img {
    max-width: 100%;
    max-height: 100%;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
</style>
