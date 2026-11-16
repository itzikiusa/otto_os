<script lang="ts">
  import { shortenHomePath } from '../../../lib/homePath';
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

  const IdeWorkspacePopoverLazy = lazyComponent(() => import('./IdeWorkspacePopover.svelte'));

  interface Props {
    mode?: 'switcher' | 'pills-bar';
    onpickfolder?: (mode: 'open' | 'attach') => void;
  }

  let { mode = 'switcher', onpickfolder }: Props = $props();

  let menuOpen = $state(false);
  let triggerEl = $state<HTMLButtonElement | undefined>(undefined);


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

  function startFolderPick(pickMode: 'open' | 'attach'): void {
    menuOpen = false;
    onpickfolder?.(pickMode);
  }
</script>

{#if mode === 'switcher'}
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
      {:else if subtitlePath && pinnedProjects.length === 0}
        <span class="ws-path" dir="ltr">{subtitlePath}</span>
      {/if}
      <Icon name="chevronDown" size={12} />
    </button>
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
{:else if !viewport.isPhone && pinnedProjects.length > 0}
  <div class="ws-pills-bar" role="group" aria-label="Pinned IDE projects">
    <div class="ws-pills-strip">
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
  </div>
{/if}

<style>
  .ws-bar-cluster { display: inline-flex; align-items: center; gap: 6px; min-inline-size: 0; max-inline-size: 100%; block-size: 28px; }
  .ws-switcher-btn {
    display: inline-flex; align-items: center; gap: 6px; max-inline-size: min(230px, 22vw); min-inline-size: 0;
    block-size: 28px; padding-inline: 10px; background: var(--surface-2); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-s); cursor: pointer;
  }
  .ws-switcher-btn:hover { border-color: var(--border-strong); background: var(--hover); }
  .ws-icon { display: inline-flex; color: var(--accent-text); flex-shrink: 0; }
  .ws-name { font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .ws-path { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-inline-size: 90px; }
  .ws-pills-bar {
    display: flex; align-items: center; padding-block: 4px; padding-inline: 16px;
    background: var(--surface); border-block-end: 1px solid var(--border); flex-shrink: 0; min-inline-size: 0;
  }
  .ws-pills-strip {
    display: flex; align-items: center; gap: 6px; inline-size: 100%;
    overflow-x: auto; scrollbar-width: none; min-inline-size: 0;
  }
  .ws-pills-strip::-webkit-scrollbar { display: none; }
  .ws-proj-pill {
    display: inline-flex; align-items: center; gap: 4px; block-size: 24px; padding-inline: 8px;
    border-radius: var(--radius-s); border: 1px solid var(--border); background: var(--surface-2);
    color: var(--text-dim); font-size: var(--fs-xs); font-weight: 500; white-space: nowrap; cursor: pointer; flex-shrink: 0;
  }
  .ws-proj-pill:hover { background: var(--hover); color: var(--text); border-color: var(--border-strong); }
  .ws-proj-pill.is-active { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); font-weight: 600; }
  .ws-pill-name { max-inline-size: 140px; overflow: hidden; text-overflow: ellipsis; }
  .ws-pill-cnt { font-family: var(--font-mono); font-size: var(--fs-xs); opacity: 0.85; }
  .ws-add-proj { border-style: dashed; background: transparent; }
  @media (max-width: 1024px) { .ws-path, .ws-add-lbl { display: none; } .ws-switcher-btn { max-inline-size: 175px; } }
  @media (max-width: 640px) { .ws-switcher-btn { max-inline-size: 145px; block-size: 26px; padding-inline: 8px; font-size: var(--fs-xs); } }
</style>
