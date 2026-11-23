<script lang="ts">
  import { shortenHomePath } from '../../../lib/homePath';
  // macOS-style Named Multi-Repo Project & Workspace Switcher for the IDE header.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
  import { tabKeys } from '../../../lib/tabKeys';
  import { ide, ideBasename, type IdeNamedProject } from '../ideStore.svelte';
  import { projectTabs, scratchesMenuItems, type ProjectTab } from './ideProjectList';
  import { onScratches, openScratches } from './ideScratches.svelte';
  import { ideFolderLabel } from './ideProjectRecents.svelte';

  const IdeWorkspacePopoverLazy = lazyComponent(() => import('./IdeWorkspacePopover.svelte'));

  // `switcher`: one button opening the popover. `tabs` (Settings › Appearance ›
  // "Show projects as tabs", wide layouts only): every project is a tab in the
  // header row, Scratches first, and a trailing ▾ opens the same popover for
  // workspaces, folders, search and New project.
  interface Props {
    mode?: 'switcher' | 'tabs';
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
    if (onScratches()) return 'Scratches';
    if (ide.activeProject) return ide.activeProject.name;
    return ideFolderLabel(ide.target.root, ws.workspaces.filter((w) => w.id !== SCRATCH_WORKSPACE_ID));
  });

  const tabs = $derived(projectTabs(ide.namedProjects, ide.activeProjectId, onScratches()));
  // A plain folder or SSH target is no tab: the ▾ names it instead.
  const offTabs = $derived(ide.hasTarget && !tabs.some((t) => t.active));
  let stripEl = $state<HTMLDivElement | undefined>(undefined);

  // Keep the active tab in view (a switch from the popover, or a long strip).
  $effect(() => {
    const id = tabs.find((t) => t.active)?.id;
    if (!id || !stripEl) return;
    stripEl.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });

  async function pickTab(e: MouseEvent, tab: ProjectTab): Promise<void> {
    if (tab.active) return;
    const btn = e.currentTarget as HTMLButtonElement;
    // Both refuse while unsaved edits are kept (the discard prompt).
    if (tab.id === 'scratches') {
      if (!(await openScratches())) return;
    } else {
      await ide.activateProject(tab.id);
      if (ide.activeProjectId !== tab.id) return;
    }
    btn.focus({ preventScroll: true });
  }

  function tabMenu(e: MouseEvent, tab: ProjectTab): void {
    const proj = ide.namedProjects.find((p) => p.id === tab.id);
    if (proj) openProjectContextMenu(e, proj);
    else ctxMenu.show(e, scratchesMenuItems());
  }

  const subtitlePath = $derived(ide.hasTarget ? shortenHomePath(ide.target.root) : '');

  function openProjectContextMenu(e: MouseEvent, proj: IdeNamedProject): void {
    e.preventDefault();
    // Every item opens a dialog or switches project: close the popover first, or
    // its backdrop sits above the dialog and eats the first click.
    menuOpen = false;
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
            if (ide.activeProjectId !== proj.id) return;
            ide.bulkRepoPickerMode = 'attach';
          })(),
      },
      {
        label: 'Project Structure & SDKs…',
        icon: 'gear',
        action: () =>
          void (async () => {
            await ide.activateProject(proj.id);
            if (ide.activeProjectId !== proj.id) return;
            ide.openSettings('project.modules');
          })(),
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
            if (!ok) return;
            const wasActive = ide.activeProjectId === proj.id;
            await ide.deleteNamedProject(proj.id);
            // The open project is gone: land on the default project.
            if (wasActive && !ide.namedProjects.some((p) => p.id === proj.id)) await openScratches();
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

{#if mode === 'tabs'}
  <div class="ws-tabs-cluster" data-testid="ide-project-tabs">
    <div bind:this={stripEl} class="ws-tabs-strip segmented" role="tablist" aria-label="IDE projects" tabindex="-1" onkeydown={tabKeys({ activate: 'click' })}>
      {#each tabs as tab (tab.id)}
        <button
          type="button"
          role="tab"
          aria-selected={tab.active}
          tabindex={tab.active || (!tabs.some((t) => t.active) && tab.id === 'scratches') ? 0 : -1}
          title={tab.deletable ? `${tab.label} — right-click for options` : 'Scratches — the default project'}
          onclick={(e) => void pickTab(e, tab)}
          oncontextmenu={(e) => { e.preventDefault(); tabMenu(e, tab); }}
          data-tab-id={tab.id}
          data-testid={tab.deletable ? 'ide-project-tab' : 'ide-project-tab-scratches'}
        >
          {#if !tab.deletable}<Icon name="workbench" size={12} />{/if}
          <span class="ws-tab-name" dir="auto">{tab.label}</span>
        </button>
      {/each}
    </div>
    <button
      bind:this={triggerEl}
      type="button"
      class="ws-switcher-btn"
      class:ws-more={!offTabs}
      aria-haspopup="dialog"
      aria-expanded={menuOpen}
      aria-label={offTabs ? `Workspaces and projects — ${primaryLabel}` : 'Workspaces, folders and new project'}
      title={offTabs ? `${ide.target.kind === 'ssh' ? 'SSH' : 'Folder'}: ${ide.target.root}` : 'Workspaces, folders and new project'}
      onclick={() => (menuOpen = !menuOpen)}
      onpointerenter={() => IdeWorkspacePopoverLazy.prefetch()}
      data-testid="ide-target-switcher"
    >
      {#if offTabs}
        <span class="ws-icon" aria-hidden="true"><Icon name={ide.target.kind === 'ssh' ? 'server' : 'folder'} size={13} /></span>
        <span class="ws-name" dir="ltr">{primaryLabel}</span>
      {/if}
      <Icon name="chevronDown" size={12} />
    </button>
  </div>
{:else}
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
        <Icon name={ide.target.kind === 'ssh' ? 'server' : onScratches() ? 'workbench' : 'folder'} size={13} />
      </span>
      <span class="ws-name" dir="ltr">{primaryLabel}</span>
      {#if ide.projectRoots.length > 1}
        <Badge tone="accent" variant="outline" label={`${ide.projectRoots.length} repos`} />
      {:else if subtitlePath}
        <span class="ws-path" dir="ltr">{subtitlePath}</span>
      {/if}
      <Icon name="chevronDown" size={12} />
    </button>
  </div>

{/if}
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

<style>
  .ws-bar-cluster { display: inline-flex; align-items: center; gap: 6px; min-inline-size: 0; max-inline-size: 100%; block-size: 28px; }
  .ws-switcher-btn {
    display: inline-flex; align-items: center; gap: 6px; max-inline-size: min(230px, 22vw); min-inline-size: 0;
    block-size: 28px; padding-inline: 10px; background: var(--surface-2); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-s); cursor: pointer;
  }
  /* Tabs mode: the strip scrolls; the ▾ after it stays put. */
  .ws-tabs-cluster { display: flex; align-items: center; gap: 6px; min-inline-size: 0; max-inline-size: 100%; }
  .ws-tabs-strip { flex: 0 1 auto; min-inline-size: 0; overflow-x: auto; scrollbar-width: none; }
  .ws-tabs-strip::-webkit-scrollbar { display: none; }
  .ws-tabs-strip > button { display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0; white-space: nowrap; }
  .ws-tab-name { max-inline-size: 160px; overflow: hidden; text-overflow: ellipsis; }
  .ws-tabs-cluster > .ws-switcher-btn { flex-shrink: 0; }
  .ws-switcher-btn.ws-more { padding-inline: 6px; }
  .ws-switcher-btn:hover { border-color: var(--border-strong); background: var(--hover); }
  .ws-icon { display: inline-flex; color: var(--accent-text); flex-shrink: 0; }
  .ws-name { font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  .ws-path { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-inline-size: 90px; }
  @media (max-width: 1024px) { .ws-path { display: none; } .ws-switcher-btn { max-inline-size: 175px; } }
  @media (max-width: 640px) { .ws-switcher-btn :global(.badge) { display: none; } .ws-switcher-btn { max-inline-size: min(145px, 34vw); block-size: 26px; padding-inline: 8px; font-size: var(--fs-xs); } }
</style>
