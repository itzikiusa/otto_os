<script lang="ts">
  // macOS-style Named Multi-Repo Project & Workspace Switcher for the IDE header.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { ide, ideBasename, type IdeNamedProject } from '../ideStore.svelte';

  const FolderPickerLazy = lazyComponent(() => import('../../../lib/components/FolderPicker.svelte'));
  const IdeWorkspacePopoverLazy = lazyComponent(() => import('./IdeWorkspacePopover.svelte'));

  let menuOpen = $state(false);
  let folderPickerMode = $state<'open' | 'attach' | null>(null);
  let triggerEl = $state<HTMLButtonElement | undefined>(undefined);

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
    menuOpen = false;
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
    onclick={() => (menuOpen = !menuOpen)}
    onpointerenter={() => IdeWorkspacePopoverLazy.prefetch()}
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
          use:rowMenu
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
  <LazyMount
    lazy={IdeWorkspacePopoverLazy}
    what="project switcher"
    quiet
    props={{
      triggerEl,
      onclose: () => (menuOpen = false),
      onpickfolder: startFolderPick,
      onprojectcontextmenu: openProjectContextMenu,
    }}
    onerror={() => (menuOpen = false)}
  />
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
  @media (max-width: 1024px) { .ws-path, .ws-add-lbl { display: none; } .ws-pills-strip { max-inline-size: 220px; } }
  @media (max-width: 640px) { .ws-switcher-btn { max-inline-size: 145px; block-size: 26px; padding-inline: 8px; font-size: var(--fs-xs); } }
</style>
