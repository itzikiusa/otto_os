<script lang="ts">
  // The sidebar's "Group by project" view (filter menu): IDE projects, then a
  // folder per project with its threads in inbox order (pinned projects
  // first), each foldable, with "New thread here" and the project's menu.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import type { Session } from '../../lib/api/types';
  import type { FamilyRow } from './threadFamilies';
  import type { OrderedIdeProjectGroup } from './ThreadIdeProjectsSection.svelte';
  import ThreadGroupNote from './ThreadGroupNote.svelte';
  import { IdeProjectsSectionView } from './threadLazy.svelte';
  import { threads } from './threadsStore.svelte';

  interface Group {
    key: string;
    label: string;
    cwd: string;
    items: Session[];
    ordered: FamilyRow[];
  }
  interface Props {
    ideGroups: OrderedIdeProjectGroup[];
    groups: Group[];
    foldedFamilies: Record<string, boolean>;
    rows: Snippet<[FamilyRow[]]>;
    ontogglefamily: (id: string) => void;
    onnewthread: (cwd: string) => void;
    onprojectmenu: (e: MouseEvent, grp: Group) => void;
    onrowclick: (e: MouseEvent, s: Session) => void;
    onreopen: (s: Session, beside: boolean) => void;
    onmenu: (e: MouseEvent, s: Session) => void;
    onselect?: () => void;
  }
  let { ideGroups, groups, foldedFamilies, rows, ontogglefamily, onnewthread, onprojectmenu, onrowclick, onreopen, onmenu, onselect }: Props = $props();

  let closedFolders = $state<Record<string, boolean>>({});
</script>

  <div data-testid="threads-active-list">
    {#if ideGroups.length > 0}
      <LazyMount
        lazy={IdeProjectsSectionView}
        what="IDE projects"
        variant="compact"
        props={{
          groups: ideGroups,
          foldedFamilies,
          ontogglefamily,
          onnewthread,
          onrowclick,
          onreopen,
          onmenu,
          onselect,
        }}
      />
    {/if}
    {#each groups as grp (grp.key)}
      {@const open = !closedFolders[grp.key]}
      <section class="th-group" aria-label="Project {grp.label}">
        <div class="th-proj reveal-host">
          <button
            type="button"
            class="th-proj-toggle"
            aria-expanded={open}
            title={grp.cwd}
            oncontextmenu={(e) => onprojectmenu(e, grp)}
            onclick={() => (closedFolders = { ...closedFolders, [grp.key]: open })}
          >
            <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
            <span class="ellipsis" dir="auto">{grp.label}</span>
            {#if threads.isProjectPinned(grp.key)}
              <span class="th-pinned-mark" aria-label="Pinned" title="Pinned project"><Icon name="star" size={12} /></span>
            {/if}
            {#if !open}<ThreadGroupNote items={grp.items} testid="threads-project-note-{grp.key}" />{/if}
          </button>
          <button
            type="button"
            class="icon-btn reveal-on-hover"
            aria-label="New thread in {grp.label}"
            title="New thread in {grp.label}"
            onclick={() => onnewthread(grp.cwd)}
          >
            <Icon name="plus" size={12} />
          </button>
        </div>
        {#if open}
          <div class="th-list">
            {@render rows(grp.ordered)}
            {#if grp.ordered.length === 0}
              <p class="th-empty-row">No threads yet <button type="button" class="btn small ghost" data-testid="threads-empty-new" onclick={() => onnewthread(grp.cwd)}>New thread</button></p>
            {/if}
          </div>
        {/if}
      </section>
    {/each}
  </div>

<style>
  .th-group {
    display: flex;
    flex-direction: column;
  }
  .th-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }
  .th-empty-row {
    margin: 0;
    padding-block: var(--sp-2);
    padding-inline: var(--sp-8) var(--sp-4);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .th-proj {
    display: flex;
    align-items: center;
  }
  .th-proj-toggle {
    flex: 1;
    min-width: 0;
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
  .th-proj-toggle :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .th-proj-toggle:hover {
    background: var(--hover);
  }
  .th-pinned-mark {
    display: inline-flex;
    color: var(--text-dim);
  }
</style>
