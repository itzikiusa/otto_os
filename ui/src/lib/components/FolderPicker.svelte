<script lang="ts">
  // Daemon-side folder/file picker. Browses the directory tree on the machine that
  // runs ottod (the path the user picks is used as a session cwd / repo path,
  // so it must exist on the daemon host, not the client). Backed by GET
  // /fs/browse. `gitOnly` highlights git repos for the repo picker.
  // When `files` is true, files are shown and can be picked directly (for
  // identity-file selection etc.); directories still navigate on click.
  import { api } from '../api/client';
  import type { FsBrowse, FsEntry } from '../api/types';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';

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

  async function load(path: string): Promise<void> {
    loading = true;
    error = '';
    filter = ''; // reset the filter on navigate so the new listing isn't pre-filtered
    try {
      let q = path ? `?path=${encodeURIComponent(path)}` : '';
      if (files) {
        q = q ? `${q}&files=true` : '?files=true';
      }
      view = await api.get<FsBrowse>(`/fs/browse${q}`);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load(start);
  });
</script>

<Modal {title} {onclose} width={620}>
  {#if view}
    <div class="crumb mono">{view.path}</div>
  {/if}

  <div class="pick-tools">
    <!-- svelte-ignore a11y_autofocus -->
    <input class="input filter-input" placeholder="Filter…" bind:value={filter} autofocus />
    <label class="hidden-toggle" title="Show dotfiles (names starting with .)">
      <input type="checkbox" bind:checked={showHidden} />
      Show hidden
    </label>
  </div>

  <div class="browser">
    {#if loading}
      <div class="dim pad">Loading…</div>
    {:else if error}
      <div class="err pad">{error}</div>
    {:else if view}
      {#if view.parent !== null}
        <button class="row" onclick={() => load(view!.parent!)}>
          <Icon name="folder" size={14} />
          <span class="grow">..</span>
          <span class="dim">up</span>
        </button>
      {/if}
      {#each shown as e (e.path)}
        {#if e.is_dir}
          <div class="row-wrap">
            <button class="row" onclick={() => load(e.path)}>
              <Icon name={e.is_git_repo ? 'branch' : 'folder'} size={14} />
              <span class="grow ellipsis">{e.name}</span>
              {#if e.is_git_repo}<span class="chip">git</span>{/if}
            </button>
            {#if gitOnly ? e.is_git_repo : true}
              <button class="use" title="Use this folder" onclick={() => onpick(e.path)}>Use</button>
            {/if}
          </div>
        {:else}
          <!-- file row: only rendered when files=true (backend filters) -->
          <div class="row-wrap">
            <button class="row file-row" onclick={() => onpick(e.path)}>
              <Icon name="file" size={14} />
              <span class="grow ellipsis">{e.name}</span>
              <span class="dim use-file">select</span>
            </button>
          </div>
        {/if}
      {/each}
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

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <!-- Always selectable when picking a plain folder; in gitOnly mode, selectable
         once you've navigated INTO a git repo (so you're not forced to pick it
         from the parent listing). -->
    {#if !files && (!gitOnly || view?.is_git_repo)}
      <button class="btn primary" disabled={!view} onclick={() => view && onpick(view.path)}>
        {gitOnly ? 'Use this repository' : 'Use this folder'}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .crumb {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 2px 2px 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
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
    height: 320px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .row-wrap {
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
