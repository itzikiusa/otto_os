<script lang="ts">
  import { shortenHomePath } from '../../../lib/homePath';
  // Lazy-loaded Workspace & Named Multi-Repo Project switcher popover.
  import { tick } from 'svelte';
  import { placeAnchored, type AnchorPlacement } from '../../../lib/anchorPlace';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { plural } from '../../../lib/plural';
  import { rowMenu } from '../../../lib/rowMenu';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { ide, ideBasename, type IdeNamedProject } from '../ideStore.svelte';
  import { onScratches, openScratches } from './ideScratches.svelte';

  interface Props {
    triggerEl: HTMLButtonElement | undefined;
    onclose: () => void;
    onpickfolder: (mode: 'open' | 'attach') => void;
    onprojectcontextmenu: (e: MouseEvent, proj: IdeNamedProject) => void;
  }

  let { triggerEl, onclose, onpickfolder, onprojectcontextmenu }: Props = $props();

  let query = $state('');
  let popEl = $state<HTMLDivElement | undefined>(undefined);
  let searchEl = $state<HTMLInputElement | undefined>(undefined);
  let width = $state(500);
  let pos = $state<AnchorPlacement>({ left: 12, top: 48, maxH: 460 });


  const filteredNamedProjects = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return ide.namedProjects;
    return ide.namedProjects.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        p.primary_root.toLowerCase().includes(q) ||
        p.folders.some((f) => f.toLowerCase().includes(q)),
    );
  });

  const filteredWorkspaces = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = ws.workspaces.filter((w) => w.id !== SCRATCH_WORKSPACE_ID && w.root_path);
    if (!q) return list;
    return list.filter(
      (w) => (w.name || '').toLowerCase().includes(q) || w.root_path.toLowerCase().includes(q),
    );
  });

  function closePopover(restoreFocus = true): void {
    onclose();
    if (restoreFocus) triggerEl?.focus();
  }

  function clampPosition(): void {
    if (typeof window === 'undefined' || !triggerEl) return;
    width = Math.max(260, Math.min(540, window.innerWidth - 16));
    if (popEl) {
      pos = placeAnchored(triggerEl, popEl, { prefer: 'below', align: 'start' });
    }
  }

  $effect(() => {
    clampPosition();
    void tick().then(() => {
      clampPosition();
      searchEl?.focus();
    });
  });

  $effect(() => {
    void filteredWorkspaces.length;
    void filteredNamedProjects.length;
    void tick().then(clampPosition);
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        closePopover(true);
      }
    };
    const onResize = (): void => clampPosition();
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', onResize);
    };
  });

  async function pickWorkspace(rootPath: string, id: string): Promise<void> {
    try {
      await ide.setLocalTarget(rootPath, id);
    } finally {
      closePopover(true);
    }
  }

  async function pickNamedProject(proj: IdeNamedProject): Promise<void> {
    try {
      await ide.activateProject(proj.id);
    } finally {
      closePopover(true);
    }
  }
</script>

<div class="ws-backdrop" role="presentation" onclick={() => closePopover(true)}></div>
<div
  bind:this={popEl}
  class="ws-popover"
  role="dialog"
  aria-label="Switch workspace or project folder"
  style:left="{pos.left}px"
  style:top="{pos.top}px"
  style:inline-size="{width}px"
  style:max-block-size="{pos.maxH}px"
  data-testid="ide-workspace-popover"
>
  <div class="ws-search-bar">
    <Icon name="search" size={13} />
    <input
      bind:this={searchEl}
      class="ws-search-in"
      type="search"
      dir="ltr"
      aria-label="Filter projects, workspaces, and folders"
      placeholder="Search multi-repo projects, workspaces, or paths…"
      bind:value={query}
      data-testid="ide-workspace-search"
    />
  </div>

  <div class="ws-scroll">
    {#if !query.trim() || 'scratches'.includes(query.trim().toLowerCase())}
      <!-- Built in and pinned first: the same scratch folder in every workspace. -->
      <button
        type="button"
        class="ws-item"
        class:is-active={onScratches()}
        onclick={() => void openScratches().finally(() => closePopover(true))}
        data-testid="ide-scratches-option"
      >
        <span class="ws-item-icon"><Icon name="workbench" size={14} /></span>
        <span class="ws-item-copy">
          <span class="ws-item-title">Scratches</span>
          <span class="ws-item-desc">Your scratch files — the same in every workspace</span>
        </span>
        {#if onScratches()}<Badge tone="ok" dot label="Active" />{/if}
      </button>
    {/if}
    <div class="ws-sec-head">
      <span class="ws-sec-label">Multi-repo projects ({filteredNamedProjects.length})</span>
      <button
        type="button"
        class="btn small ghost"
        onclick={() => { onclose(); ide.bulkRepoPickerMode = 'create'; }}
        data-testid="ide-popover-top-new-project"
      >
        <Icon name="plus" size={12} />
        New project…
      </button>
    </div>
    {#if filteredNamedProjects.length === 0}
      <div class="ws-empty">
        {query.trim()
          ? 'No multi-repo projects match this filter.'
          : 'Group multiple repositories (e.g. Go or Java services) into one switchable project.'}
      </div>
    {:else}
      {#each filteredNamedProjects as proj (proj.id)}
        {@const active = ide.activeProjectId === proj.id}
        <div class="ws-attached-row" class:is-active={active}>
          <button
            type="button"
            class="ws-proj-select"
            onclick={() => void pickNamedProject(proj)}
            oncontextmenu={(e) => onprojectcontextmenu(e, proj)}
            use:rowMenu
            data-testid="ide-named-project-option"
          >
            <span class="ws-item-icon"><Icon name="layers" size={14} /></span>
            <span class="ws-item-copy">
              <span class="ws-item-title" dir="auto">
                {proj.name}
                <Badge tone="neutral" variant="outline" label={plural(proj.folders.length, 'repo')} />
              </span>
              <span class="ws-item-sub" dir="ltr">{shortenHomePath(proj.primary_root)}</span>
            </span>
          </button>
          {#if active}<Badge tone="ok" dot label="Active" />{/if}
        </div>
      {/each}
    {/if}

    {#if ide.hasTarget && ide.projectRoots.length > 1}
      <div class="ws-sec-label">Folders in current project ({ide.projectRoots.length})</div>
      {#each ide.projectRoots as root (root.path)}
        <div class="ws-attached-row">
          <Icon name="folder" size={13} />
          <div class="ws-item-copy">
            <span class="ws-item-title" dir="ltr">
              {root.name}
              {#if root.isPrimary}<Badge tone="neutral" variant="outline" label="Primary" />{/if}
            </span>
            <span class="ws-item-sub" dir="ltr">{shortenHomePath(root.path)}</span>
          </div>
          {#if !root.isPrimary}
            <button type="button" class="btn small ghost" onclick={() => void ide.removeProjectFolder(root.path)}>
              Remove
            </button>
          {/if}
        </div>
      {/each}
    {/if}

    <div class="ws-sec-label">Workspaces ({filteredWorkspaces.length})</div>
    {#if filteredWorkspaces.length === 0}
      <div class="ws-empty">
        {query.trim() ? 'No workspaces match this filter.' : 'No registered workspaces yet — open a folder or create a multi-repo project below.'}
      </div>
    {:else}
      {#each filteredWorkspaces as w (w.id)}
        {@const active = !ide.activeProjectId && ide.target.kind === 'local' && ide.target.root === w.root_path}
        <button
          type="button"
          class="ws-item"
          class:is-active={active}
          onclick={() => void pickWorkspace(w.root_path, w.id)}
          data-testid="ide-workspace-option"
        >
          <span class="ws-item-icon"><Icon name="folder" size={14} /></span>
          <span class="ws-item-copy">
            <span class="ws-item-title" dir="ltr">{w.name || ideBasename(w.root_path)}</span>
            <span class="ws-item-sub" dir="ltr">{shortenHomePath(w.root_path)}</span>
          </span>
          {#if active}<Badge tone="ok" dot label="Active" />{/if}
        </button>
      {/each}
    {/if}
  </div>

  <div class="ws-actions-footer">
    {#if ide.hasTarget && ide.target.kind === 'local'}
      <button
        type="button"
        class="ws-action-btn"
        onclick={() => { onclose(); ide.bulkRepoPickerMode = 'attach'; }}
        data-testid="ide-switcher-bulk-attach"
      >
        <Icon name="plus" size={13} />
        <span>Add repos…</span>
      </button>
    {/if}
    <button type="button" class="ws-action-btn" onclick={() => onpickfolder('open')} data-testid="ide-open-local-folder-btn">
      <Icon name="folder" size={13} />
      <span>Open local folder…</span>
    </button>
    {#if ide.hasTarget && ide.target.kind === 'local'}
      <button type="button" class="ws-action-btn" onclick={() => onpickfolder('attach')} data-testid="ide-switcher-add-folder">
        <Icon name="plus" size={13} />
        <span>Add folder…</span>
      </button>
    {/if}
    <button type="button" class="ws-action-btn" onclick={() => { onclose(); ide.openSettings('build.ssh'); }} data-testid="ide-switcher-ssh">
      <Icon name="server" size={13} />
      <span>Remote SSH…</span>
    </button>
  </div>
</div>

<style>
  .ws-backdrop { position: fixed; inset: 0; z-index: var(--z-popover-backdrop); }
  .ws-popover {
    position: fixed; z-index: var(--z-popover); display: flex; flex-direction: column;
    background: var(--surface); border: 1px solid var(--border-strong); border-radius: var(--radius-l); box-shadow: var(--glass-shadow); overflow: hidden;
  }
  .ws-search-bar { display: flex; align-items: center; gap: 8px; padding-block: 8px; padding-inline: 12px; border-block-end: 1px solid var(--border); background: var(--surface-2); color: var(--text-dim); flex-shrink: 0; }
  .ws-search-bar:focus-within { border-color: var(--accent); box-shadow: inset 0 -1px 0 var(--accent); }
  .ws-search-in { flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-s); outline: none; }
  .ws-scroll { flex: 1; min-block-size: 0; overflow-y: auto; padding: 6px; display: flex; flex-direction: column; gap: 2px; }
  .ws-sec-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding-inline-end: 4px; }
  .ws-sec-label { padding-block: 6px 2px; padding-inline: 8px; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .ws-empty { padding: 12px 8px; font-size: var(--fs-s); color: var(--text-dim); }
  .ws-item, .ws-attached-row {
    display: flex; align-items: center; gap: 10px; inline-size: 100%; padding-block: 6px; padding-inline: 8px;
    border: 1px solid transparent; border-radius: var(--radius-s); background: transparent; color: var(--text); text-align: start;
  }
  .ws-proj-select { display: flex; align-items: center; gap: 10px; flex: 1; min-inline-size: 0; border: 0; background: transparent; color: inherit; text-align: start; padding: 0; cursor: pointer; }
  .ws-item { cursor: pointer; }
  .ws-item:hover, .ws-attached-row:hover { background: var(--hover); }
  .ws-item.is-active, .ws-attached-row.is-active { background: var(--accent-soft); border-color: var(--accent-line); }
  .ws-item-icon { display: inline-flex; color: var(--accent-text); flex-shrink: 0; }
  .ws-item-copy { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; gap: 2px; }
  .ws-item-title { display: flex; align-items: center; gap: 6px; font-size: var(--fs-s); font-weight: 600; color: var(--text); word-break: break-word; }
  .ws-item-sub { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ws-item-desc { font-size: var(--fs-xs); color: var(--text-dim); }
  .ws-actions-footer { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; padding: 8px; border-block-start: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; }
  .ws-action-btn {
    display: inline-flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 10px;
    border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); color: var(--text); font-size: var(--fs-xs); font-weight: 500; cursor: pointer;
  }
  .ws-action-btn:hover { background: var(--hover); border-color: var(--border-strong); }
</style>
