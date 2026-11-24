<script lang="ts">
  // Left list pane for the Containers tab: search + state filter, Compose
  // project grouping with stack Start / Stop / Restart controls, and container
  // rows with status dot, image, ports summary and right-click menu.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
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

  function toggleGroup(key: string): void {
    collapsed = { ...collapsed, [key]: !collapsed[key] };
  }

  function openRowMenu(e: MouseEvent, c: ContainerSummary): void {
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

<aside class="ct-list-pane" aria-label="Containers list">
  <div class="ct-list-controls">
    <div class="input-group ct-search-box">
      <Icon name="search" size={13} />
      <input
        class="ct-search-input"
        type="search"
        dir="auto"
        aria-label="Filter containers"
        placeholder="Filter containers, images, ports…"
        bind:value={query}
      />
      {#if query}
        <button
          type="button"
          class="icon-btn"
          aria-label="Clear filter"
          title="Clear filter"
          onclick={() => (query = '')}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>

    <div class="segmented ct-state-filter" role="group" aria-label="Filter by state">
      <button
        type="button"
        class:active={stateFilter === 'all'}
        aria-pressed={stateFilter === 'all'}
        onclick={() => (stateFilter = 'all')}
      >
        All <span class="ct-seg-count">{containers.length}</span>
      </button>
      <button
        type="button"
        class:active={stateFilter === 'running'}
        aria-pressed={stateFilter === 'running'}
        onclick={() => (stateFilter = 'running')}
      >
        Running <span class="ct-seg-count">{runningTotal}</span>
      </button>
      <button
        type="button"
        class:active={stateFilter === 'stopped'}
        aria-pressed={stateFilter === 'stopped'}
        onclick={() => (stateFilter = 'stopped')}
      >
        Stopped <span class="ct-seg-count">{stoppedTotal}</span>
      </button>
    </div>
  </div>

  <div class="ct-list-scroll">
    {#if containers.length === 0}
      <EmptyState
        icon="box"
        title="No containers yet"
        body="Run an image or start a Compose project to see containers here."
        actionLabel="Run container…"
        actionIcon="play"
        actionKind="secondary"
        onaction={onopenrun}
      />
    {:else if groups.length === 0}
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
        {@const isCollapsed = !!collapsed[group.key]}
        {@const stackBusy = group.project ? busyStacks.has(group.project) : false}
        <section class="ct-group" aria-label={group.label}>
          <div class="ct-group-head">
            <button
              type="button"
              class="ct-group-toggle"
              aria-expanded={!isCollapsed}
              onclick={() => toggleGroup(group.key)}
            >
              <Icon name={isCollapsed ? 'chevronRight' : 'chevronDown'} size={12} />
              <Icon name={group.project ? 'layers' : 'box'} size={12} />
              <span class="ct-group-name" title={group.label}>{group.label}</span>
              <span class="ct-group-count">{group.runningCount}/{group.totalCount}</span>
            </button>

            {#if group.project}
              {@const proj = group.project}
              <div class="ct-stack-actions">
                <button
                  type="button"
                  class="icon-btn"
                  disabled={stackBusy || group.runningCount === group.totalCount}
                  aria-label="Start stack {proj}"
                  title="Start stack {proj}"
                  onclick={() => onstackaction(proj, 'start')}
                >
                  <Icon name="play" size={12} />
                </button>
                <button
                  type="button"
                  class="icon-btn"
                  disabled={stackBusy || group.runningCount === 0}
                  aria-label="Restart stack {proj}"
                  title="Restart stack {proj}"
                  onclick={() => onstackaction(proj, 'restart')}
                >
                  <Icon name="refresh" size={12} />
                </button>
                <button
                  type="button"
                  class="icon-btn"
                  disabled={stackBusy || group.runningCount === 0}
                  aria-label="Stop stack {proj}"
                  title="Stop stack {proj}"
                  onclick={() => onstackaction(proj, 'stop')}
                >
                  <Icon name="stop" size={12} />
                </button>
              </div>
            {/if}
          </div>

          {#if !isCollapsed}
            <div class="ct-group-items">
              {#each group.containers as c (c.id)}
                {@const active = c.id === selectedId}
                {@const st = containerStateInfo(c.state)}
                {@const health = containerHealthInfo(c.health)}
                {@const portsText = formatPortsSummary(c.ports)}
                {@const svc = composeServiceOf(c)}
                {@const busy = busyIds.has(c.id)}
                <button
                  type="button"
                  class="ct-item"
                  class:selected={active}
                  aria-current={active ? 'true' : undefined}
                  data-testid="container-row"
                  data-container-id={c.id}
                  onclick={() => onselect(c.id)}
                  oncontextmenu={(e) => openRowMenu(e, c)}
                  use:rowMenu
                >
                  <span
                    class="ct-dot {dotToneClass(c.state)}"
                    class:live={st.live}
                    title={st.label}
                    aria-hidden="true"
                  ></span>
                  <div class="ct-item-main">
                    <div class="ct-item-top">
                      <span class="ct-item-name" title={c.name}>{c.name}</span>
                      <span class="ct-item-state" data-testid="container-state">{st.label}</span>
                      {#if health}
                        <span class="ct-health ct-health-{health.tone}" title={`Health: ${health.label}`}>
                          {health.label}
                        </span>
                      {/if}
                    </div>
                    <div class="ct-item-sub">
                      <span class="ct-item-image" dir="ltr" title={c.image}>{c.image}</span>
                      {#if svc && svc !== c.name}
                        <span class="ct-item-sep" aria-hidden="true">·</span>
                        <span class="ct-item-svc">{svc}</span>
                      {/if}
                    </div>
                    {#if portsText || busy}
                      <div class="ct-item-meta">
                        {#if busy}
                          <span class="ct-item-busy">Working…</span>
                        {:else}
                          <span class="ct-item-ports" dir="ltr" title={portsText}>{portsText}</span>
                        {/if}
                      </div>
                    {/if}
                  </div>
                </button>
              {/each}
            </div>
          {/if}
        </section>
      {/each}
    {/if}
  </div>

  <footer class="ct-list-foot">
    <span>{plural(filtered.length, 'container')}</span>
    {#if filtered.length !== containers.length}
      <span>of {containers.length}</span>
    {/if}
  </footer>
</aside>

<style>
  .ct-list-pane {
    display: flex;
    flex-direction: column;
    inline-size: 310px;
    flex-shrink: 0;
    min-block-size: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--bg);
  }
  .ct-list-controls {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ct-search-box {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline: 8px 4px;
    block-size: 28px;
    border-radius: var(--radius-s);
    border: 1px solid var(--control-border);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .ct-search-box:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .ct-search-input {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .ct-search-input::placeholder {
    color: var(--text-dim);
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
  .ct-list-scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    padding-block: 6px;
  }
  .ct-group {
    margin-block-end: 6px;
  }
  .ct-group-head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-block: 4px;
    padding-inline: 8px;
  }
  .ct-group-toggle {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
    text-align: start;
  }
  .ct-group-toggle:hover {
    color: var(--text);
    background: var(--hover);
  }
  .ct-group-name {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-group-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .ct-stack-actions {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
  .ct-group-items {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-inline: 6px;
  }
  .ct-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    inline-size: 100%;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
    transition: background var(--dur-fast) ease-out, border-color var(--dur-fast) ease-out;
  }
  .ct-item:hover {
    background: var(--hover);
  }
  .ct-item.selected {
    background: var(--accent-soft);
    border-color: var(--accent-line);
  }
  /* Phone: the row is the tap target. */
  @media (pointer: coarse) {
    .ct-item {
      min-block-size: 44px;
    }
  }
  .ct-item-state {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .ct-dot {
    inline-size: 8px;
    block-size: 8px;
    border-radius: 50%;
    margin-block-start: 6px;
    flex-shrink: 0;
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
  .ct-item-main {
    flex: 1;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ct-item-top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
  }
  .ct-item-name {
    flex: 1;
    min-inline-size: 0;
    font-size: var(--fs-m);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-health {
    font-size: var(--fs-xs);
    flex-shrink: 0;
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
    color: var(--text-dim);
    min-inline-size: 0;
  }
  .ct-item-image {
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
  }
  .ct-item-sep {
    flex-shrink: 0;
  }
  .ct-item-svc {
    flex-shrink: 0;
  }
  .ct-item-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
  }
  .ct-item-ports {
    font-family: var(--font-mono);
    color: var(--accent-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-item-busy {
    color: var(--warning);
  }
  .ct-list-foot {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    border-block-start: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  @media (max-width: 640px) {
    .ct-list-pane {
      inline-size: 100%;
      border-inline-end: 0;
    }
  }
</style>
