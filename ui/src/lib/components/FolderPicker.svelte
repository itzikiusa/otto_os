<script lang="ts">
  // Daemon-side folder/file picker. Browses the directory tree on the machine that
  // runs ottod (the path the user picks is used as a session cwd / repo path,
  // so it must exist on the daemon host, not the client). Backed by GET
  // /fs/browse. `gitOnly` highlights git repos for the repo picker.
  // When `files` is true, files are shown and can be picked directly (for
  // identity-file selection etc.); directories still navigate on click.
  import { untrack, onDestroy, onMount, tick } from 'svelte';
  import { emptyHistory, recordFolder, historyTarget, folderCrumbs, emptyShortcuts, parseShortcuts, rememberFolder, toggleFavorite } from './folderNavigation';
  import { auth } from '../stores/auth.svelte';
  import { api, baseUrl } from '../api/client';
  import type { FsBrowse, FsEntry } from '../api/types';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import VirtualList from './VirtualList.svelte';
  import { moveGrid, gridAction } from './folderGrid';

  interface Props {
    title?: string;
    start?: string;
    /** Only allow picking directories that are git repos (repo registration). */
    gitOnly?: boolean;
    /** When true, files are shown as selectable rows and the picker can return
     *  a file path. Directory navigation still works normally. Default false. */
    files?: boolean;
    onpick: (path: string) => void;
    onclose: () => void;
  }
  let { title = 'Choose Folder', start = '', gitOnly = false, files = false, onpick, onclose }: Props = $props();

  let view: FsBrowse | null = $state(null);
  let loading = $state(true);
  let error = $state('');
  // Type-to-filter the current listing; "Show hidden" reveals dotfiles
  // (default off — dotfiles are hidden so the listing isn't cluttered).
  let filter = $state('');
  let filterElement: HTMLInputElement | undefined = $state();
  let retryElement: HTMLButtonElement | undefined = $state();
  let gridElement: HTMLDivElement | undefined = $state();
  let position = $state({ index: 0, action: 0 });
  let scrollVersion = $state(0);
  const gridId = `folder-grid-${crypto.randomUUID()}`;
  let showHidden = $state(false);

  // The entries actually rendered: dotfiles hidden unless `showHidden`, and
  // name-filtered (case-insensitive) by the filter box. The `..` up row is
  // rendered separately and always shown.
  const shown = $derived.by((): FsEntry[] => {
    const entries: FsEntry[] = view?.entries ?? [];
    const q = filter.trim().toLowerCase();
    return entries.filter((e) => {
      if (!showHidden && e.name.startsWith('.')) return false;
      return q === '' || e.name.toLowerCase().includes(q);
    });
  });

  let history = $state(emptyHistory());
  let shortcuts = $state(emptyShortcuts());
  let storageKey = '';
  let crumbsElement: HTMLElement | undefined = $state();
  let requestSeq = 0;
  let requestController: AbortController | null = null;
  let lastAttempt: { path: string; index?: number } = { path: '' };
  const backIndex = $derived(historyTarget(history, -1));
  const forwardIndex = $derived(historyTarget(history, 1));

  async function load(path: string, index?: number): Promise<void> {
    const restoreFocus = !!gridElement && (document.activeElement === gridElement || gridElement.contains(document.activeElement));
    if (restoreFocus) filterElement?.focus();
    requestController?.abort();
    const controller = new AbortController();
    requestController = controller;
    let timedOut = false;
    const timeout = setTimeout(() => { timedOut = true; controller.abort(); }, 12000);
    const seq = ++requestSeq;
    lastAttempt = { path, index };
    loading = true;
    error = '';
    try {
      let q = path ? `?path=${encodeURIComponent(path)}` : '';
      if (files) q = q ? `${q}&files=true` : '?files=true';
      const next = await api.get<FsBrowse>(`/fs/browse${q}`, controller.signal);
      if (seq !== requestSeq) return;
      view = next;
      updateShortcuts(current => rememberFolder(current, next.path));
      filter = ''; // A successful navigation starts with the full listing.
      if (index === undefined) history = recordFolder(history, next.path);
      else {
        const paths = [...history.paths];
        paths[index] = next.path;
        history = { paths, index };
      }
      await tick();
      if (seq === requestSeq && crumbsElement) crumbsElement.scrollLeft = crumbsElement.scrollWidth;
    } catch (e) {
      if (seq === requestSeq) error = timedOut
        ? 'Folder listing timed out. Retry or choose another folder.'
        : (e instanceof Error ? e.message : String(e)) || 'Could not open folder.';
    } finally {
      clearTimeout(timeout);
      if (seq === requestSeq) {
        loading = false;
        if (requestController === controller) requestController = null;
        if (restoreFocus) { await tick(); if (seq === requestSeq) (error ? retryElement : gridElement)?.focus(); }
      }
    }
  }

  function disposeRequest() { requestSeq++; requestController?.abort(); requestController = null; }
  function closePicker() { disposeRequest(); onclose(); }
  function pick(path: string) { disposeRequest(); onpick(path); }
  function cellId(index: number, action: number) { return `${gridId}-${encodeURIComponent(shown[index]?.path ?? '')}-${action}`; }
  function activate(index: number, action: number) {
    const entry = shown[index]; if (!entry) return;
    const intent = gridAction(entry, action, gitOnly);
    if (intent === 'open') void load(entry.path);
    else if (intent === 'pick') pick(entry.path);
  }
  function gridKey(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey || shown.length === 0) return;
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); event.stopPropagation(); activate(position.index, position.action); return; }
    const next = moveGrid(position, event.key, shown.length, Math.max(1, Math.floor((gridElement?.clientHeight ?? 288) / 32)));
    if (!next) return;
    event.preventDefault(); event.stopPropagation();
    if (!gridAction(shown[next.index], next.action, gitOnly)) next.action = 0;
    position = next; scrollVersion++;
  }
  $effect(() => {
    void filter; void showHidden; void view?.path;
    untrack(() => { position = { index: 0, action: 0 }; scrollVersion++; });
  });

  function updateShortcuts(update: (current: ReturnType<typeof emptyShortcuts>) => ReturnType<typeof emptyShortcuts>) {
    // Merge against storage at mutation time: another window may have pinned a
    // folder since this picker opened. Navigating must never overwrite that pin.
    let latest = shortcuts;
    try { latest = parseShortcuts(localStorage.getItem(storageKey)); } catch { /* Memory-only fallback. */ }
    shortcuts = update(latest);
    try {
      localStorage.setItem(storageKey, JSON.stringify(shortcuts));
      window.dispatchEvent(new CustomEvent('otto:folder-shortcuts', { detail: storageKey }));
    } catch { /* Private browsing may disable storage. */ }
  }

  function favoriteCurrent() {
    if (!view || loading || error) return;
    const path = view.path;
    updateShortcuts(current => toggleFavorite(current, path));
  }

  onMount(() => {
    const refresh = () => {
      try { shortcuts = parseShortcuts(localStorage.getItem(storageKey)); } catch { /* Keep in-memory choices. */ }
    };
    const stored = (event: StorageEvent) => { if (event.key === storageKey || event.key === null) refresh(); };
    const local = (event: Event) => { if ((event as CustomEvent).detail === storageKey) refresh(); };
    window.addEventListener('storage', stored);
    window.addEventListener('otto:folder-shortcuts', local);
    return () => {
      window.removeEventListener('storage', stored);
      window.removeEventListener('otto:folder-shortcuts', local);
    };
  });

  function folderName(path: string) { return path.split('/').filter(Boolean).at(-1) ?? '/'; }

  function traverse(index: number | null) {
    if (index !== null) void load(history.paths[index], index);
  }

  $effect(() => {
    const initial = start;
    const key = `otto_folder_shortcuts:${baseUrl()}:${auth.me?.id ?? 'anonymous'}`;
    void files;
    untrack(() => {
      storageKey = key;
      try { shortcuts = parseShortcuts(localStorage.getItem(key)); } catch { shortcuts = emptyShortcuts(); }
      history = emptyHistory();
      view = null;
      void load(initial);
    });
  });
  onDestroy(disposeRequest);
</script>

<Modal {title} onclose={closePicker} width={620}>
  <div class="navigation" aria-label="Folder navigation">
    <button class="btn" disabled={loading || backIndex === null} onclick={() => traverse(backIndex)}>Back</button>
    <button class="btn" disabled={loading || forwardIndex === null} onclick={() => traverse(forwardIndex)}>Forward</button>
    <button class="btn" disabled={loading || !view?.parent} onclick={() => view?.parent && load(view.parent)}>Up</button>
    <button class="btn favorite-action" disabled={!view || loading || !!error} onclick={favoriteCurrent}>
      {view && shortcuts.favorites.includes(view.path) ? 'Remove favorite' : 'Add favorite'}
    </button>
  </div>
  {#if view}
    <nav bind:this={crumbsElement} class="crumb mono" dir="ltr" aria-label="Folder path" data-path={view.path}>
      {#each folderCrumbs(view.path) as part, i (part.path)}
        {#if i > 1}<span aria-hidden="true">/</span>{/if}
        <button title={part.path} aria-current={part.path === view.path ? 'location' : undefined}
          disabled={loading} onclick={() => load(part.path)}>{part.label}</button>
      {/each}
    </nav>
  {/if}

  <div class="pick-tools">
    <!-- svelte-ignore a11y_autofocus -->
    <input class="input filter-input" placeholder="Filter…" bind:this={filterElement} bind:value={filter} autofocus />
    <label class="hidden-toggle" title="Show dotfiles (names starting with .)">
      <input type="checkbox" bind:checked={showHidden} />
      Show hidden
    </label>
  </div>

  <div class="pick-content">
    <aside class="shortcuts" aria-label="Quick navigation">
      <section aria-label="Favorites">
        <h4>Favorites</h4>
        <div class="shortcut-list">
          {#each shortcuts.favorites as path (path)}
            <div class="favorite-row">
              <button title={path} disabled={loading} class:current={path === view?.path} onclick={() => load(path)}>{folderName(path)}</button>
              <button class="remove-favorite" aria-label={`Remove ${folderName(path)} from favorites`} title="Remove favorite"
                onclick={() => updateShortcuts(current => toggleFavorite(current, path))}>×</button>
            </div>
          {:else}<p class="dim">Save a folder with Add favorite.</p>{/each}
        </div>
      </section>
      <section aria-label="Recents">
        <h4>Recents</h4>
        <div class="shortcut-list">
          {#each shortcuts.recents as path (path)}
            <button title={path} disabled={loading} class:current={path === view?.path} onclick={() => load(path)}>{folderName(path)}</button>
          {:else}<p class="dim">Folders you open appear here.</p>{/each}
        </div>
      </section>
    </aside>
  <div class="browser">
    {#if loading}
      <div class="dim pad">Loading…</div>
    {:else if error}
      <div class="err pad" role="alert">{error}</div>
      <button class="btn retry" bind:this={retryElement} onclick={() => load(lastAttempt.path, lastAttempt.index)}>Retry</button>
    {:else if view}
      {#if view.parent !== null}
        <button class="row" onclick={() => load(view!.parent!)}>
          <Icon name="folder" size={14} />
          <span class="grow">..</span>
          <span class="dim">up</span>
        </button>
      {/if}
      {#if shown.length > 0}
        <div class="folder-grid" bind:this={gridElement} role="grid" tabindex="0"
          aria-label="Folder entries. Arrow keys choose a row and Open or Use; Enter activates."
          aria-rowcount={shown.length} aria-colcount={2}
          aria-activedescendant={cellId(position.index, position.action)} onkeydown={gridKey}>
          <VirtualList items={shown} estimateHeight={32} pinnedIndex={position.index}
            scrollIndex={position.index} {scrollVersion} tabindex={-1} key={entry => entry.path} class="picker-rows">
            {#snippet row(e: FsEntry, index: number)}
              <div class="row-wrap" role="row" aria-rowindex={index + 1}>
                <div class="entry-cell" role="gridcell" id={cellId(index, 0)} aria-colindex={1} aria-selected={position.index === index && position.action === 0}>
                  <button class="row" class:file-row={!e.is_dir} tabindex="-1" onclick={() => activate(index, 0)}>
                    <Icon name={e.is_dir ? (e.is_git_repo ? 'branch' : 'folder') : 'file'} size={14} />
                    <span class="grow ellipsis">{e.name}</span>
                    {#if e.is_git_repo}<span class="chip">git</span>{/if}
                    {#if !e.is_dir}<span class="dim use-file">select</span>{/if}
                  </button>
                </div>
                {#if gridAction(e, 1, gitOnly)}
                  <div role="gridcell" id={cellId(index, 1)} aria-colindex={2} aria-selected={position.index === index && position.action === 1}>
                    <button class="use" tabindex="-1" title="Use this folder" onclick={() => activate(index, 1)}>Use</button>
                  </div>
                {/if}
              </div>
            {/snippet}
          </VirtualList>
        </div>
      {/if}
      {#if shown.length === 0}
        <div class="dim pad">
          {#if view.entries.length === 0}
            {files ? 'No items here.' : 'No subfolders here.'}
          {:else}
            No matches.
          {/if}
        </div>
      {/if}
    {/if}
  </div>

  </div>

  {#snippet footer()}
    <button class="btn" onclick={closePicker}>Cancel</button>
    <!-- Always selectable when picking a plain folder; in gitOnly mode, selectable
         once you've navigated INTO a git repo (so you're not forced to pick it
         from the parent listing). -->
    {#if !files && (!gitOnly || view?.is_git_repo)}
      <button class="btn primary" disabled={!view || loading || !!error} onclick={() => view && pick(view.path)}>
        {gitOnly ? 'Use this repository' : 'Use this folder'}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .navigation {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .favorite-action { margin-inline-start: auto; }
  .pick-content { display: flex; gap: 10px; min-width: 0; }
  .shortcuts { flex: 0 0 145px; min-width: 0; }
  .shortcuts h4 { font-size: 11px; color: var(--text-dim); margin: 5px 6px; }
  .shortcuts section + section { margin-top: 10px; }
  .shortcut-list { max-height: 130px; overflow-y: auto; }
  .shortcut-list p { font-size: 11px; margin: 6px; }
  .shortcut-list button {
    display: block; width: 100%; padding: 5px 6px; border: 0;
    border-radius: var(--radius-s); background: transparent; color: var(--text);
    text-align: start; font-size: 12px; cursor: pointer;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .favorite-row { display: flex; align-items: center; min-width: 0; }
  .favorite-row > button:first-child { flex: 1; min-width: 0; }
  .shortcut-list .remove-favorite { width: 24px; flex: 0 0 24px; text-align: center; color: var(--text-dim); }
  .shortcut-list button:hover, .shortcut-list button.current { background: var(--surface-2); }
  @media (max-width: 520px) {
    .pick-content { flex-direction: column; }
    .shortcuts { flex: none; display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
    .shortcuts section { min-width: 0; }
    .shortcuts section + section { margin-top: 0; }
    .shortcut-list { max-height: 85px; }
    .pick-content .browser { flex: none; height: 240px; }
  }
  .navigation .btn { padding: 4px 10px; }
  .crumb {
    display: flex;
    align-items: center;
    gap: 3px;
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 2px 2px 10px;
    overflow-x: auto;
    white-space: nowrap;
  }
  .crumb button {
    flex-shrink: 0;
    font: inherit;
    color: inherit;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    padding: 4px;
    cursor: pointer;
  }
  .crumb button:hover { background: var(--surface-2); color: var(--text); }
  .crumb button[aria-current] { color: var(--text); }
  .retry { margin: 0 14px 14px; }
  .pick-tools {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  .filter-input {
    flex: 1;
    min-width: 0;
  }
  .hidden-toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    font-size: 11.5px;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .hidden-toggle input {
    cursor: pointer;
  }
  .browser {
    flex: 1;
    min-width: 0;
    height: 320px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .folder-grid { flex: 1; min-height: 0; outline-offset: -2px; }
  .folder-grid :global(.picker-rows) { height: 100%; }
  .entry-cell { flex: 1; min-width: 0; }
  .entry-cell .row { width: 100%; height: 32px; }
  .folder-grid:focus-within [aria-selected="true"] { background: var(--surface); outline: 1px solid var(--accent); outline-offset: -1px; }
  .browser > .row { flex: none; height: 32px; }
  .row-wrap {
    height: 32px;
    display: flex;
    align-items: center;
  }
  .row {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
    text-align: start;
    min-width: 0;
  }
  .row:hover {
    background: var(--surface);
  }
  .file-row {
    color: var(--text-dim);
  }
  .file-row:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .use-file {
    font-size: 11px;
    flex-shrink: 0;
    margin-inline-end: 4px;
  }
  .use {
    flex-shrink: 0;
    margin-inline-end: 8px;
    padding: 3px 10px;
    font-size: 11px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }
  .use:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .chip {
    font-size: 9.5px;
    padding: 1px 5px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .pad {
    padding: 14px;
    font-size: 12px;
  }
  .err {
    color: var(--danger, #e5534b);
  }
</style>
