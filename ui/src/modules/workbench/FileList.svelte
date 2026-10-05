<script lang="ts">
  // Workbench file list: search, pinned-first live docs, and the trash view
  // (Restore / Delete forever). Every row has a context menu (ctxMenu).
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { WorkbenchDoc } from '../../lib/api/types';
  import { workbench, sortDocs } from './workbench.svelte';

  interface Props {
    onopen: (id: string) => void;
    onnew: () => void;
    onrename: (id: string) => void;
  }
  let { onopen, onnew, onrename }: Props = $props();

  let query = $state('');

  function matches(d: WorkbenchDoc, q: string): boolean {
    if (!q) return true;
    const hay = `${d.name} ${d.folder} ${d.tags.join(' ')} ${d.language}`.toLowerCase();
    return q
      .toLowerCase()
      .split(/\s+/)
      .filter(Boolean)
      .every((t) => hay.includes(t));
  }

  const live = $derived(sortDocs(workbench.docs).filter((d) => matches(d, query.trim())));
  const pinned = $derived(live.filter((d) => d.pinned));
  const rest = $derived(live.filter((d) => !d.pinned));
  const trashed = $derived(workbench.trash.filter((d) => matches(d, query.trim())));

  function iconFor(lang: string): IconName {
    if (lang === 'image') return 'image';
    if (lang === 'md') return 'note';
    if (lang === 'sql') return 'db';
    return 'file';
  }

  function sizeLabel(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  async function act(label: string, fn: () => Promise<unknown>): Promise<void> {
    try {
      await fn();
    } catch (e) {
      toasts.error(`Couldn’t ${label}`, loadErrorText(e));
    }
  }

  function rowMenu(e: MouseEvent, d: WorkbenchDoc): void {
    e.preventDefault();
    const items: MenuItem[] = [
      { label: 'Open', icon: 'file', action: () => onopen(d.id) },
      { label: 'Rename…', icon: 'edit', action: () => onrename(d.id) },
      {
        label: d.pinned ? 'Unpin' : 'Pin to top',
        icon: 'pin',
        action: () => void act(d.pinned ? 'unpin' : 'pin', () => workbench.patchMeta(d.id, { pinned: !d.pinned })),
      },
      { label: 'Duplicate', icon: 'copy', action: () => void act('duplicate', () => workbench.duplicate(d.id)) },
      { separator: true },
      {
        label: 'Move to trash',
        icon: 'trash',
        danger: true,
        action: () =>
          void act('move to trash', async () => {
            await workbench.moveToTrash(d.id);
            toasts.success('Moved to trash', `“${d.name}” and its history can be restored from the trash.`);
          }),
      },
    ];
    ctxMenu.show(e, items);
  }

  function trashMenu(e: MouseEvent, d: WorkbenchDoc): void {
    e.preventDefault();
    ctxMenu.show(e, [
      { label: 'Restore', icon: 'undo', action: () => void restore(d) },
      { separator: true },
      { label: 'Delete forever…', icon: 'trash', danger: true, action: () => void purge(d) },
    ]);
  }

  async function restore(d: WorkbenchDoc): Promise<void> {
    await act('restore', async () => {
      await workbench.restoreFromTrash(d.id);
      toasts.success('Restored', `“${d.name}” is back with its full history.`);
    });
  }

  async function purge(d: WorkbenchDoc): Promise<void> {
    const ok = await confirmer.ask(
      `“${d.name}” and its entire edit history will be erased. This cannot be undone.`,
      { title: 'Delete forever?', confirmLabel: 'Delete forever', danger: true },
    );
    if (!ok) return;
    await act('delete', () => workbench.purge(d.id));
  }

  function toggleTrash(): void {
    workbench.showTrash = !workbench.showTrash;
    if (workbench.showTrash) void workbench.loadTrash();
  }
</script>

{#snippet row(d: WorkbenchDoc)}
  <li>
    <button
      class="wb-row"
      class:active={workbench.active === d.id}
      data-testid="wb-file-row"
      data-doc-id={d.id}
      title={d.folder ? `${d.folder}/${d.name}` : d.name}
      onclick={() => onopen(d.id)}
      oncontextmenu={(e) => rowMenu(e, d)}
    >
      <Icon name={iconFor(d.language)} size={13} />
      <span class="wb-name">{d.name}</span>
      {#if workbench.isDirty(d.id)}<span class="wb-dot" title="Unsaved changes" aria-label="Unsaved changes"></span>{/if}
      <span class="wb-when"><RelTime iso={d.updated_at} /></span>
    </button>
  </li>
{/snippet}

<div class="wb-files">
  <div class="wb-files-head">
    <label class="wb-search">
      <Icon name="search" size={12} />
      <input
        type="search"
        placeholder={workbench.showTrash ? 'Search trash' : 'Search files'}
        aria-label={workbench.showTrash ? 'Search trash' : 'Search files'}
        data-testid="wb-search"
        bind:value={query}
      />
    </label>
    <button class="icon-btn" onclick={onnew} aria-label="New file" title="New file (also in the toolbar)">
      <Icon name="plus" size={13} />
    </button>
  </div>

  <div class="wb-files-body">
    {#if workbench.showTrash}
      <LoadState
        what="trash"
        variant="compact"
        loading={workbench.trashLoading}
        error={workbench.trashError}
        empty={workbench.trash.length === 0}
        onretry={() => void workbench.loadTrash()}
      >
        {#snippet emptyView()}
          <p class="wb-hint">Trash is empty. Files you move to the trash keep their full history until you delete them forever.</p>
        {/snippet}
        <ul class="wb-list" aria-label="Trash">
          {#each trashed as d (d.id)}
            <li class="wb-trash-row" data-testid="wb-trash-row" oncontextmenu={(e) => trashMenu(e, d)}>
              <span class="wb-name" title={d.name}>{d.name}</span>
              <span class="wb-when">{sizeLabel(d.size)}</span>
              <button class="btn small" onclick={() => void restore(d)} title="Restore with its full history">Restore</button>
              <button class="icon-btn wb-purge" onclick={() => void purge(d)} aria-label={`Delete ${d.name} forever`} title="Delete forever">
                <Icon name="trash" size={12} />
              </button>
            </li>
          {/each}
          {#if trashed.length === 0 && workbench.trash.length > 0}<li class="wb-hint">No matches.</li>{/if}
        </ul>
      </LoadState>
    {:else}
      <LoadState
        what="files"
        variant="compact"
        loading={workbench.loading && !workbench.loaded}
        error={workbench.error}
        empty={workbench.docs.length === 0}
        onretry={() => void workbench.loadList()}
      >
        {#snippet emptyView()}
          <EmptyState
            icon="workbench"
            title="No scratch files yet"
            body="Scripts, JSON, notes, diagrams — every save is kept in history."
            actionLabel="New file"
            actionIcon="plus"
            actionKind="secondary"
            onaction={onnew}
          />
        {/snippet}
        {#if pinned.length > 0}
          <h4 class="wb-sec">Pinned</h4>
          <ul class="wb-list" aria-label="Pinned files">
            {#each pinned as d (d.id)}{@render row(d)}{/each}
          </ul>
        {/if}
        {#if rest.length > 0}
          {#if pinned.length > 0}<h4 class="wb-sec">Files</h4>{/if}
          <ul class="wb-list" aria-label="Files">
            {#each rest as d (d.id)}{@render row(d)}{/each}
          </ul>
        {/if}
        {#if live.length === 0}<p class="wb-hint">No files match “{query}”.</p>{/if}
      </LoadState>
    {/if}
  </div>

  <button class="wb-trash-toggle" class:on={workbench.showTrash} onclick={toggleTrash} data-testid="wb-trash-toggle" aria-pressed={workbench.showTrash}>
    <Icon name={workbench.showTrash ? 'chevronLeft' : 'trash'} size={12} />
    {workbench.showTrash ? 'Back to files' : 'Trash'}
  </button>
</div>

<style>
  .wb-files {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .wb-files-head {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 8px;
    border-block-end: 1px solid var(--border);
  }
  .wb-search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    min-width: 0;
  }
  .wb-search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    padding-block: 4px;
    outline: none;
  }
  .wb-search:focus-within {
    border-color: var(--accent-text); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent)
  }
  .wb-files-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px;
  }
  .wb-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .wb-sec {
    margin: 8px 8px 2px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  .wb-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .wb-row:hover {
    background: var(--hover);
  }
  .wb-row.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .wb-row:focus-visible,
  .wb-trash-toggle:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .wb-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wb-when {
    flex: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .wb-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--warning);
  }
  .wb-trash-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    font-size: var(--fs-s);
  }
  .wb-purge:hover {
    color: var(--danger);
  }
  .wb-hint {
    margin: 12px 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .wb-trash-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 8px 12px;
    border: 0;
    border-block-start: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
  }
  .wb-trash-toggle:hover,
  .wb-trash-toggle.on {
    color: var(--text);
    background: var(--hover);
  }
</style>
