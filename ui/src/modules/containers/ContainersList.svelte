<script lang="ts">
  // The Containers tab's list pane (the shared ListPane inside ContainersPage's
  // ListDetail): "Containers N · Run · ⋯", the search row, the state filter
  // row, Compose projects as folding sections with their stack Start / Restart
  // / Stop, rows with state word, health, image and ports, and the count in
  // the footer. Rows draw their own three lines with the global .side-row
  // classes (one look, one keyboard walk).
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideSection from '../../lib/components/side/SideSection.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { plural } from '../../lib/plural';
  import type { ContainerSummary } from '../../lib/api/types/containers';
  import {
    composeServiceOf,
    containerHealthInfo,
    containerStateInfo,
    filterContainers,
    formatPortsSummary,
    groupContainersByProject,
    type ContainerGroup,
    type ContainerStateFilter,
  } from './containers-util';

  interface Props {
    containers: readonly ContainerSummary[];
    selectedId: string | null;
    busyIds: ReadonlySet<string>;
    busyStacks: ReadonlySet<string>;
    onselect: (id: string) => void;
    onaction: (c: ContainerSummary, action: 'start' | 'stop' | 'restart' | 'pause' | 'unpause' | 'remove') => void;
    onstackaction: (project: string, action: 'start' | 'stop' | 'restart') => void;
    onopenrun: () => void;
    onrefresh: () => void;
  }

  let {
    containers,
    selectedId,
    busyIds,
    busyStacks,
    onselect,
    onaction,
    onstackaction,
    onopenrun,
    onrefresh,
  }: Props = $props();

  let query = $state('');
  let stateFilter = $state<ContainerStateFilter>('all');
  let collapsed = $state<Record<string, boolean>>({});

  const runningTotal = $derived(
    containers.filter((c) => {
      const s = c.state.toLowerCase();
      return s === 'running' || s === 'restarting';
    }).length,
  );
  const stoppedTotal = $derived(
    containers.filter((c) => {
      const s = c.state.toLowerCase();
      return s !== 'running' && s !== 'restarting' && s !== 'paused';
    }).length,
  );

  const filtered = $derived(filterContainers(containers, query, stateFilter));
  const groups = $derived(groupContainersByProject(filtered));


  function openRowMenu(e: MouseEvent, c: ContainerSummary): void {
    e.preventDefault();
    const st = c.state.toLowerCase();
    const running = st === 'running';
    const paused = st === 'paused';
    const items: MenuItem[] = [];
    if (paused) {
      items.push({ label: 'Resume', icon: 'play', action: () => onaction(c, 'unpause') });
    } else if (running) {
      items.push(
        { label: 'Restart', icon: 'refresh', action: () => onaction(c, 'restart') },
        { label: 'Pause', icon: 'pause', action: () => onaction(c, 'pause') },
        { label: 'Stop', icon: 'stop', action: () => onaction(c, 'stop') },
      );
    } else {
      items.push({ label: 'Start', icon: 'play', action: () => onaction(c, 'start') });
    }
    items.push(
      { separator: true },
      { label: 'Remove container…', icon: 'trash', danger: true, action: () => onaction(c, 'remove') },
    );
    ctxMenu.show(e, items);
  }

  function dotToneClass(state: string): string {
    const info = containerStateInfo(state);
    if (info.tone === 'ok') return 'ct-dot-ok';
    if (info.tone === 'warn') return 'ct-dot-warn';
    if (info.tone === 'bad') return 'ct-dot-bad';
    if (info.tone === 'info') return 'ct-dot-info';
    return 'ct-dot-idle';
  }
</script>

<ListPane
  count={containers.length}
  primary={{ icon: 'play', label: 'Run container…', onclick: onopenrun, testid: 'containers-run' }}
  menu={() => [{ label: 'Refresh', icon: 'refresh', action: onrefresh }]}
  testid="containers-list"
>
  {#snippet searchRow()}
    <div class="ct-search">
      <Icon name="search" size={14} />
      <input type="search" dir="auto" aria-label="Filter containers" placeholder="Filter containers, images, ports…" bind:value={query} />
      {#if query}
        <button type="button" class="icon-btn" aria-label="Clear filter" title="Clear filter" onclick={() => (query = '')}>
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>
  {/snippet}
  {#snippet filterRow()}
    <div class="segmented ct-state-filter" role="group" aria-label="Filter by state">
      <button type="button" class:active={stateFilter === 'all'} aria-pressed={stateFilter === 'all'} onclick={() => (stateFilter = 'all')}>
        All <span class="ct-seg-count">{containers.length}</span>
      </button>
      <button type="button" class:active={stateFilter === 'running'} aria-pressed={stateFilter === 'running'} onclick={() => (stateFilter = 'running')}>
        Running <span class="ct-seg-count">{runningTotal}</span>
      </button>
      <button type="button" class:active={stateFilter === 'stopped'} aria-pressed={stateFilter === 'stopped'} onclick={() => (stateFilter = 'stopped')}>
        Stopped <span class="ct-seg-count">{stoppedTotal}</span>
      </button>
    </div>
  {/snippet}
  {#snippet footer()}
    <span>{plural(filtered.length, 'container')}{filtered.length !== containers.length ? ` of ${containers.length}` : ''}</span>
  {/snippet}

  {#if groups.length === 0}
    <EmptyState
      icon="search"
      title="No matching containers"
      body="Try clearing the search or switching the state filter."
      actionLabel="Clear filters"
      actionKind="secondary"
      onaction={() => {
        query = '';
        stateFilter = 'all';
      }}
    />
  {:else}
    {#each groups as group (group.key)}
      {@const stackBusy = group.project ? busyStacks.has(group.project) : false}
      <SideSection
        title={group.label}
        bind:open={() => !collapsed[group.key], (v) => (collapsed = { ...collapsed, [group.key]: !v })}
      >
        {#snippet actions()}
          <span class="ssh-count" title="{group.runningCount} of {group.totalCount} running">{group.runningCount}/{group.totalCount}</span>
          {#if group.project}
            {@const proj = group.project}
            <button type="button" class="icon-btn" disabled={stackBusy || group.runningCount === group.totalCount} aria-label="Start stack {proj}" title="Start stack {proj}" onclick={() => onstackaction(proj, 'start')}>
              <Icon name="play" size={12} />
            </button>
            <button type="button" class="icon-btn" disabled={stackBusy || group.runningCount === 0} aria-label="Restart stack {proj}" title="Restart stack {proj}" onclick={() => onstackaction(proj, 'restart')}>
              <Icon name="refresh" size={12} />
            </button>
            <button type="button" class="icon-btn" disabled={stackBusy || group.runningCount === 0} aria-label="Stop stack {proj}" title="Stop stack {proj}" onclick={() => onstackaction(proj, 'stop')}>
              <Icon name="stop" size={12} />
            </button>
          {/if}
        {/snippet}
        {#each group.containers as c (c.id)}
          {@const active = c.id === selectedId}
          {@const st = containerStateInfo(c.state)}
          {@const health = containerHealthInfo(c.health)}
          {@const portsText = formatPortsSummary(c.ports)}
          {@const svc = composeServiceOf(c)}
          {@const busy = busyIds.has(c.id)}
          <div class="side-row two-line ct-row" class:selected={active}>
            <button
              type="button"
              class="side-row-main ct-item"
              data-side-row
              aria-current={active ? 'true' : undefined}
              data-testid="container-row"
              data-container-id={c.id}
              onclick={() => onselect(c.id)}
              oncontextmenu={(e) => openRowMenu(e, c)}
              use:rowMenu
            >
              <span class="ct-dot {dotToneClass(c.state)}" class:live={st.live} title={st.label} aria-hidden="true"></span>
              <span class="side-row-text">
                <span class="ct-item-top">
                  <span class="side-row-title ct-item-name" dir="auto" title={c.name}>{c.name}</span>
                  <span class="ct-item-state" data-testid="container-state">{st.label}</span>
                  {#if health}
                    <span class="ct-health ct-health-{health.tone}" title={`Health: ${health.label}`}>{health.label}</span>
                  {/if}
                </span>
                <span class="side-row-meta ct-item-sub">
                  <span class="ct-item-image" dir="ltr" title={c.image}>{c.image}</span>
                  {#if svc && svc !== c.name}
                    <span aria-hidden="true">·</span>
                    <span class="ct-item-svc">{svc}</span>
                  {/if}
                </span>
                {#if busy}
                  <span class="ct-item-meta ct-item-busy">Working…</span>
                {:else if portsText}
                  <span class="ct-item-meta ct-item-ports" dir="ltr" title={portsText}>{portsText}</span>
                {/if}
              </span>
            </button>
            <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label="{c.name} actions" title="More" onclick={(e) => openRowMenu(e, c)}>
              <Icon name="more" size={14} />
            </button>
          </div>
        {/each}
      </SideSection>
    {/each}
  {/if}
</ListPane>

<style>
  /* One clear button: ours, not WebKit's as well (the Threads search field). */
  .ct-search {
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 28px;
    padding-inline: 8px 2px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
  }
  .ct-search:focus-within {
    color: var(--text);
    border-color: var(--accent-text);
  }
  .ct-search input::-webkit-search-cancel-button {
    display: none;
  }
  .ct-search input {
    flex: 1;
    min-width: 0;
    block-size: 100%;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    outline: none;
  }
  @media (pointer: coarse) {
    .ct-search {
      block-size: 36px;
    }
  }
  .ct-state-filter {
    display: flex;
    inline-size: 100%;
  }
  .ct-state-filter > button {
    flex: 1;
    justify-content: center;
    gap: 4px;
  }
  .ct-seg-count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  /* Three lines (name · state · health / image · service / ports): the row
     grows past the two-line 44 px and the dot sits on the first line. */
  .ct-item {
    align-items: flex-start;
    padding-block: 6px;
  }
  .ct-item-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
  }
  .ct-item-name {
    flex: 1;
  }
  .ct-item-state {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .ct-health {
    flex: none;
    font-size: var(--fs-xs);
  }
  .ct-health-ok {
    color: var(--success);
  }
  .ct-health-bad {
    color: var(--danger);
  }
  .ct-health-warn {
    color: var(--warning);
  }
  .ct-item-sub {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
  }
  .ct-item-image {
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .ct-item-svc {
    flex: none;
  }
  .ct-item-meta {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
  }
  .ct-item-ports {
    font-family: var(--font-mono);
    color: var(--accent-text);
  }
  .ct-item-busy {
    color: var(--warning);
  }
  .ct-dot {
    flex: none;
    inline-size: 8px;
    block-size: 8px;
    margin-block-start: 6px;
    border-radius: 50%;
  }
  .ct-dot-ok {
    background: var(--status-working);
  }
  .ct-dot-warn {
    background: var(--status-warn);
  }
  .ct-dot-bad {
    background: var(--status-exited);
  }
  .ct-dot-info {
    background: var(--info);
  }
  .ct-dot-idle {
    background: var(--status-idle);
  }
  .ct-dot.live {
    animation: otto-pulse 1.8s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .ct-dot.live {
      animation: none;
    }
  }
</style>
