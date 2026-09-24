<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
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

  // Monotonic request token: a slower, OLDER response must never overwrite a
  // newer one (live ticks fire reloads back to back).
  let reqSeq = 0;

  async function reload(id: string): Promise<void> {
    const seq = ++reqSeq;
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
      if (seq !== reqSeq) return;
      summary = s;
      items = its;
      graph = g;
    } catch (e) {
      if (seq !== reqSeq) return;
      err = e instanceof ApiError ? e.message : 'Failed to load Mission Control';
    } finally {
      if (seq === reqSeq) loading = false;
    }
  }

  // Load on workspace change and filter change.
  $effect(() => {
    const id = ws.currentId;
    // establish dependencies so the effect re-runs when these change
    void [kindF, statusF, riskF, debouncedQ];
    if (id) void reload(id);
  });

  // Live work_graph_updated ticks: only THIS workspace's (or a reconnect
  // resync, which carries none), debounced. Every tick from every workspace
  // used to refetch summary + 300 items + the graph, once per item transition.
  const LIVE_DEBOUNCE_MS = 500;
  let liveTimer: ReturnType<typeof setTimeout> | null = null;
  let seenTick = untrack(() => missionControlBus.tick);
  $effect(() => {
    const tick = missionControlBus.tick;
    const evWs = missionControlBus.workspaceId;
    if (tick === seenTick) return;
    seenTick = tick;
    const id = untrack(() => ws.currentId);
    if (!id || (evWs !== '' && evWs !== id)) return;
    if (liveTimer) clearTimeout(liveTimer);
    liveTimer = setTimeout(() => {
      liveTimer = null;
      const cur = ws.currentId;
      if (cur) void reload(cur);
    }, LIVE_DEBOUNCE_MS);
  });
  onDestroy(() => {
    if (liveTimer) clearTimeout(liveTimer);
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

  // ── Detail pane width (drag-resizable, persisted) ──────────────────────────
  // Mirrors the DatabasePage sidebar idiom: the chosen width survives reloads.
  // Applied via a CSS var so the ≤900px fullscreen overlay still wins.
  const DETAIL_W_DEFAULT = 380;
  let detailW = $state(loadDetailW());
  function loadDetailW(): number {
    if (typeof localStorage === 'undefined') return DETAIL_W_DEFAULT;
    const v = Number(localStorage.getItem('mc.detailW'));
    return Number.isFinite(v) && v >= 300 ? v : DETAIL_W_DEFAULT;
  }
  function persistDetailW(): void {
    try {
      localStorage.setItem('mc.detailW', String(Math.round(detailW)));
    } catch {
      /* storage unavailable — non-fatal */
    }
  }
  function startDetailResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = detailW;
    const onMove = (ev: PointerEvent): void => {
      // The pane is anchored RIGHT, so dragging LEFT widens it.
      detailW = Math.max(300, Math.min(720, startW + (startX - ev.clientX)));
    };
    const onUp = (): void => {
      persistDetailW();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }
  function resetDetailW(): void {
    detailW = DETAIL_W_DEFAULT;
    persistDetailW();
  }
</script>

<div class="mc-page">
<PageHeader
  title="Mission Control"
  subtitle="Every agentic activity as one traceable unit — sessions, swarms, loops, workflows, reviews, stories, PRs & triggers."
>
  {#snippet actions()}
    <button class="btn small" disabled={backfilling || loading} onclick={runBackfill} title="Re-derive the graph from every source">
      <Icon name="refresh" size={13} /> {backfilling ? 'Refreshing…' : 'Refresh'}
    </button>
  {/snippet}
</PageHeader>
<PageBody>
<div class="mission-control" class:detail-open={selectedId}>
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
      <span class="t-val">{fmtCost(summary?.total_cost ?? 0)}</span>
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
        <div class="card">
          <EmptyState
            icon="radar"
            title={hasFilters ? 'No work items match these filters' : 'No work items yet'}
            body={hasFilters
              ? 'Try clearing the filters, or refresh to re-derive the graph from every module.'
              : 'Mission Control unifies every agentic activity. Start a session, swarm, loop, workflow, review, or story — or press Refresh to materialize existing work.'}
            actionLabel={hasFilters ? 'Clear filters' : backfilling ? 'Refreshing…' : 'Refresh / backfill'}
            onaction={hasFilters ? clearFilters : runBackfill}
          />
        </div>
      {:else if view === 'list'}
        <WorkItemList {items} needsApproval={needsApprovalIds} {selectedId} onOpen={(id) => (selectedId = id)} />
      {:else}
        <WorkGraphView {graph} {selectedId} onOpen={(id) => (selectedId = id)} />
      {/if}
    </div>

    {#if selectedId}
      <div class="mc-detail" style={`--mc-detail-w:${detailW}px`}>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="detail-resizer"
          role="separator"
          aria-orientation="vertical"
          aria-label="Drag to resize the detail pane (double-click to reset)"
          title="Drag to resize · double-click to reset"
          ondblclick={resetDetailW}
          onpointerdown={startDetailResize}
        ></div>
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
</PageBody>
</div>

<style>
  .mission-control {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 100%;
  }
  .mc-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
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
    border-color: var(--warning);
    background: color-mix(in srgb, var(--warning) 8%, var(--surface));
  }
  .t-val {
    font-size: var(--fs-2xl);
    font-weight: 700;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  .t-val.accent {
    color: var(--accent-text);
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
    font-size: var(--fs-s);
    padding: 5px 8px;
  }
  .filters .search {
    min-width: 160px;
  }
  /* The shared segmented-control look (app.css .segmented): selection is a
     raised surface, never a colour — the old lime fill read as "success". */
  .view-toggle {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    flex: 0 0 auto;
  }
  .view-toggle button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 10px;
    background: transparent;
    border: none;
    border-radius: 4px;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 500;
    cursor: pointer;
  }
  .view-toggle button.on {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
  }
  .banner-err {
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--danger);
    border-radius: 6px;
    padding: 7px 10px;
    font-size: var(--fs-s);
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
    /* Default width; an inline `--mc-detail-w` (drag-resizable, persisted)
       overrides it. The ≤900px overlay below goes fullscreen instead. */
    flex: 0 0 var(--mc-detail-w, 380px);
    width: var(--mc-detail-w, 380px);
    border-radius: var(--radius-m, 8px);
    overflow: hidden;
    border: 1px solid var(--border);
    align-self: stretch;
    position: relative; /* anchors the drag handle on the inline-start edge */
  }
  /* Draggable divider on the detail pane's inline-start edge (the pane clips
     its overflow, so the handle sits just inside the border). */
  .detail-resizer {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    width: 6px;
    cursor: col-resize;
    background: transparent;
    z-index: 2;
    touch-action: none;
  }
  .detail-resizer:hover {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
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
    /* Fullscreen overlay — nothing to drag. */
    .detail-resizer {
      display: none;
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
