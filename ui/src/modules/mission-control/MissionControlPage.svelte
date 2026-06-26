<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { missionControlBus } from '../../lib/events.svelte';
  import { missionControlApi } from '../../lib/api/missionControl';
  import { ApiError } from '../../lib/api/client';
  import type {
    GraphView,
    MissionSummary,
    WorkItem,
    WorkKind,
    WorkStatus,
    RiskLevel,
    MissionFilterQuery,
  } from '../../lib/api/types';
  import { WORK_KINDS, WORK_STATUSES, RISK_LEVELS, KIND_LABEL, STATUS_LABEL, RISK_LABEL, fmtCost } from './lib';
  import WorkItemList from './WorkItemList.svelte';
  import WorkGraphView from './WorkGraphView.svelte';
  import WorkItemDetail from './WorkItemDetail.svelte';

  let summary = $state<MissionSummary | null>(null);
  let items = $state<WorkItem[]>([]);
  let graph = $state<GraphView>({ nodes: [], edges: [] });
  let loading = $state(false);
  let err = $state('');
  let backfilling = $state(false);

  let view = $state<'list' | 'graph'>('list');
  let selectedId = $state<string | null>(null);

  // filters
  let kindF = $state<WorkKind | ''>('');
  let statusF = $state<WorkStatus | ''>('');
  let riskF = $state<RiskLevel | ''>('');
  let q = $state('');
  let debouncedQ = $state('');
  let qTimer: ReturnType<typeof setTimeout> | null = null;
  function onQInput(): void {
    if (qTimer) clearTimeout(qTimer);
    qTimer = setTimeout(() => (debouncedQ = q), 250);
  }

  const needsApprovalIds = $derived(
    new Set(graph.nodes.filter((n) => n.needs_approval).map((n) => n.id)),
  );

  async function reload(id: string): Promise<void> {
    loading = true;
    err = '';
    const f: MissionFilterQuery = {
      kind: kindF || undefined,
      status: statusF || undefined,
      risk: riskF || undefined,
      q: debouncedQ || undefined,
      limit: 300,
    };
    try {
      const [s, its, g] = await Promise.all([
        missionControlApi.summary(id),
        missionControlApi.items(id, f),
        missionControlApi.graph(id, f),
      ]);
      summary = s;
      items = its;
      graph = g;
    } catch (e) {
      err = e instanceof ApiError ? e.message : 'Failed to load Mission Control';
    } finally {
      loading = false;
    }
  }

  // Load on workspace change, filter change, and each live work_graph_updated tick.
  $effect(() => {
    const id = ws.currentId;
    // establish dependencies so the effect re-runs when these change
    void [kindF, statusF, riskF, debouncedQ, missionControlBus.tick];
    if (id) void reload(id);
  });

  async function runBackfill(): Promise<void> {
    const id = ws.currentId;
    if (!id) return;
    backfilling = true;
    err = '';
    try {
      await missionControlApi.backfill(id);
      await reload(id);
    } catch (e) {
      err = e instanceof ApiError ? e.message : 'Backfill failed';
    } finally {
      backfilling = false;
    }
  }

  function clearFilters(): void {
    kindF = '';
    statusF = '';
    riskF = '';
    q = '';
    debouncedQ = '';
  }

  const hasFilters = $derived(kindF !== '' || statusF !== '' || riskF !== '' || debouncedQ !== '');
  function onChange(): void {
    const id = ws.currentId;
    if (id) void reload(id);
  }
</script>

<div class="page mission-control" class:detail-open={selectedId}>
  <div class="page-header head-row">
    <div>
      <h1>Mission Control</h1>
      <div class="sub dim">Every agentic activity as one traceable unit — sessions, swarms, loops, workflows, reviews, stories, PRs &amp; triggers.</div>
    </div>
    <div class="head-actions">
      <button class="btn small" disabled={backfilling || loading} onclick={runBackfill} title="Re-derive the graph from every source">
        <Icon name="refresh" size={13} /> {backfilling ? 'Refreshing…' : 'Refresh'}
      </button>
    </div>
  </div>

  <!-- summary tiles -->
  <div class="tiles">
    <div class="tile">
      <span class="t-val">{summary?.total ?? 0}</span>
      <span class="t-lbl">Work items</span>
    </div>
    <div class="tile">
      <span class="t-val accent">{summary?.active ?? 0}</span>
      <span class="t-lbl">Active</span>
    </div>
    <div class="tile" class:warn={(summary?.needs_approval ?? 0) > 0}>
      <span class="t-val">{summary?.needs_approval ?? 0}</span>
      <span class="t-lbl">Needs approval</span>
    </div>
    <div class="tile">
      <span class="t-val mono">{fmtCost(summary?.total_cost ?? 0)}</span>
      <span class="t-lbl">Total cost</span>
    </div>
  </div>

  <!-- toolbar -->
  <div class="toolbar">
    <div class="filters">
      <select bind:value={kindF} aria-label="Filter by kind">
        <option value="">All kinds</option>
        {#each WORK_KINDS as k (k)}<option value={k}>{KIND_LABEL[k]}</option>{/each}
      </select>
      <select bind:value={statusF} aria-label="Filter by status">
        <option value="">All statuses</option>
        {#each WORK_STATUSES as s (s)}<option value={s}>{STATUS_LABEL[s]}</option>{/each}
      </select>
      <select bind:value={riskF} aria-label="Filter by risk">
        <option value="">All risk</option>
        {#each RISK_LEVELS as r (r)}<option value={r}>{RISK_LABEL[r]}</option>{/each}
      </select>
      <input class="search" type="search" placeholder="Search title…" bind:value={q} oninput={onQInput} aria-label="Search work items" />
      {#if hasFilters}<button class="btn ghost small" onclick={clearFilters}>Clear</button>{/if}
    </div>
    <div class="view-toggle" role="tablist" aria-label="View">
      <button role="tab" aria-selected={view === 'list'} class:on={view === 'list'} onclick={() => (view = 'list')}>
        <Icon name="sidebar" size={13} /> List
      </button>
      <button role="tab" aria-selected={view === 'graph'} class:on={view === 'graph'} onclick={() => (view = 'graph')}>
        <Icon name="grid" size={13} /> Graph
      </button>
    </div>
  </div>

  {#if err}<div class="banner-err">{err}</div>{/if}

  <!-- body -->
  <div class="mc-body">
    <div class="mc-main">
      {#if items.length === 0 && !loading}
        <div class="empty card">
          <div class="empty-icon"><Icon name="radar" size={28} /></div>
          <h3>No work items{hasFilters ? ' match these filters' : ' yet'}</h3>
          <p class="dim">
            {#if hasFilters}
              Try clearing the filters, or refresh to re-derive the graph from every module.
            {:else}
              Mission Control unifies every agentic activity. Start a session, swarm, loop, workflow,
              review, or story — or press Refresh to materialize existing work.
            {/if}
          </p>
          <div class="empty-actions">
            {#if hasFilters}<button class="btn small" onclick={clearFilters}>Clear filters</button>{/if}
            <button class="btn primary small" disabled={backfilling} onclick={runBackfill}>
              {backfilling ? 'Refreshing…' : 'Refresh / backfill'}
            </button>
          </div>
        </div>
      {:else if view === 'list'}
        <WorkItemList {items} needsApproval={needsApprovalIds} {selectedId} onOpen={(id) => (selectedId = id)} />
      {:else}
        <WorkGraphView {graph} {selectedId} onOpen={(id) => (selectedId = id)} />
      {/if}
    </div>

    {#if selectedId}
      <div class="mc-detail">
        <WorkItemDetail
          wsId={ws.currentId ?? ''}
          id={selectedId}
          onClose={() => (selectedId = null)}
          onOpen={(id) => (selectedId = id)}
          {onChange}
        />
      </div>
    {/if}
  </div>
</div>

<style>
  .mission-control {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 100%;
  }
  .head-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .sub {
    font-size: 12px;
    margin-top: 2px;
    max-width: 640px;
  }
  .head-actions {
    flex: 0 0 auto;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
  }
  .tile {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 11px 14px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tile.warn {
    border-color: #ffd33d;
    background: color-mix(in srgb, #ffd33d 8%, var(--surface));
  }
  .t-val {
    font-size: 22px;
    font-weight: 700;
    line-height: 1;
  }
  .t-val.accent {
    color: var(--accent);
  }
  .t-lbl {
    font-size: 11px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }
  .filters {
    display: flex;
    gap: 7px;
    flex-wrap: wrap;
    align-items: center;
  }
  .filters select,
  .filters .search {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    padding: 5px 8px;
  }
  .filters .search {
    min-width: 160px;
  }
  .view-toggle {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 7px;
    overflow: hidden;
    flex: 0 0 auto;
  }
  .view-toggle button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: var(--surface);
    border: none;
    color: var(--text-dim);
    font: inherit;
    font-size: 12.5px;
    font-weight: 600;
    padding: 6px 12px;
    cursor: pointer;
  }
  .view-toggle button.on {
    background: #7ee787;
    color: #0a0a0a;
  }
  .banner-err {
    background: color-mix(in srgb, #ff5f57 14%, transparent);
    border: 1px solid #ff5f5766;
    color: #ff5f57;
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 12.5px;
  }
  .mc-body {
    flex: 1 1 auto;
    display: flex;
    gap: 12px;
    min-height: 320px;
  }
  .mc-main {
    flex: 1 1 auto;
    min-width: 0;
  }
  .mc-detail {
    flex: 0 0 380px;
    width: 380px;
    border-radius: var(--radius-m, 8px);
    overflow: hidden;
    border: 1px solid var(--border);
    align-self: stretch;
  }
  .empty {
    text-align: center;
    padding: 40px 20px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    min-height: 240px;
    justify-content: center;
  }
  .empty-icon {
    color: var(--text-dim);
    opacity: 0.7;
  }
  .empty h3 {
    margin: 4px 0 0;
    font-size: 15px;
  }
  .empty p {
    margin: 0;
    max-width: 460px;
    font-size: 12.5px;
  }
  .empty-actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
  @media (max-width: 900px) {
    .tiles {
      grid-template-columns: repeat(2, 1fr);
    }
    .mc-detail {
      position: fixed;
      inset: 0;
      z-index: 40;
      width: auto;
      flex: none;
      border: none;
      border-radius: 0;
    }
  }
  @media (max-width: 560px) {
    .toolbar {
      align-items: stretch;
    }
    .filters {
      width: 100%;
    }
    .filters .search {
      flex: 1 1 auto;
      min-width: 0;
    }
  }
</style>
