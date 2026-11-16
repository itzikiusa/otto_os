<script lang="ts">
  // Dedicated `IDE` -> `📁 <Project Name>` folder tree inside `ThreadSidebar.svelte`.
  // Surfaces Named IDE Projects (`otto.ide.namedProjects.v1`), active IDE roots,
  // and multi-repo IDE projects so every IDE thread appears in one unified place
  // with 1-click `+ New thread` and `Open in IDE` actions.
  import type { Session } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import type { IdeSidebarProject } from './ideProjectModel';
  import { openProjectInIde } from './ideProjects.svelte';
  import type { FamilyRow } from './threadFamilies';
  import { promptNewScript } from './threadGitMenus';
  import ThreadSidebarRow from './ThreadSidebarRow.svelte';
  import { threads } from './threadsStore.svelte';

  export interface OrderedIdeProjectGroup {
    project: IdeSidebarProject;
    items: Session[];
    ordered: FamilyRow[];
  }

  interface Props {
    groups: OrderedIdeProjectGroup[];
    foldedFamilies: Record<string, boolean>;
    ontogglefamily: (id: string) => void;
    onnewthread: (cwd: string) => void;
    onrowclick: (e: MouseEvent, s: Session) => void;
    onreopen: (s: Session, beside: boolean) => void;
    onmenu: (e: MouseEvent, s: Session) => void;
    onselect?: () => void;
  }

  let {
    groups,
    foldedFamilies,
    ontogglefamily,
    onnewthread,
    onrowclick,
    onreopen,
    onmenu,
    onselect,
  }: Props = $props();

  let sectionClosed = $state(false);
  let closedProjects = $state<Record<string, boolean>>({});

  const totalThreads = $derived(groups.reduce((acc, g) => acc + g.items.length, 0));

  function showIdeProjectMenu(e: MouseEvent, project: IdeSidebarProject, firstThreadId: string | null): void {
    e.preventDefault();
    ctxMenu.show(e, [
      {
        label: 'Open in IDE',
        icon: 'ide',
        action: () => openProjectInIde(project.primaryRoot, firstThreadId),
      },
      {
        label: 'New thread here',
        icon: 'edit',
        action: () => onnewthread(project.primaryRoot),
      },
      {
        label: threads.projectFilters.includes(project.primaryRoot) ? 'Remove from filter' : 'Filter by project',
        icon: 'filter',
        action: () => threads.toggleProjectFilter(project.primaryRoot),
      },
      {
        label: 'Add project script…',
        icon: 'play',
        action: () => void promptNewScript(project.primaryRoot),
      },
    ]);
  }
</script>

{#if groups.length > 0}
  <section class="th-ide" aria-label="IDE projects" data-testid="threads-ide-folder">
    <div class="th-ide-head">
      <button
        type="button"
        class="th-ide-toggle"
        aria-expanded={!sectionClosed}
        onclick={() => (sectionClosed = !sectionClosed)}
        data-testid="threads-ide-folder-toggle"
      >
        <Icon name={!sectionClosed ? 'chevronDown' : 'chevronRight'} size={12} />
        <Icon name="ide" size={13} />
        <span class="th-ide-title">IDE</span>
        {#if totalThreads > 0}
          <span class="th-ide-count">{totalThreads}</span>
        {/if}
      </button>
    </div>

    {#if !sectionClosed}
      <div class="th-ide-projects">
        {#each groups as grp (grp.project.id)}
          {@const open = !closedProjects[grp.project.id]}
          {@const firstThreadId = grp.ordered[0]?.session.id ?? null}
          <div class="th-ide-proj" data-testid="threads-ide-project-{grp.project.id}">
            <div class="th-ide-row reveal-host" data-testid="threads-ide-project-row">
              <button
                type="button"
                class="th-ide-proj-toggle"
                use:rowMenu
                aria-expanded={open}
                title={grp.project.folders.join('\n')}
                oncontextmenu={(e) => showIdeProjectMenu(e, grp.project, firstThreadId)}
                onclick={() => (closedProjects = { ...closedProjects, [grp.project.id]: open })}
              >
                <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
                <Icon name="folder" size={13} />
                <span class="ellipsis" dir="auto">{grp.project.name}</span>
                {#if grp.project.metaLabel}
                  <span class="th-ide-meta">{grp.project.metaLabel}</span>
                {/if}
                {#if grp.items.length > 0}
                  <span class="th-ide-count">{grp.items.length}</span>
                {/if}
              </button>

              <button
                type="button"
                class="icon-btn reveal-on-hover"
                aria-label="New thread in {grp.project.name}"
                title="New thread in {grp.project.name}"
                onclick={() => onnewthread(grp.project.primaryRoot)}
                data-testid="threads-ide-new-thread"
              >
                <Icon name="plus" size={12} />
              </button>

              <button
                type="button"
                class="icon-btn th-ide-open-btn"
                aria-label="Open {grp.project.name} in IDE"
                title="Open {grp.project.name} in IDE"
                onclick={() => openProjectInIde(grp.project.primaryRoot, firstThreadId)}
                data-testid="threads-open-in-ide"
              >
                <Icon name="ide" size={12} />
              </button>
            </div>

            {#if open}
              <div class="th-ide-threads">
                {#each grp.ordered as item (item.session.id)}
                  <ThreadSidebarRow
                    session={item.session}
                    variant="active"
                    child={item.child}
                    family={{
                      kids: item.kids,
                      detached: item.detached,
                      folded: foldedFamilies[item.session.id] === true,
                      ontoggle: () => ontogglefamily(item.session.id),
                    }}
                    {onrowclick}
                    {onreopen}
                    {onmenu}
                    {onselect}
                  />
                {:else}
                  <p class="th-ide-empty">No threads yet</p>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .th-ide {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-block-end: 4px;
    border-block-end: 1px solid var(--border);
  }
  .th-ide-head {
    display: flex;
    align-items: center;
  }
  .th-ide-toggle {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 26px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-align: start;
    cursor: pointer;
  }
  .th-ide-toggle:hover {
    background: var(--hover);
    color: var(--text);
  }
  .th-ide-toggle :global(svg) {
    flex-shrink: 0;
  }
  .th-ide-title {
    color: var(--accent-text);
  }
  .th-ide-projects {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-inline-start: 6px;
  }
  .th-ide-proj {
    display: flex;
    flex-direction: column;
  }
  .th-ide-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .th-ide-proj-toggle {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    font-weight: 500;
    text-align: start;
    cursor: pointer;
  }
  .th-ide-proj-toggle :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .th-ide-proj-toggle:hover {
    background: var(--hover);
  }
  .th-ide-meta {
    flex-shrink: 0;
    padding-inline: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 400;
  }
  .th-ide-count {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .th-ide-open-btn {
    color: var(--text-dim);
  }
  .th-ide-open-btn:hover {
    color: var(--accent-text);
  }
  .th-ide-threads {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding-inline-start: 8px;
  }
  .th-ide-empty {
    margin: 0;
    padding-block: 4px;
    padding-inline: 24px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  @media (pointer: coarse) {
    .th-ide-toggle,
    .th-ide-proj-toggle {
      min-block-size: var(--hit-min);
    }
  }
</style>
