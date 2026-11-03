<script lang="ts">
  // The sidebar's project filter (T3): "Search projects…", All projects, then
  // each project with a check (multi-select), a pin star and a ⚙ for the
  // project's own menu. A clamped popover under the filter button.
  import { onMount, tick } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { placeAnchored, type AnchorPlacement } from '../../lib/anchorPlace';
  import { threads } from './threadsStore.svelte';
  import { compactPath } from './threadsHelpers';

  interface Project {
    key: string;
    label: string;
    cwd: string;
  }
  interface Props {
    anchor: HTMLElement;
    projects: readonly Project[];
    homePath: string | null;
    onclose: () => void;
    onsettings: (e: MouseEvent, p: Project) => void;
  }
  let { anchor, projects, homePath, onclose, onsettings }: Props = $props();

  let query = $state('');
  let panelEl = $state<HTMLDivElement | null>(null);
  let searchEl = $state<HTMLInputElement | null>(null);
  let pos = $state<AnchorPlacement>({ top: -9999, left: 0, maxH: 420 });

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = q ? projects.filter((p) => `${p.label} ${p.cwd}`.toLowerCase().includes(q)) : [...projects];
    return list.sort((a, b) => Number(threads.isProjectPinned(b.key)) - Number(threads.isProjectPinned(a.key)));
  });

  function place(): void {
    if (panelEl) pos = placeAnchored(anchor, panelEl, { prefer: 'below', align: 'end' });
  }
  onMount(() => {
    void tick().then(() => {
      place();
      searchEl?.focus();
    });
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  });
  $effect(() => {
    void shown.length;
    void tick().then(place);
  });

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
      anchor.focus();
    }
  }
</script>

<button type="button" class="pf-catch" aria-label="Close project filter" title="Close" tabindex="-1" onclick={onclose}></button>
<div
  bind:this={panelEl}
  class="pf-panel"
  role="dialog"
  aria-label="Filter by project"
  tabindex="-1"
  style:top="{pos.top}px"
  style:left="{pos.left}px"
  style:max-height="{pos.maxH}px"
  data-testid="threads-project-filter-popover"
  onkeydown={onKey}
>
  <div class="pf-search">
    <Icon name="search" size={13} />
    <input bind:this={searchEl} bind:value={query} type="search" dir="auto" placeholder="Search projects…" aria-label="Search projects" />
  </div>
  <div class="pf-list">
    <button
      type="button"
      class="pf-row"
      aria-pressed={threads.projectFilters.length === 0}
      onclick={() => (threads.projectFilters = [])}
    >
      <span class="pf-check">{#if threads.projectFilters.length === 0}<Icon name="check" size={12} />{/if}</span>
      <span class="pf-name">All projects</span>
    </button>
    {#each shown as p (p.key)}
      {@const on = threads.projectFilters.includes(p.key)}
      {@const pinned = threads.isProjectPinned(p.key)}
      <div class="pf-rowwrap reveal-host">
        <button type="button" class="pf-row" aria-pressed={on} title={p.cwd} onclick={() => threads.toggleProjectFilter(p.key)}>
          <span class="pf-check">{#if on}<Icon name="check" size={12} />{/if}</span>
          <span class="pf-name ellipsis" dir="auto">{p.label}</span>
          <span class="pf-path mono ellipsis" dir="ltr">{compactPath(p.cwd, homePath)}</span>
        </button>
        <button
          type="button"
          class="icon-btn pf-pin"
          class:on={pinned}
          class:reveal-on-hover={!pinned}
          aria-pressed={pinned}
          aria-label="{pinned ? 'Unpin' : 'Pin'} {p.label}"
          title={pinned ? 'Unpin project' : 'Pin project to the top'}
          onclick={() => threads.toggleProjectPin(p.key)}
        >
          <Icon name="star" size={12} />
        </button>
        <button
          type="button"
          class="icon-btn reveal-on-hover"
          aria-haspopup="menu"
          aria-label="{p.label} project options"
          title="Project options"
          onclick={(e) => onsettings(e, p)}
        >
          <Icon name="gear" size={12} />
        </button>
      </div>
    {:else}
      <p class="pf-empty">No projects match.</p>
    {/each}
  </div>
</div>

<style>
  .pf-catch {
    position: fixed;
    inset: 0;
    z-index: var(--z-popover-backdrop);
    border: 0;
    padding: 0;
    background: transparent;
    cursor: default;
  }
  .pf-panel {
    position: fixed;
    z-index: var(--z-popover);
    inline-size: min(320px, calc(100vw - 16px));
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    box-shadow: var(--glass-shadow);
  }
  .pf-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-block-end: 1px solid var(--border);
    color: var(--text-dim);
  }
  .pf-search:focus-within {
    color: var(--text);
    box-shadow: inset 0 -1px 0 0 var(--accent-text);
  }
  .pf-search input {
    flex: 1;
    min-width: 0;
    block-size: 26px;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    outline: none;
  }
  .pf-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px;
  }
  .pf-rowwrap {
    display: flex;
    align-items: center;
    border-radius: var(--radius-m);
  }
  .pf-rowwrap:hover {
    background: var(--hover);
  }
  .pf-row {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 30px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .pf-check {
    inline-size: 14px;
    display: inline-flex;
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .pf-name {
    min-width: 0;
  }
  .pf-path {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-align: end;
  }
  .pf-pin.on {
    color: var(--warning);
  }
  .pf-pin.on :global(svg path) {
    fill: currentColor;
  }
  .pf-empty {
    margin: 0;
    padding: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
