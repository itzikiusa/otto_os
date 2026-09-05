<script lang="ts">
  // Mission Control — 6-bucket work-queue view (B4).
  //
  // Surfaces: needs_you | working | review_ready | waiting | failed | budget_warn
  // Live updates: driven off existing WS buses (reviewBus, workflowRunBus,
  // budgetBus, ws.needsYou changes) — minimal new polling (30 s fallback).

  import { onMount, onDestroy } from 'svelte';
  import { api } from '../../lib/api/client';
  import Icon from '../../lib/components/Icon.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { reviewBus, workflowRunBus, budgetBus } from '../../lib/events.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { router } from '../../lib/router.svelte';
  import { winKey } from '../../lib/win';

  // ---------------------------------------------------------------------------
  // Types (module-local; mirroring the Rust DTOs without touching api/types.ts)
  // ---------------------------------------------------------------------------

  interface MissionItem {
    kind: string;
    id: string;
    title: string;
    status: string;
    session_id?: string;
    repo?: string;
    cost_usd?: number;
    age_secs: number;
  }

  interface MissionView {
    needs_you: MissionItem[];
    working: MissionItem[];
    review_ready: MissionItem[];
    waiting: MissionItem[];
    failed: MissionItem[];
    budget_warn: MissionItem[];
  }

  interface SavedView {
    id: string;
    user_id: string;
    workspace_id: string;
    name: string;
    filter: Record<string, unknown>;
    created_at: string;
  }

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let view: MissionView | null = $state(null);
  let savedViews: SavedView[] = $state([]);
  let loading = $state(false);
  let newViewName = $state('');
  let newViewFilter = $state('{}');
  let showNewViewForm = $state(false);

  /** The active saved view's ID (null = no filter active = show all).
   *  Tracked by id, not by filter-object identity — `load(false)` replaces
   *  `savedViews` with fresh objects, which silently broke an `===` compare. */
  let activeViewId: string | null = $state(null);

  const activeFilter: Record<string, unknown> | null = $derived(
    activeViewId === null
      ? null
      : (savedViews.find((sv) => sv.id === activeViewId)?.filter ?? null),
  );

  const wsId = $derived(ws.currentId);

  // ---------------------------------------------------------------------------
  // Data loading
  // ---------------------------------------------------------------------------

  async function load(showSpinner = true) {
    if (!wsId) return;
    if (showSpinner) loading = true;
    try {
      const [v, sv] = await Promise.all([
        api.get<MissionView>(`/workspaces/${wsId}/mission`),
        api.get<SavedView[]>(`/workspaces/${wsId}/mission/views`),
        // Task roll-up for the done/total strip on session-backed cards
        // (`GET /workspaces/{wid}/activity/summary`; `tasks_updated` keeps it live).
        activity.loadSummary(wsId),
      ]);
      view = v;
      savedViews = sv;
    } catch {
      /* best-effort — stale data stays */
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Saved-view CRUD
  // ---------------------------------------------------------------------------

  async function createView() {
    if (!wsId || !newViewName.trim()) return;
    let filter: Record<string, unknown> = {};
    try {
      filter = JSON.parse(newViewFilter || '{}');
    } catch {
      toasts.error('Filter must be valid JSON');
      return;
    }
    try {
      await api.post(`/workspaces/${wsId}/mission/views`, {
        name: newViewName.trim(),
        filter,
      });
      newViewName = '';
      newViewFilter = '{}';
      showNewViewForm = false;
      await load(false);
    } catch (e: unknown) {
      toasts.error(e instanceof Error ? e.message : 'Failed to save view');
    }
  }

  async function deleteView(id: string) {
    try {
      await api.del(`/mission-views/${id}`);
      await load(false);
    } catch (e: unknown) {
      toasts.error(e instanceof Error ? e.message : 'Failed to delete view');
    }
  }

  // ---------------------------------------------------------------------------
  // Bucket filtering (when a saved view with a bucket filter is active)
  // ---------------------------------------------------------------------------

  type BucketKey = keyof MissionView;
  const ALL_BUCKETS: BucketKey[] = [
    'needs_you',
    'working',
    'review_ready',
    'waiting',
    'failed',
    'budget_warn',
  ];

  function activeBuckets(): BucketKey[] {
    if (!activeFilter) return ALL_BUCKETS;
    const b = activeFilter['bucket'];
    if (typeof b === 'string' && ALL_BUCKETS.includes(b as BucketKey)) {
      return [b as BucketKey];
    }
    return ALL_BUCKETS;
  }

  function filteredItems(key: BucketKey): MissionItem[] {
    if (!view) return [];
    const items = view[key] ?? [];
    if (!activeFilter) return items;
    // Apply min_cost_usd filter if present.
    const minCost = activeFilter['min_cost_usd'];
    if (typeof minCost === 'number') {
      return items.filter((it) => (it.cost_usd ?? 0) >= minCost);
    }
    return items;
  }

  // ---------------------------------------------------------------------------
  // Live refresh: react to event buses and needsYou changes
  // ---------------------------------------------------------------------------

  // Refresh when any of the existing buses tick (review/workflow/budget).
  let prevReviewTick = reviewBus.tick;
  let prevWorkflowTick = workflowRunBus.tick;
  let prevBudgetTick = budgetBus.tick;
  let prevNeedsYouKeys = '';

  $effect(() => {
    const rt = reviewBus.tick;
    const wt = workflowRunBus.tick;
    const bt = budgetBus.tick;
    // Track needsYou changes as a serialized key set.
    const ny = Object.keys(ws.needsYou)
      .filter((k) => ws.needsYou[k])
      .sort()
      .join(',');

    if (rt !== prevReviewTick || wt !== prevWorkflowTick || bt !== prevBudgetTick || ny !== prevNeedsYouKeys) {
      prevReviewTick = rt;
      prevWorkflowTick = wt;
      prevBudgetTick = bt;
      prevNeedsYouKeys = ny;
      // Debounce: wait 500 ms to batch rapid bus ticks.
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => load(false), 500);
    }
  });

  let refreshTimer: ReturnType<typeof setTimeout>;

  // 30 s fallback poll — the buses cover most live updates; this catches
  // anything that doesn't have a WS event (e.g. new sessions, Idle transitions).
  let pollInterval: ReturnType<typeof setInterval>;

  onMount(() => {
    // The `$effect(wsId)` below fires on mount too — no initial load() here,
    // or the view loads twice back to back.
    pollInterval = setInterval(() => load(false), 30_000);
  });

  onDestroy(() => {
    clearInterval(pollInterval);
    clearTimeout(refreshTimer);
  });

  // Reload when the workspace changes.
  $effect(() => {
    if (wsId) load();
  });

  // ---------------------------------------------------------------------------
  // Display helpers
  // ---------------------------------------------------------------------------

  const BUCKET_LABELS: Record<BucketKey, string> = {
    needs_you: 'Needs You',
    working: 'Working',
    review_ready: 'Review Ready',
    waiting: 'Waiting',
    failed: 'Failed',
    budget_warn: 'Budget Warning',
  };

  // Icon-component names (the rest of the app uses Icon, not emoji).
  const BUCKET_ICONS: Record<BucketKey, string> = {
    needs_you: 'bell',
    working: 'zap',
    review_ready: 'eye',
    waiting: 'clock',
    failed: 'x',
    budget_warn: 'chart',
  };

  function fmtAge(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m`;
    if (secs < 86400) return `${Math.floor(secs / 3600)}h`;
    return `${Math.floor(secs / 86400)}d`;
  }

  /** The session a card is backed by, if any. */
  function sessionOf(item: MissionItem): string | null {
    return item.session_id ?? (item.kind === 'session' ? item.id : null);
  }

  function openSession(item: MissionItem) {
    const sid = sessionOf(item);
    if (!sid) return;
    // From the board you want the conversation, not the raw PTY: preselect the
    // Chat view (SessionView persists the choice per session under this key —
    // docs/design/conversation-view.md §5.1) before navigating.
    try {
      localStorage.setItem(winKey(`otto_session_view:${sid}`), 'chat');
    } catch {
      /* storage unavailable — SessionView falls back to its default */
    }
    // Navigate for real (route + tabs view) — a bare store mutation would
    // change a tab invisibly behind the Mission Control surface.
    ws.setViewMode('tabs');
    ws.navigateToSession(sid);
  }

  // ---------------------------------------------------------------------------
  // Sub-tasks: push a task from the board into the agent (§5.5)
  // ---------------------------------------------------------------------------

  const canEdit = $derived(ws.myRole !== 'viewer');
  /** Card (item id) whose inline "+ Sub-task" input is open. */
  let subtaskFor = $state<string | null>(null);
  let subtaskTitle = $state('');
  let subtaskBusy = $state(false);

  function openSubtask(e: Event, item: MissionItem): void {
    e.stopPropagation();
    subtaskFor = item.id;
    subtaskTitle = '';
    queueMicrotask(() =>
      (document.querySelector(`[data-subtask-for="${item.id}"] input`) as HTMLInputElement | null)?.focus(),
    );
  }

  function closeSubtask(): void {
    subtaskFor = null;
    subtaskTitle = '';
  }

  async function submitSubtask(item: MissionItem): Promise<void> {
    const sid = sessionOf(item);
    const title = subtaskTitle.trim();
    if (!sid || !title || subtaskBusy) return;
    subtaskBusy = true;
    try {
      await activity.addTask(sid, title);
      toasts.info('Sub-task queued', `"${title}" is handed to the agent when it is idle.`);
      closeSubtask();
      if (wsId) void activity.loadSummary(wsId);
    } catch (e) {
      toasts.error('Could not add sub-task', e instanceof Error ? e.message : String(e));
    } finally {
      subtaskBusy = false;
    }
  }

  function onSubtaskKeydown(e: KeyboardEvent, item: MissionItem): void {
    e.stopPropagation();
    if (e.key === 'Enter') {
      e.preventDefault();
      void submitSubtask(item);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeSubtask();
    }
  }
</script>

<div class="mission">
  <!-- Header bar -->
  <div class="mission-header">
    <h2>Mission Control</h2>
    <div class="header-actions">
      {#if activeViewId !== null}
        <button class="chip active" onclick={() => (activeViewId = null)}>
          Clear filter ×
        </button>
      {/if}
      <button class="chip" onclick={() => router.go('history')} title="Browse past conversations" data-testid="mission-history-btn">
        <Icon name="clock" size={12} /> History
      </button>
      <button class="icon-btn" onclick={() => load()} title="Refresh" aria-label="Refresh">
        <Icon name="refresh" size={13} />
      </button>
    </div>
  </div>

  <!-- Saved views bar -->
  <div class="saved-views">
    <span class="sv-label">Views:</span>
    <button
      class="chip"
      class:active={activeViewId === null}
      onclick={() => (activeViewId = null)}
    >All</button>
    {#each savedViews as sv (sv.id)}
      <button
        class="chip"
        class:active={activeViewId === sv.id}
        onclick={() => (activeViewId = sv.id)}
      >
        {sv.name}
        <span
          class="chip-del"
          role="button"
          tabindex="-1"
          onclick={(e) => {
            e.stopPropagation();
            deleteView(sv.id);
          }}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.stopPropagation();
              deleteView(sv.id);
            }
          }}
          title="Delete view"
        >×</span>
      </button>
    {/each}
    <button class="chip new" onclick={() => (showNewViewForm = !showNewViewForm)}>
      + Save view
    </button>
  </div>

  {#if showNewViewForm}
    <div class="new-view-form">
      <input
        type="text"
        placeholder="View name"
        bind:value={newViewName}
        class="sv-input"
      />
      <input
        type="text"
        placeholder={'Filter JSON e.g. {"bucket":"needs_you"}'}
        bind:value={newViewFilter}
        class="sv-input wide"
      />
      <button class="btn-save" onclick={createView}>Save</button>
      <button class="btn-cancel" onclick={() => (showNewViewForm = false)}>Cancel</button>
    </div>
  {/if}

  <!-- Buckets -->
  {#if loading && !view}
    <div class="loading">Loading work queue…</div>
  {:else if !view}
    <div class="empty">No workspace selected.</div>
  {:else}
    <div class="buckets">
      {#each activeBuckets() as bucket (bucket)}
        {@const items = filteredItems(bucket)}
        <section class="bucket" class:empty-bucket={items.length === 0}>
          <div class="bucket-header">
            <span class="bucket-icon"><Icon name={BUCKET_ICONS[bucket]} size={13} /></span>
            <span class="bucket-name">{BUCKET_LABELS[bucket]}</span>
            {#if items.length > 0}
              <span class="bucket-count">{items.length}</span>
            {/if}
          </div>

          {#if items.length === 0}
            <div class="bucket-empty">Nothing here</div>
          {:else}
            <ul class="item-list">
              {#each items as item (item.id)}
                {@const sid = sessionOf(item)}
                {@const sum = sid ? activity.summary(sid) : null}
                <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
                <li
                  class="item"
                  class:clickable={!!sid}
                  role={sid ? 'button' : undefined}
                  tabindex={sid ? 0 : undefined}
                  data-session-id={sid}
                  onclick={() => openSession(item)}
                  onkeydown={(e) => {
                    if (e.target !== e.currentTarget) return;
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      openSession(item);
                    }
                  }}
                >
                  <span class="item-title">{item.title}</span>
                  <div class="item-meta">
                    {#if item.repo}
                      <span class="meta-tag">{item.repo}</span>
                    {/if}
                    {#if item.cost_usd !== undefined && item.cost_usd > 0}
                      <span class="meta-tag cost">${item.cost_usd.toFixed(2)}</span>
                    {/if}
                    <span class="meta-tag age">{fmtAge(item.age_secs)} ago</span>
                  </div>
                  {#if sid}
                    <!-- Task strip: done/total from the activity roll-up (counts ALL
                         rows, so board-added tasks are included) + inline sub-task. -->
                    <div class="task-strip" data-testid="task-strip">
                      {#if sum && sum.total > 0}
                        <span class="strip-count mono" data-testid="task-strip-count">{sum.done}/{sum.total}</span>
                        <span class="strip-track" aria-hidden="true">
                          <span class="strip-fill" style="width:{Math.round((sum.done / sum.total) * 100)}%"></span>
                        </span>
                        {#if sum.in_progress}
                          <span class="strip-now" title={sum.in_progress}>{sum.in_progress}</span>
                        {/if}
                      {:else}
                        <span class="strip-count mono dim" data-testid="task-strip-count">0/0</span>
                        <span class="strip-now dim">no tasks yet</span>
                      {/if}
                      {#if canEdit && subtaskFor !== item.id}
                        <button
                          class="subtask-btn"
                          onclick={(e) => openSubtask(e, item)}
                          onkeydown={(e) => e.stopPropagation()}
                          title="Push a sub-task to this agent"
                          data-testid="subtask-btn"
                        >+ Sub-task</button>
                      {/if}
                    </div>
                    {#if subtaskFor === item.id}
                      <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                      <div class="subtask-form" data-subtask-for={item.id} onclick={(e) => e.stopPropagation()}>
                        <input
                          class="sv-input subtask-input"
                          placeholder="Sub-task for the agent…"
                          bind:value={subtaskTitle}
                          onkeydown={(e) => onSubtaskKeydown(e, item)}
                          spellcheck="false"
                          aria-label="Sub-task title"
                        />
                        <button class="btn-save" disabled={subtaskTitle.trim() === '' || subtaskBusy} onclick={() => void submitSubtask(item)}>Add</button>
                        <button class="btn-cancel" onclick={closeSubtask}>Cancel</button>
                      </div>
                    {/if}
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .mission {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
    background: var(--bg);
    color: var(--text);
  }

  .mission-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px 8px;
    border-bottom: 1px solid var(--border);
  }

  .mission-header h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .icon-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 16px;
    padding: 2px 6px;
    border-radius: 4px;
  }
  .icon-btn:hover {
    background: var(--surface);
    color: var(--text);
  }

  /* Saved views */
  .saved-views {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
  }

  .sv-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 10px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--surface);
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }
  .chip:hover,
  .chip.active {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .chip.new {
    border-style: dashed;
  }
  .chip-del {
    font-size: 14px;
    line-height: 1;
    opacity: 0.7;
    cursor: pointer;
  }
  .chip-del:hover {
    opacity: 1;
  }

  /* New-view form */
  .new-view-form {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  .sv-input {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    padding: 4px 8px;
    font-size: 13px;
    min-width: 140px;
  }
  .sv-input.wide {
    min-width: 260px;
    flex: 1;
  }
  .btn-save {
    padding: 4px 12px;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
  }
  .btn-cancel {
    padding: 4px 12px;
    background: none;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
  }

  /* Buckets grid */
  .buckets {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 12px;
    padding: 12px 16px;
    overflow-y: auto;
    flex: 1;
  }

  .bucket {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
  }
  .bucket.empty-bucket {
    opacity: 0.6;
  }

  .bucket-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .bucket-icon {
    display: grid;
    place-items: center;
    color: var(--text-dim);
  }
  .bucket-name {
    flex: 1;
  }
  .bucket-count {
    background: var(--accent);
    color: #fff;
    border-radius: 9px;
    padding: 1px 7px;
    font-size: 11px;
    font-weight: 700;
  }

  .bucket-empty {
    padding: 10px 12px;
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
  }

  .item-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .item {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    transition: background 0.1s;
  }
  .item:last-child {
    border-bottom: none;
  }
  .item.clickable {
    cursor: pointer;
  }
  .item.clickable:hover {
    background: var(--surface-2);
  }
  .item.clickable:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .item-title {
    display: block;
    font-size: 13px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item-meta {
    display: flex;
    gap: 4px;
    margin-top: 3px;
    flex-wrap: wrap;
  }

  .meta-tag {
    font-size: 11px;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 4px;
    padding: 1px 5px;
  }
  .meta-tag.cost {
    color: #f59e0b;
  }
  .meta-tag.age {
    color: var(--text-dim);
  }

  /* Task strip + sub-task (session-backed cards) */
  .task-strip {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    min-width: 0;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .dim {
    color: var(--text-dim);
  }
  .strip-count {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--text);
  }
  .strip-track {
    flex: 0 0 56px;
    height: 4px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .strip-fill {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: 999px;
    transition: width 200ms ease-out;
  }
  .strip-now {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .subtask-btn {
    flex-shrink: 0;
    margin-inline-start: auto;
    height: 18px;
    padding: 0 7px;
    border: 1px dashed var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: 10.5px;
    cursor: pointer;
  }
  .subtask-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .subtask-form {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }
  .subtask-input {
    flex: 1;
    min-width: 0;
    font-size: 12px;
  }

  .loading,
  .empty {
    padding: 24px;
    text-align: center;
    color: var(--text-dim);
    font-size: 13px;
  }
</style>
