///<reference types="svelte" />
;// Agents → Work Queue — 6-bucket work-queue view (B4). Named WorkQueue (not
// MissionControl) so it is not confused with the Mission Control module.
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
import NeedsYouLink from '../home/LazyNeedsYouLink.svelte';
function $$render() {

  
  
  
  
  
  

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
      danger: true,
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
;
async () => {

 { svelteHTML.createElement("div", { "class":`mission`,});
  
   { svelteHTML.createElement("div", { "class":`mission-header`,});
    
     { svelteHTML.createElement("h2", {});  }
     { svelteHTML.createElement("div", { "class":`header-actions`,});
      
       { const $$_kniLuoYsdeeN3C = __sveltets_2_ensureComponent(NeedsYouLink); new $$_kniLuoYsdeeN3C({ target: __sveltets_2_any(), props: {}});}
       { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => router.go('history'),"title":`Browse past conversations`,...__sveltets_2_empty({"data-testid":`mission-history-btn`}),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => load(),"title":`Refresh`,"aria-label":`Refresh`,"disabled":loading,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
       }
     }
   }

  
   { svelteHTML.createElement("div", {     "class":`saved-views`,"role":`group`,"aria-label":`Saved views`,});
     { svelteHTML.createElement("span", { "class":`sv-label`,});  }
     { svelteHTML.createElement("button", {        "class":`wq-view solo`,"aria-pressed":activeViewId === null,"onclick":() => (activeViewId = null),});activeViewId === null;  }
       for(let sv of __sveltets_2_ensureArray(savedViews)){sv.id;
       { svelteHTML.createElement("span", {  "class":`wq-view-wrap`,});activeViewId === sv.id;
         { svelteHTML.createElement("button", {          "class":`wq-view`,"aria-pressed":activeViewId === sv.id,"onclick":() => (activeViewId = sv.id),"title":sv.name,});activeViewId === sv.id; { svelteHTML.createElement("span", { "class":`ellipsis`,});sv.name; } }
         { svelteHTML.createElement("button", {             "class":`wq-view-del`,"onclick":() => void deleteView(sv.id),"disabled":deletingViews[sv.id],"aria-busy":deletingViews[sv.id] ? 'true' : undefined,"title":deletingViews[sv.id] ? `Deleting view “${sv.name}”…` : `Delete view “${sv.name}”`,"aria-label":deletingViews[sv.id] ? `Deleting view “${sv.name}”…` : `Delete view “${sv.name}”`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
       }
    }
     { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => (showNewViewForm = !showNewViewForm),"aria-expanded":showNewViewForm,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
     }
   }

  if(showNewViewForm){
     { svelteHTML.createElement("div", { "class":`new-view-form`,});
       { svelteHTML.createElement("input", {              "dir":`auto`,"type":`text`,"class":`input`,"placeholder":`View name`,"aria-label":`View name`,"bind:value":newViewName,"onkeydown":(e) => { if (e.key === 'Enter') void createView(); else if (e.key === 'Escape') showNewViewForm = false; },});/*Ωignore_startΩ*/() => newViewName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(advancedFilter){
         { svelteHTML.createElement("input", {            "dir":`ltr`,"type":`text`,"class":`input wide mono`,"aria-label":`Filter JSON`,"placeholder":'{"bucket":"needs_you"}',"bind:value":newViewFilter,});/*Ωignore_startΩ*/() => newViewFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      }else{
         { svelteHTML.createElement("select", {     "class":`input`,"aria-label":`View status`,"bind:value":filterBucket,});/*Ωignore_startΩ*/() => filterBucket = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",});  }  for(let bucket of __sveltets_2_ensureArray(ALL_BUCKETS)){ { svelteHTML.createElement("option", { "value":bucket,});BUCKET_LABELS[bucket]; }} }
         { svelteHTML.createElement("select", {     "class":`input`,"aria-label":`View provider`,"bind:value":filterProvider,});/*Ωignore_startΩ*/() => filterProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",});  }  for(let provider of __sveltets_2_ensureArray(providers)){ { svelteHTML.createElement("option", { "value":provider,});provider; }} }
         { svelteHTML.createElement("select", {     "class":`input`,"aria-label":`View repository`,"bind:value":filterRepo,});/*Ωignore_startΩ*/() => filterRepo = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",});  }  for(let repo of __sveltets_2_ensureArray(repositories)){ { svelteHTML.createElement("option", { "value":repo,});repo; }} }
      }
       { svelteHTML.createElement("label", { "class":`checkbox-row adv`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":advancedFilter,});/*Ωignore_startΩ*/() => advancedFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}   }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (showNewViewForm = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":createView,"disabled":!newViewName.trim() || savingView,});savingView ? 'Saving…' : 'Save view'; }
     }
  }

  
  if(!wsId){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`folder`,"title":NO_WORKSPACE,"body":`Pick a workspace in the sidebar to see its sessions by what they need.`,}});}
  }else{
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`the work queue`,"variant":`page`,"loading":loading,"error":loadError,"empty":!view,"onretry":() => load(),}});
     { svelteHTML.createElement("div", { "class":`buckets`,});
         for(let bucket of __sveltets_2_ensureArray(activeBuckets())){bucket;
        const items = filteredItems(bucket);
         { svelteHTML.createElement("section", {  "class":`bucket`,});items.length === 0;
           { svelteHTML.createElement("div", { "class":`bucket-header`,});
             { svelteHTML.createElement("span", { "class":`bucket-icon`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":BUCKET_ICONS[bucket],"size":13,}});} }
             { svelteHTML.createElement("span", { "class":`bucket-name`,});BUCKET_LABELS[bucket]; }
            if(items.length > 0){
               { svelteHTML.createElement("span", { "class":`bucket-count`,});items.length; }
            }
           }

          if(items.length === 0){
             { svelteHTML.createElement("div", { "class":`bucket-empty`,});  }
          }else{
             { svelteHTML.createElement("ul", { "class":`item-list`,});
                 for(let item of __sveltets_2_ensureArray(items)){item.id;
                const sid = sessionOf(item);
                const sum = sid ? activity.summary(sid) : null;
                
                 { svelteHTML.createElement("li", {    "class":`item`,...__sveltets_2_empty({"data-session-id":sid}),});!!sid;
                  if(sid){
                     { svelteHTML.createElement("button", {     "class":`item-title item-open`,"title":item.title,"onclick":() => openSession(item),});item.title; }
                  }else{
                     { svelteHTML.createElement("span", {   "class":`item-title`,"title":item.title,});item.title; }
                  }
                   { svelteHTML.createElement("div", { "class":`item-meta`,});
                    if(item.repo){
                       { svelteHTML.createElement("span", {   "class":`meta-tag repo`,"title":item.repo,});item.repo; }
                    }
                    if(item.cost_usd !== undefined && item.cost_usd > 0){
                       { svelteHTML.createElement("span", { "class":`meta-tag cost`,}); item.cost_usd.toFixed(2); }
                    }
                     { svelteHTML.createElement("span", { "class":`meta-tag age`,});fmtAge(item.age_secs);  }
                   }
                  if(sid){
                    
                     { svelteHTML.createElement("div", {   "class":`task-strip`,...__sveltets_2_empty({"data-testid":`task-strip`}),});
                      if(sum && sum.total > 0){
                         { svelteHTML.createElement("span", {   "class":`strip-count mono`,...__sveltets_2_empty({"data-testid":`task-strip-count`}),});sum.done; sum.total; }
                         { svelteHTML.createElement("span", {   "class":`strip-track`,"aria-hidden":`true`,});
                           { svelteHTML.createElement("span", {   "class":`strip-fill`,"style":`width:${Math.round((sum.done / sum.total) * 100)}%`,}); }
                         }
                        if(sum.in_progress){
                           { svelteHTML.createElement("span", {   "class":`strip-now`,"title":sum.in_progress,});sum.in_progress; }
                        }
                      }else{
                         { svelteHTML.createElement("span", {   "class":`strip-count mono dim`,...__sveltets_2_empty({"data-testid":`task-strip-count`}),});  }
                         { svelteHTML.createElement("span", { "class":`strip-now dim`,});   }
                      }
                      if(canEdit && subtaskFor !== item.id){
                         { svelteHTML.createElement("button", {         "class":`subtask-btn`,"onclick":(e) => openSubtask(e, item),"title":`Push a sub-task to this agent`,...__sveltets_2_empty({"data-testid":`subtask-btn`}),}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
                      }
                     }
                    if(subtaskFor === item.id){
                       { svelteHTML.createElement("div", {   "class":`subtask-form`,...__sveltets_2_empty({"data-subtask-for":item.id}),});
                         { svelteHTML.createElement("input", {              "dir":`auto`,"class":`input subtask-input`,"placeholder":`Sub-task for the agent…`,"bind:value":subtaskTitle,"onkeydown":(e) => onSubtaskKeydown(e, item),"spellcheck":`false`,"aria-label":`Sub-task title`,});/*Ωignore_startΩ*/() => subtaskTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":closeSubtask,});  }
                         { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":subtaskTitle.trim() === '' || subtaskBusy,"onclick":() => void submitSubtask(item),});subtaskBusy ? 'Adding…' : 'Add'; }
                       }
                    }
                  }
                 }
              }
             }
          }
         }
      }
     }
   LoadState}
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WorkQueue__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkQueue__SvelteComponent_ = ReturnType<typeof WorkQueue__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkQueue__SvelteComponent_;