<script lang="ts">
  // macOS-style Named Multi-Repo Project & Workspace Switcher for the IDE header.
  // Renders the primary project dropdown, 1-click Quick Project Switcher pills for
  // switching between 6+ multi-repo projects in <50ms, and a clamped switcher popover.
  import { tick } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { ide, ideBasename, type IdeNamedProject } from '../ideStore.svelte';

  const FolderPickerLazy = lazyComponent(() => import('../../../lib/components/FolderPicker.svelte'));

  let menuOpen = $state(false);
  let query = $state('');
  let folderPickerMode = $state<'open' | 'attach' | null>(null);
  let triggerEl = $state<HTMLButtonElement | undefined>(undefined);
  let popEl = $state<HTMLDivElement | undefined>(undefined);
  let searchEl = $state<HTMLInputElement | undefined>(undefined);
  let pos = $state({ left: 12, top: 48, width: 500, maxH: 460 });

  function shortenHomePath(p: string): string {
    if (!p) return '';
    const m = p.match(/^\/Users\/[^/]+(\/.*)?$/);
    if (m) return `~${m[1] || ''}`;
    const mLinux = p.match(/^\/home\/[^/]+(\/.*)?$/);
    if (mLinux) return `~${mLinux[1] || ''}`;
    return p;
  }

  const primaryLabel = $derived.by((): string => {
    if (!ide.hasTarget) return 'Select project';
    if (ide.target.kind === 'ssh') {
      const host = ide.target.connection_name || 'SSH';
      return `${host}:${ideBasename(ide.target.root)}`;
    }
    if (ide.activeProject) return ide.activeProject.name;
    const matchedWs = ws.workspaces.find(
      (w) => w.id !== SCRATCH_WORKSPACE_ID && w.root_path === ide.target.root,
    );
    return matchedWs?.name || ideBasename(ide.target.root) || ide.target.root;
  });

  const subtitlePath = $derived(ide.hasTarget ? shortenHomePath(ide.target.root) : '');
  const extraRootCount = $derived(Math.max(0, ide.projectRoots.length - 1));
  const pinnedProjects = $derived(ide.namedProjects.filter((p) => p.pinned !== false));

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

  function clampPosition(): void {
    if (typeof window === 'undefined' || !triggerEl) return;
    const rect = triggerEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.max(260, Math.min(540, vw - 16));
    const isRtl = document.documentElement.dir === 'rtl';
    const rawLeft = isRtl ? rect.right - width : rect.left;
    const left = Math.max(8, Math.min(rawLeft, vw - width - 8));
    const maxH = Math.max(200, Math.min(520, vh - 16));
    const measuredH = popEl ? Math.min(popEl.offsetHeight || 340, maxH) : 340;
    const top = Math.max(8, Math.min(rect.bottom + 6, vh - measuredH - 8));
    pos = { left, top, width, maxH };
  }

  async function toggleMenu(): Promise<void> {
    menuOpen = !menuOpen;
    if (menuOpen) {
      query = '';
      clampPosition();
      await tick();
      clampPosition();
      searchEl?.focus();
    }
  }

  function closeMenu(): void {
    menuOpen = false;
  }

  $effect(() => {
    if (!menuOpen) return;
    void filteredWorkspaces.length;
    void filteredNamedProjects.length;
    requestAnimationFrame(() => clampPosition());
  });

  $effect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        closeMenu();
        triggerEl?.focus();
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
    closeMenu();
    await ide.setLocalTarget(rootPath, id);
  }

  async function pickNamedProject(proj: IdeNamedProject): Promise<void> {
    closeMenu();
    await ide.activateProject(proj.id);
  }

  function openProjectContextMenu(e: MouseEvent, proj: IdeNamedProject): void {
    e.preventDefault();
    const items: MenuItem[] = [
      {
        label: 'Rename project…',
        icon: 'edit',
        action: () =>
          void (async () => {
            const next = await confirmer.promptText('Project name', {
              title: 'Rename IDE Project',
              initial: proj.name,
              confirmLabel: 'Rename',
            });
            if (next && next.trim()) {
              await ide.saveNamedProject({
                id: proj.id,
                name: next.trim(),
                primary_root: proj.primary_root,
                folders: proj.folders,
                pinned: proj.pinned,
              });
            }
          })(),
      },
      {
        label: 'Attach multiple repositories…',
        icon: 'plus',
        action: () =>
          void (async () => {
            await ide.activateProject(proj.id);
            ide.bulkRepoPickerMode = 'attach';
          })(),
      },
      {
        label: 'Project Structure & SDKs…',
        icon: 'gear',
        action: () =>
          void (async () => {
            await ide.activateProject(proj.id);
            ide.openSettings('project.modules');
          })(),
      },
      {
        label: proj.pinned !== false ? 'Unpin from header bar' : 'Pin in header bar',
        icon: 'pin',
        action: () =>
          void ide.saveNamedProject({
            id: proj.id,
            name: proj.name,
            primary_root: proj.primary_root,
            folders: proj.folders,
            pinned: proj.pinned === false,
          }),
      },
      { separator: true },
      {
        label: 'Delete project…',
        icon: 'trash',
        danger: true,
        action: () =>
          void (async () => {
            const ok = await confirmer.ask(`Remove named project "${proj.name}"? Repository files on disk are untouched.`, {
              title: 'Delete IDE project?',
              confirmLabel: 'Delete',
              danger: true,
            });
            if (ok) await ide.deleteNamedProject(proj.id);
          })(),
      },
    ];
    ctxMenu.show(e, items);
  }

  function startFolderPick(mode: 'open' | 'attach'): void {
    closeMenu();
    folderPickerMode = mode;
  }

  async function handlePickedFolder(picked: string): Promise<void> {
    const mode = folderPickerMode;
    folderPickerMode = null;
    if (!picked) return;
    if (mode === 'attach' && ide.hasTarget) await ide.addProjectFolder(picked);
    else await ide.setLocalTarget(picked);
  }
</script>

<div class="ws-bar-cluster" role="group" aria-label="IDE project switcher">
  <button
    bind:this={triggerEl}
    type="button"
    class="ws-switcher-btn"
    aria-haspopup="dialog"
    aria-expanded={menuOpen}
    title={ide.hasTarget ? `${ide.target.kind === 'ssh' ? 'SSH' : 'Project'}: ${ide.target.root}` : 'Select workspace or project folder'}
    onclick={() => void toggleMenu()}
    data-testid="ide-target-switcher"
  >
    <span class="ws-icon" aria-hidden="true">
      <Icon name={ide.target.kind === 'ssh' ? 'server' : 'folder'} size={13} />
    </span>
    <span class="ws-name" dir="ltr">{primaryLabel}</span>
    {#if ide.projectRoots.length > 1}
      <Badge tone="accent" variant="outline" label={`${ide.projectRoots.length} repos`} />
    {:else if extraRootCount > 0}
      <Badge tone="accent" variant="outline" label={`+${extraRootCount}`} />
    {/if}
    {#if subtitlePath && pinnedProjects.length === 0}
      <span class="ws-path" dir="ltr">{subtitlePath}</span>
    {/if}
    <Icon name="chevronDown" size={12} />
  </button>

  {#if !viewport.isPhone}
    <div class="ws-pills-strip" role="group" aria-label="Pinned IDE projects">
      {#each pinnedProjects as proj (proj.id)}
        {@const isActive = ide.activeProjectId === proj.id}
        <button
          type="button"
          class="ws-proj-pill"
          class:is-active={isActive}
          aria-pressed={isActive}
          title={`${proj.name} (${proj.folders.length} ${proj.folders.length === 1 ? 'repo' : 'repos'}) — right-click for options`}
          onclick={() => void ide.activateProject(proj.id)}
          oncontextmenu={(e) => openProjectContextMenu(e, proj)}
          data-testid="ide-project-pill"
        >
          <span class="ws-pill-name" dir="auto">{proj.name}</span>
          {#if proj.folders.length > 1}
            <span class="ws-pill-cnt">{proj.folders.length}</span>
          {/if}
        </button>
      {/each}
      <button
        type="button"
        class="ws-proj-pill ws-add-proj"
        aria-label="New multi-repo IDE project"
        title="New multi-repo IDE project…"
        onclick={() => (ide.bulkRepoPickerMode = 'create')}
        data-testid="ide-new-project-btn"
      >
        <Icon name="plus" size={12} />
        <span class="ws-add-lbl">Project</span>
      </button>
    </div>
  {/if}
</div>

{#if menuOpen}
  <div class="ws-backdrop" role="presentation" onclick={closeMenu}></div>
  <div
    bind:this={popEl}
    class="ws-popover"
    role="dialog"
    aria-label="Switch workspace or project folder"
    style:inset-inline-start="{pos.left}px"
    style:top="{pos.top}px"
    style:inline-size="{pos.width}px"
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
      {#if filteredNamedProjects.length > 0}
        <div class="ws-sec-label">Named Multi-Repo Projects ({filteredNamedProjects.length})</div>
        {#each filteredNamedProjects as proj (proj.id)}
          {@const active = ide.activeProjectId === proj.id}
          <div class="ws-attached-row" class:is-active={active}>
            <button
              type="button"
              class="ws-proj-select"
              onclick={() => void pickNamedProject(proj)}
              oncontextmenu={(e) => openProjectContextMenu(e, proj)}
              data-testid="ide-named-project-option"
            >
              <span class="ws-item-icon"><Icon name="layers" size={14} /></span>
              <span class="ws-item-copy">
                <span class="ws-item-title" dir="auto">
                  {proj.name}
                  <Badge tone="accent" variant="outline" label={`${proj.folders.length} repos`} />
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
      <button
        type="button"
        class="ws-action-btn"
        onclick={() => { closeMenu(); ide.bulkRepoPickerMode = 'create'; }}
        data-testid="ide-switcher-new-project"
      >
        <Icon name="layers" size={13} />
        <span>New multi-repo project…</span>
      </button>
      {#if ide.hasTarget && ide.target.kind === 'local'}
        <button
          type="button"
          class="ws-action-btn"
          onclick={() => { closeMenu(); ide.bulkRepoPickerMode = 'attach'; }}
          data-testid="ide-switcher-bulk-attach"
        >
          <Icon name="plus" size={13} />
          <span>Bulk attach repos…</span>
        </button>
      {/if}
      <button type="button" class="ws-action-btn" onclick={() => startFolderPick('open')} data-testid="ide-open-local-folder-btn">
        <Icon name="folder" size={13} />
        <span>Open local folder…</span>
      </button>
      {#if ide.hasTarget && ide.target.kind === 'local'}
        <button type="button" class="ws-action-btn" onclick={() => startFolderPick('attach')} data-testid="ide-switcher-add-folder">
          <Icon name="plus" size={13} />
          <span>Add folder…</span>
        </button>
      {/if}
      <button type="button" class="ws-action-btn" onclick={() => { closeMenu(); ide.openSettings('build.ssh'); }} data-testid="ide-switcher-ssh">
        <Icon name="server" size={13} />
        <span>Remote SSH…</span>
      </button>
    </div>
  </div>
{/if}

{#if folderPickerMode}
  <LazyMount
    lazy={FolderPickerLazy}
    what="folder picker"
    quiet
    props={{
      title: folderPickerMode === 'attach' ? 'Add folder to project' : 'Open project folder',
      start: ide.target.kind === 'local' ? ide.target.root : '',
      onpick: (p: string) => void handlePickedFolder(p),
      onclose: () => (folderPickerMode = null),
    }}
    onerror={() => (folderPickerMode = null)}
  />
{/if}

<style>
  .ws-bar-cluster { display: inline-flex; align-items: center; gap: 6px; min-inline-size: 0; block-size: 28px; }
  .ws-switcher-btn {
    display: inline-flex; align-items: center; gap: 6px; max-inline-size: min(380px, 32vw); min-inline-size: 0;
    block-size: 28px; padding-inline: 10px; background: var(--surface-2); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-s); cursor: pointer;
  }
  .ws-switcher-btn:hover { border-color: var(--border-strong); background: var(--hover); }
  .ws-icon { display: inline-flex; color: var(--accent-text); flex-shrink: 0; }
  .ws-name { font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .ws-path { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-inline-size: 160px; }
  .ws-pills-strip {
    display: inline-flex; align-items: center; gap: 4px; max-inline-size: min(440px, 30vw);
    overflow-x: auto; scrollbar-width: none; min-inline-size: 0;
  }
  .ws-pills-strip::-webkit-scrollbar { display: none; }
  .ws-proj-pill {
    display: inline-flex; align-items: center; gap: 4px; block-size: 24px; padding-inline: 8px;
    border-radius: var(--radius-s); border: 1px solid var(--border); background: var(--surface);
    color: var(--text-dim); font-size: var(--fs-xs); font-weight: 500; white-space: nowrap; cursor: pointer; flex-shrink: 0;
  }
  .ws-proj-pill:hover { background: var(--hover); color: var(--text); border-color: var(--border-strong); }
  .ws-proj-pill.is-active { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); font-weight: 600; }
  .ws-pill-name { max-inline-size: 112px; overflow: hidden; text-overflow: ellipsis; }
  .ws-pill-cnt { font-family: var(--font-mono); font-size: var(--fs-xs); opacity: 0.85; }
  .ws-add-proj { border-style: dashed; }
  .ws-backdrop { position: fixed; inset: 0; z-index: var(--z-popover-backdrop); }
  .ws-popover {
    position: fixed; z-index: var(--z-popover); display: flex; flex-direction: column;
    background: var(--surface); border: 1px solid var(--border-strong); border-radius: var(--radius-m); box-shadow: var(--shadow-card); overflow: hidden;
  }
  .ws-search-bar { display: flex; align-items: center; gap: 8px; padding-block: 8px; padding-inline: 12px; border-block-end: 1px solid var(--border); background: var(--surface-2); color: var(--text-dim); flex-shrink: 0; }
  .ws-search-bar:focus-within { border-color: var(--accent); box-shadow: inset 0 -1px 0 var(--accent); }
  .ws-search-in { flex: 1; min-inline-size: 0; border: 0; background: transparent; color: var(--text); font-size: var(--fs-s); outline: none; }
  .ws-scroll { flex: 1; min-block-size: 0; overflow-y: auto; padding: 6px; display: flex; flex-direction: column; gap: 2px; }
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
  .ws-actions-footer { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; padding: 8px; border-block-start: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0; }
  .ws-action-btn {
    display: inline-flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 10px;
    border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); color: var(--text); font-size: var(--fs-xs); font-weight: 500; cursor: pointer;
  }
  .ws-action-btn:hover { background: var(--hover); border-color: var(--border-strong); }
  @media (max-width: 1024px) { .ws-path, .ws-add-lbl { display: none; } .ws-pills-strip { max-inline-size: 220px; } }
  @media (max-width: 640px) { .ws-switcher-btn { max-inline-size: 145px; block-size: 26px; padding-inline: 8px; font-size: var(--fs-xs); } }
</style>
