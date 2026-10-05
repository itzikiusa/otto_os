<script lang="ts">
  // Mission Control — 6-bucket work-queue view (B4).
  //
  // Surfaces: needs_you | working | review_ready | waiting | failed | budget_warn
  // Live updates: driven off existing WS buses (reviewBus, workflowRunBus,
  // budgetBus, ws.needsYou changes) — minimal new polling (30 s fallback).

  import { onMount, onDestroy } from 'svelte';
  import { NO_WORKSPACE } from '../../lib/labels';
  import { toastError } from '../../lib/toastError';
  import { api } from '../../lib/api/client';
  import type { Poller } from '../../lib/poll';
  import { liveQuery } from '../../lib/live';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { reviewBus, workflowRunBus, budgetBus } from '../../lib/events.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { router } from '../../lib/router.svelte';
  import { winKey } from '../../lib/win';
  import { matchesSavedView } from './viewFilters';
  import { agentProviders } from '../../lib/providers';

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
  /** Why the last load failed — shown inline with Retry while there is no
   *  data to fall back on (a failed REFRESH keeps the stale board). */
  let loadError = $state<string | null>(null);
  let newViewName = $state('');
  let newViewFilter = $state('{}');
  let filterBucket = $state('');
  let filterProvider = $state('');
  let filterRepo = $state('');
  let advancedFilter = $state(false);
  const providers = $derived(agentProviders());
  const repositories = $derived([...new Set([
    ...ws.sessions.map((s) => s.cwd),
    ...(view ? Object.values(view).flat() as MissionItem[] : []).map((item) => item.repo ?? ''),
  ].filter(Boolean))].sort());
  let showNewViewForm = $state(false);
  /** A saved view is being created — the Save button reads "Saving…". */
  let savingView = $state(false);
  /** Saved views whose delete is in flight, by id (that pill's × disables). */
  let deletingViews = $state<Record<string, boolean>>({});

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

  let loadGeneration = 0;
  let alive = true;
  async function load(showSpinner = true) {
    const owner = wsId;
    const generation = ++loadGeneration;
    const current = () => alive && owner === wsId && generation === loadGeneration;
    if (!owner) { view = null; savedViews = []; loading = false; return; }
    if (showSpinner) loading = true;
    try {
      const [v, sv] = await Promise.all([
        api.get<MissionView>(`/workspaces/${owner}/mission`),
        api.get<SavedView[]>(`/workspaces/${owner}/mission/views`),
        // Task roll-up for the done/total strip on session-backed cards
        // (`GET /workspaces/{wid}/activity/summary`; `tasks_updated` keeps it live).
        activity.loadSummary(owner),
      ]);
      if (!current()) return;
      view = v;
      savedViews = sv;
      loadError = null;
    } catch (e) {
      // Best-effort — stale data stays; with none, the error shows inline.
      if (current()) loadError = loadErrorText(e);
    } finally {
      if (current()) loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Saved-view CRUD
  // ---------------------------------------------------------------------------

  async function createView() {
    if (!wsId || !newViewName.trim() || savingView) return;
    const owner = wsId;
    const name = newViewName.trim();
    const generation = loadGeneration;
    let filter: Record<string, unknown> = {};
    try {
      filter = advancedFilter ? JSON.parse(newViewFilter || '{}') : {
        ...(filterBucket ? { bucket: filterBucket } : {}),
        ...(filterProvider ? { provider: filterProvider } : {}),
        ...(filterRepo ? { repo: filterRepo } : {}),
      };
      if (!filter || typeof filter !== 'object' || Array.isArray(filter)) throw new Error('object required');
    } catch {
      toasts.error('Couldn’t save the view', 'The filter must be a JSON object, like {"bucket":"needs_you"}.');
      return;
    }
    savingView = true;
    try {
      await api.post(`/workspaces/${owner}/mission/views`, {
        name,
        filter,
      });
      if (!alive || wsId !== owner || generation !== loadGeneration) return;
      newViewName = '';
      newViewFilter = '{}';
      filterBucket = ''; filterProvider = ''; filterRepo = ''; advancedFilter = false;
      showNewViewForm = false;
      await load(false);
    } catch (e: unknown) {
      toastError('Couldn’t save the view', e);
    } finally {
      savingView = false;
    }
  }

  async function deleteView(id: string) {
    const owner = wsId;
    const name = savedViews.find((sv) => sv.id === id)?.name;
    const ok = await confirmer.ask(`Delete the saved view${name ? ` “${name}”` : ''}? Only the view is removed — no sessions are affected.`, {
      title: 'Delete saved view?',
      confirmLabel: 'Delete view',
    });
    if (!ok || deletingViews[id]) return;
    deletingViews = { ...deletingViews, [id]: true };
    try {
      await api.del(`/mission-views/${id}`);
      toasts.success('View deleted', name);
      if (!alive || wsId !== owner) return;
      await load(false);
    } catch (e: unknown) {
      toastError('Couldn’t delete the view', e);
    } finally {
      const next = { ...deletingViews };
      delete next[id];
      deletingViews = next;
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
    return items.filter((it) => matchesSavedView(it, activeFilter, ws.sessions));
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
  // Shared poll chain: never overlaps a pending load, pauses while the window
  // is hidden (one catch-up load on return), backs off while the daemon fails.
  let fallbackPoll: Poller | null = null;

  onMount(() => {
    // The `$effect(wsId)` below fires on mount too — no immediate load here,
    // or the view loads twice back to back.
    // Event-fed: session lifecycle/status (no dedicated bus) + work-graph
    // changes for this workspace, coalesced so a burst of working↔idle flips
    // costs at most one reload per ~15 s. The 30 s cadence returns only while
    // the event socket is down (5-min safety net otherwise).
    fallbackPoll = liveQuery({
      run: () => load(false),
      on: ['session_created', 'session_status', 'session_removed', 'work_graph_updated'],
      match: (ev) => ev.workspace_id === undefined || ev.workspace_id === ws.currentId,
      fallbackMs: 30_000,
      debounceMs: 3000,
      maxWaitMs: 15_000,
      minIntervalMs: 15_000,
      immediate: false,
    });
  });

  onDestroy(() => {
    alive = false;
    ++loadGeneration;
    fallbackPoll?.stop();
    clearTimeout(refreshTimer);
  });

  // Reload when the workspace changes.
  $effect(() => {
    void load();
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
  const BUCKET_ICONS: Record<BucketKey, IconName> = {
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
      toasts.info('Sub-task queued', `“${title}” is handed to the agent when it’s idle.`);
      closeSubtask();
      if (wsId) void activity.loadSummary(wsId);
    } catch (e) {
      toastError('Couldn’t add sub-task', e);
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
    <!-- "Work Queue" — the name the tab bar gives this view; "Mission Control"
         is the separate sidebar module (the work graph). -->
    <h2>Work queue</h2>
    <div class="header-actions">
      <button class="btn small" onclick={() => router.go('history')} title="Browse past conversations" data-testid="mission-history-btn">
        <Icon name="clock" size={12} /> History
      </button>
      <button class="icon-btn" onclick={() => load()} title="Refresh" aria-label="Refresh" disabled={loading}>
        <Icon name="refresh" size={13} />
      </button>
    </div>
  </div>

  <!-- Saved views bar: one pill per view (click to filter, the × beside it
       deletes — a sibling button, never nested inside the pill). -->
  <div class="saved-views" role="group" aria-label="Saved views">
    <span class="sv-label">Views</span>
    <button
      class="wq-view solo"
      class:active={activeViewId === null}
      aria-pressed={activeViewId === null}
      onclick={() => (activeViewId = null)}
    >All</button>
    {#each savedViews as sv (sv.id)}
      <span class="wq-view-wrap" class:active={activeViewId === sv.id}>
        <button
          class="wq-view"
          class:active={activeViewId === sv.id}
          aria-pressed={activeViewId === sv.id}
          onclick={() => (activeViewId = sv.id)}
          title={sv.name}
        ><span class="ellipsis">{sv.name}</span></button>
        <button
          class="wq-view-del"
          onclick={() => void deleteView(sv.id)}
          disabled={deletingViews[sv.id]}
          aria-busy={deletingViews[sv.id] ? 'true' : undefined}
          title={deletingViews[sv.id] ? `Deleting view “${sv.name}”…` : `Delete view “${sv.name}”`}
          aria-label={deletingViews[sv.id] ? `Deleting view “${sv.name}”…` : `Delete view “${sv.name}”`}
        ><Icon name="x" size={12} /></button>
      </span>
    {/each}
    <button class="btn small ghost" onclick={() => (showNewViewForm = !showNewViewForm)} aria-expanded={showNewViewForm}>
      <Icon name="plus" size={12} /> Save view…
    </button>
  </div>

  {#if showNewViewForm}
    <div class="new-view-form">
      <input dir="auto"
        type="text"
        class="input"
        placeholder="View name"
        aria-label="View name"
        bind:value={newViewName}
        onkeydown={(e) => { if (e.key === 'Enter') void createView(); else if (e.key === 'Escape') showNewViewForm = false; }}
      />
      {#if advancedFilter}
        <input dir="ltr" type="text" class="input wide mono" aria-label="Filter JSON" placeholder={'{"bucket":"needs_you"}'} bind:value={newViewFilter} />
      {:else}
        <select class="input" aria-label="View status" bind:value={filterBucket}><option value="">All statuses</option>{#each ALL_BUCKETS as bucket}<option value={bucket}>{BUCKET_LABELS[bucket]}</option>{/each}</select>
        <select class="input" aria-label="View provider" bind:value={filterProvider}><option value="">All providers</option>{#each providers as provider}<option value={provider}>{provider}</option>{/each}</select>
        <select class="input" aria-label="View repository" bind:value={filterRepo}><option value="">All repositories</option>{#each repositories as repo}<option value={repo}>{repo}</option>{/each}</select>
      {/if}
      <label class="checkbox-row adv"><input type="checkbox" bind:checked={advancedFilter} /> Advanced (JSON)</label>
      <span class="grow"></span>
      <button class="btn small" onclick={() => (showNewViewForm = false)}>Cancel</button>
      <button class="btn small primary" onclick={createView} disabled={!newViewName.trim() || savingView}>{savingView ? 'Saving…' : 'Save view'}</button>
    </div>
  {/if}

  <!-- Buckets -->
  {#if !wsId}
    <EmptyState variant="page" icon="folder" title={NO_WORKSPACE} body="Pick a workspace in the sidebar to see its sessions by what they need." />
  {:else}
  <LoadState what="the work queue" variant="page" loading={loading} error={loadError} empty={!view} onretry={() => load()}>
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
                <!-- The title is the card's one real button; its ::after stretches over
                     the card so a click anywhere opens the session, while the
                     Sub-task controls sit above it as their own buttons. -->
                <li class="item" class:clickable={!!sid} data-session-id={sid}>
                  {#if sid}
                    <button class="item-title item-open" title={item.title} onclick={() => openSession(item)}>{item.title}</button>
                  {:else}
                    <span class="item-title" title={item.title}>{item.title}</span>
                  {/if}
                  <div class="item-meta">
                    {#if item.repo}
                      <span class="meta-tag repo" title={item.repo}>{item.repo}</span>
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
                          title="Push a sub-task to this agent"
                          data-testid="subtask-btn"
                        ><Icon name="plus" size={12} /> Sub-task</button>
                      {/if}
                    </div>
                    {#if subtaskFor === item.id}
                      <div class="subtask-form" data-subtask-for={item.id}>
                        <input dir="auto"
                          class="input subtask-input"
                          placeholder="Sub-task for the agent…"
                          bind:value={subtaskTitle}
                          onkeydown={(e) => onSubtaskKeydown(e, item)}
                          spellcheck="false"
                          aria-label="Sub-task title"
                        />
                        <button class="btn small" onclick={closeSubtask}>Cancel</button>
                        <button class="btn small primary" disabled={subtaskTitle.trim() === '' || subtaskBusy} onclick={() => void submitSubtask(item)}>{subtaskBusy ? 'Adding…' : 'Add'}</button>
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
  </LoadState>
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
    gap: 12px;
    min-height: 44px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--separator);
  }

  .mission-header h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
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
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    margin-inline-end: 2px;
  }

  /* View pills: a quiet pill, the active one in the accent (selection, not a
     primary action). */
  .wq-view-wrap {
    display: inline-flex;
    align-items: center;
    min-width: 0;
    max-width: 220px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
  }
  .wq-view-wrap.active {
    border-color: var(--accent-solid);
    background: var(--accent-soft);
  }
  .wq-view {
    display: inline-flex;
    align-items: center;
    min-width: 0;
    height: 24px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .wq-view-wrap .wq-view {
    border: none;
    background: transparent;
    padding-inline-end: 4px;
  }
  .wq-view:hover {
    color: var(--text);
  }
  .wq-view.active {
    color: var(--accent-text);
    font-weight: 500;
  }
  .wq-view.solo.active {
    border-color: var(--accent-solid);
    background: var(--accent-soft);
  }
  .wq-view-del {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    margin-inline-end: 2px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .wq-view-del:hover:not(:disabled) {
    background: var(--hover);
    color: var(--danger);
  }
  .wq-view-del:disabled {
    cursor: progress;
    opacity: var(--disabled-opacity);
  }

  /* New-view form */
  .new-view-form {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 16px;
    background: var(--surface);
    border-bottom: 1px solid var(--separator);
  }
  .new-view-form .input {
    width: auto;
    min-width: 140px;
  }
  .new-view-form .input.wide {
    flex: 1;
    min-width: 240px;
  }
  .new-view-form .adv {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .grow {
    flex: 1;
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
    border-radius: var(--radius-m);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
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
    background: var(--accent-solid);
    color: var(--accent-contrast);
    border-radius: 999px;
    padding: 1px 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
  }

  .bucket-empty {
    padding: 10px 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }

  .item-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .item {
    position: relative;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    transition: background var(--dur-fast);
  }
  .item:last-child {
    border-bottom: none;
  }
  .item.clickable {
    cursor: pointer;
  }
  .item.clickable:hover {
    background: var(--hover);
  }
  /* The open button covers the whole card (stretched hit area); its focus
     ring is drawn on the card. */
  .item-open {
    max-width: 100%;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .item-open::after {
    content: '';
    position: absolute;
    inset: 0;
  }
  .item-open:focus-visible {
    outline: none;
  }
  .item.clickable:has(.item-open:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  /* Controls inside the card stay above the stretched hit area. */
  .task-strip button,
  .subtask-form {
    position: relative;
    z-index: 1;
  }

  .item-title {
    display: block;
    font-size: var(--fs-m);
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item-meta {
    display: flex;
    gap: 4px;
    margin-top: 2px;
    flex-wrap: wrap;
  }

  .meta-tag {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 1px 4px;
  }
  /* A repo is a full path — truncate it inside the card instead of letting
     the bucket's overflow:hidden chop it mid-character. */
  .meta-tag.repo {
    max-width: 100%;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta-tag.cost {
    color: var(--warning);
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
    font-size: var(--fs-xs);
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
  }
  .strip-now {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .subtask-btn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-inline-start: auto;
    height: 20px;
    padding: 0 6px;
    border: 1px dashed var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .subtask-btn:hover {
    border-color: var(--accent);
    color: var(--accent-text);
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
    font-size: var(--fs-s);
  }
</style>
