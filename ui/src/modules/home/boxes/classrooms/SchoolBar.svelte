<script lang="ts">
  // Otto School — the box's toolbar row (ClassroomsBox): where you are in
  // the school as breadcrumbs (3D) or the head count (List), the 3D / List
  // switch, and in 3D the reset-view and fill-the-page buttons.
  import Icon from '../../../../lib/components/Icon.svelte';
  import { plural } from '../../../../lib/plural';
  import { home } from '../../home.svelte';
  import type { School } from '../../school/model';
  import type { SchoolHandle } from '../../school/scene';

  interface Props {
    boxId: string;
    mode: '3d' | 'list';
    /** WebGL is available (else List is the only view). */
    gl: boolean;
    zoomed: boolean;
    school: School;
    handle: SchoolHandle | null;
    crumbs: { label: string; go?: () => void }[];
    setMode: (m: '3d' | 'list') => void;
  }
  let { boxId, mode, gl, zoomed, school, handle, crumbs, setMode }: Props = $props();
</script>

<div class="bar">
  {#if mode === '3d'}
    <nav class="crumbs" aria-label="Where you are in the school">
      {#each crumbs as c, i (i)}
        {#if i > 0}<Icon name="chevronRight" size={12} />{/if}
        {#if c.go}
          <button class="crumb link" onclick={c.go}>{c.label}</button>
        {:else}
          <span class="crumb ellipsis" aria-current="location">{c.label}</span>
        {/if}
      {/each}
    </nav>
  {:else}
    <span class="sum">{plural(school.rooms.length, 'classroom')} · {plural(school.total, 'kid')}</span>
  {/if}
  {#if gl}
    <div class="segmented" role="group" aria-label="School view">
      <button class="seg" class:active={mode === '3d'} aria-pressed={mode === '3d'} onclick={() => setMode('3d')}>3D</button>
      <button class="seg" class:active={mode === 'list'} aria-pressed={mode === 'list'} onclick={() => setMode('list')}>List</button>
    </div>
  {/if}
  {#if mode === '3d'}
    <button class="icon-btn" onclick={() => handle?.resetView()} disabled={!handle} title="Reset the school view" aria-label="Reset the school view"><Icon name="refresh" size={12} /></button>
    <button class="icon-btn" onclick={() => home.toggleZoom(boxId)} title={zoomed ? 'Exit full page' : 'Fill the page'} aria-label={zoomed ? 'Exit full page' : 'Fill the page'}>
      <Icon name={zoomed ? 'minimize' : 'maximize'} size={12} />
    </button>
  {/if}
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
    min-width: 0;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .crumb {
    min-width: 0;
    max-width: 220px;
  }
  .crumb.link {
    background: none;
    border: 0;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    color: var(--accent-text);
    cursor: pointer;
    font: inherit;
  }
  .crumb.link:hover {
    background: var(--surface-2);
  }
  span.crumb {
    color: var(--text);
    font-weight: 600;
  }
  .sum {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
