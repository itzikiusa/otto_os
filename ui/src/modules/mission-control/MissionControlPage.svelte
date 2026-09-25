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
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
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
  /** The user closed the detail pane: don't auto-open it again this visit. */
  let userClosed = false;

  // Deep link `#/mission-control/<item id>` (the Home Mission Control box's
  // rows go there): open that item's detail. Without this the row click
  // landed on the page with nothing selected.
  $effect(() => {
    const [mod, itemId] = router.parts;
    if (mod === 'mission-control' && itemId) untrack(() => (selectedId = itemId));
  });

  /** Select an item (or close with null): the URL carries the selection so a
   *  reload / share lands on the same item, and the last pick is remembered. */
  function select(id: string | null): void {
    selectedId = id;
    if (id === null) userClosed = true;
    rememberSelection('mission-control', id);
    if (router.module === 'mission-control') router.replace(id ? `mission-control/${id}` : 'mission-control');
  }

  // Another workspace's item isn't selectable here: a switch starts fresh
  // (and re-arms the auto-open below).
  let lastWs = untrack(() => ws.currentId);
  $effect(() => {
    const id = ws.currentId;
    if (id === lastWs) return;
    lastWs = id;
    untrack(() => {
      items = [];
      if (selectedId) select(null);
      userClosed = false;
    });
  });

  // A list/detail page opens on an item, never a "pick one" void: on desktop
  // (where the detail sits beside the list) restore the last selection or the
  // first item once the list lands. Tablet/phone show the detail as a sheet
  // over the list, so there it opens only on a click.
  $effect(() => {
    if (selectedId || userClosed || !viewport.isDesktop || items.length === 0) return;
    const pick = untrack(() => initialSelection('mission-control', items, (i) => i.id));
    if (pick) untrack(() => select(pick));
  });

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
      err = e instanceof ApiError ? e.message : 'Otto couldn’t reach the daemon.';
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
  function onResizerKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    // Anchored at the inline end: ← widens (→ in RTL).
    const rtl = document.documentElement.dir === 'rtl';
    const grow = (e.key === 'ArrowLeft') !== rtl;
    detailW = Math.max(300, Math.min(720, detailW + (grow ? 24 : -24)));
    persistDetailW();
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
  <div class="tiles" aria-label="Summary">
    <div class="tile">
      <span class="t-val">{summary?.total ?? 0}</span>
      <span class="t-lbl">Work items</span>
    </div>
    <div class="tile">
      <span class="t-val accent">{summary?.active ?? 0}</span>
      <span class="t-lbl">Active</span>
    </div>
    <div class="tile" class:warn={(summary?.needs_approval ?? 0) > 0}>
      <span class="t-val" class:warn-text={(summary?.needs_approval ?? 0) > 0}>{summary?.needs_approval ?? 0}</span>
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
      <select class="input" bind:value={kindF} aria-label="Filter by kind">
        <option value="">All kinds</option>
        {#each WORK_KINDS as k (k)}<option value={k}>{KIND_LABEL[k]}</option>{/each}
      </select>
      <select class="input" bind:value={statusF} aria-label="Filter by status">
        <option value="">All statuses</option>
        {#each WORK_STATUSES as s (s)}<option value={s}>{STATUS_LABEL[s]}</option>{/each}
      </select>
      <select class="input" bind:value={riskF} aria-label="Filter by risk">
        <option value="">Any risk</option>
        {#each RISK_LEVELS as r (r)}<option value={r}>{RISK_LABEL[r]}</option>{/each}
      </select>
      <input class="input search" type="search" placeholder="Search titles…" bind:value={q} oninput={onQInput} aria-label="Search work items" />
      {#if hasFilters}<button class="btn ghost small" onclick={clearFilters}>Clear filters</button>{/if}
    </div>
    <div class="segmented view-toggle" role="tablist" aria-label="View">
      <button role="tab" aria-selected={view === 'list'} class:active={view === 'list'} onclick={() => (view = 'list')}>
        <Icon name="format" size={12} /> List
      </button>
      <button role="tab" aria-selected={view === 'graph'} class:active={view === 'graph'} onclick={() => (view = 'graph')}>
        <Icon name="share" size={12} /> Graph
      </button>
    </div>
  </div>

  {#if err && items.length > 0}
    <!-- Stale data + a failed refresh: keep the list, say so, offer Retry. -->
    <div class="banner-err" role="alert">
      <Icon name="warning" size={14} />
      <span class="be-text">Couldn't refresh Mission Control. {err}</span>
      <button class="btn small" onclick={() => ws.currentId && void reload(ws.currentId)}>Retry</button>
    </div>
  {/if}

  <!-- body -->
  <div class="mc-body">
    <div class="mc-main">
      {#if items.length === 0 && !loading && err}
        <!-- A failed load is not "no work items yet": say so, with Retry. -->
        <div class="card">
          <EmptyState
            icon="radar"
            title="Couldn't load Mission Control"
            body={err}
            actionLabel="Retry"
            actionIcon="refresh"
            onaction={() => ws.currentId && void reload(ws.currentId)}
          />
        </div>
      {:else if items.length === 0 && !loading}
        <div class="card">
          <EmptyState
            icon="radar"
            title={hasFilters ? 'No work items match these filters' : 'No work items yet'}
            body={hasFilters
              ? 'Try clearing the filters, or refresh to re-derive the graph from every module.'
              : 'Mission Control unifies every agentic activity. Start a session, swarm, loop, workflow, review, or story — or press Refresh to materialize existing work.'}
            actionLabel={hasFilters ? 'Clear filters' : backfilling ? 'Refreshing…' : 'Refresh'}
            onaction={hasFilters ? clearFilters : runBackfill}
          />
        </div>
      {:else if view === 'list'}
        <WorkItemList {items} needsApproval={needsApprovalIds} {selectedId} onOpen={select} />
      {:else}
        <WorkGraphView {graph} {selectedId} onOpen={select} />
      {/if}
    </div>

    {#if selectedId}
      <div class="mc-detail" style={`--mc-detail-w:${detailW}px`}>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div
          class="detail-resizer"
          role="separator"
          tabindex="0"
          aria-orientation="vertical"
          aria-label="Resize the detail pane"
          aria-valuenow={Math.round(detailW)}
          aria-valuemin={300}
          aria-valuemax={720}
          title="Drag or use ←/→ to resize · double-click to reset"
          ondblclick={resetDetailW}
          onpointerdown={startDetailResize}
          onkeydown={onResizerKey}
        ></div>
        <!-- ↑ a focusable separator: drag, or ←/→ to resize; double-click resets. -->
        <WorkItemDetail
          wsId={ws.currentId ?? ''}
          id={selectedId}
          onClose={() => select(null)}
          onOpen={select}
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
    border-radius: var(--radius-m);
    padding: 12px 14px;
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
    font-weight: 600;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  /* "Active" is live work: the info tone (runs' Running), not the accent. */
  .t-val.accent {
    color: var(--info);
  }
  .t-val.warn-text {
    color: var(--warning);
  }
  /* Sentence-case labels (content.md): the only uppercase micro-label is
     .section-title, and a figure's caption isn't one. */
  .t-lbl {
    font-size: var(--fs-s);
    color: var(--text-dim);
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
  .filters .search {
    min-width: 180px;
  }
  .view-toggle {
    flex: 0 0 auto;
  }
  .view-toggle button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .banner-err {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--danger);
    border-radius: var(--radius-s);
    padding: 6px 8px 6px 10px;
    font-size: var(--fs-s);
  }
  .be-text {
    flex: 1;
    min-width: 0;
    color: var(--text);
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
    border-radius: var(--radius-m);
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
  .detail-resizer:hover,
  .detail-resizer:focus-visible {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
    outline: none;
  }
  /* Tablet and phone (the 1024 breakpoint): the detail is a full sheet. */
  @media (max-width: 1024px) {
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
  @media (max-width: 640px) {
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
