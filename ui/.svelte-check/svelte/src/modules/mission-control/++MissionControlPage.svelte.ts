///<reference types="svelte" />
;
import { onDestroy, untrack } from 'svelte';
import { loadPaneWidth } from '../../lib/paneResizer';
import { onTabKey } from '../../lib/tabKeys';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { dialogFocus } from '../../lib/dialogFocus';
import Icon from '../../lib/components/Icon.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { missionControlBus } from '../../lib/events.svelte';
import { missionControlApi } from '../../lib/api/missionControl';
import { ApiError } from '../../lib/api/client';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
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
import { liveDebounce } from './liveDebounce';
import WorkItemList from './WorkItemList.svelte';
import WorkItemDetail from './WorkItemDetail.svelte';
import NeedsYouLink from '../home/LazyNeedsYouLink.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let summary = $state<MissionSummary | null>(null);
  let items = $state<WorkItem[]>([]);
  let graph = $state<GraphView>({ nodes: [], edges: [] });
  let loading = $state(false);
  let err = $state('');
  let backfilling = $state(false);

  let view = $state<'list' | 'graph'>('list');
  let selectedId = $state<string | null>(null);
  // The graph chunk loads only after Graph is selected. The detail pane stays
  // a direct WorkItemDetail binding so canLeave still sees the open draft.
  const WorkGraphLazy = lazyComponent(() => import('./WorkGraphView.svelte'));
  let detailPane: ReturnType<typeof WorkItemDetail> | undefined = $state();
  let selectionGeneration = 0;
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
  async function select(id: string | null): Promise<void> {
    if (id === selectedId) return;
    const generation = ++selectionGeneration;
    if (detailPane && !await detailPane.canLeave()) return;
    if (generation !== selectionGeneration) return;
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
      ++reqSeq;
      summary = null;
      graph = { nodes: [], edges: [] };
      loading = false;
      err = '';
      items = [];
      needsApprovalIds = new Set();
      // Drop the selection synchronously: `select(null)` awaits the detail's
      // leave check and then sets `userClosed = true` — AFTER the re-arm below
      // — so the page stopped opening on an item after a switch. The old
      // workspace's item is gone either way; there's nothing to keep editing.
      ++selectionGeneration;
      if (selectedId) {
        selectedId = null;
        if (router.module === 'mission-control') router.replace('mission-control');
      }
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

  // Tablet/phone: the detail is a full-screen sheet on the Modal layer. The
  // `detailSheet` effect below registers it ONCE (pushModal + focus trap);
  // a second registration here used to count the same sheet twice.

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

  // Which list rows carry "Needs approval". Only graph nodes say so, and the
  // graph is fetched lazily for the Graph view — so the List view used to show
  // no badge at all while the tile read "Needs approval: 3". `reload` keeps
  // this fresh on every load (from the graph when it fetched one, else a graph
  // fetch only when the summary says something needs approval).
  let needsApprovalIds = $state<Set<string>>(new Set());
  const approvalSet = (g: GraphView) => new Set(g.nodes.filter((n) => n.needs_approval).map((n) => n.id));

  // Monotonic request token: a slower, OLDER response must never overwrite a
  // newer one (live ticks fire reloads back to back).
  let reqSeq = 0;
  /** The list's page size — the server returns at most this many rows. */
  const ITEM_LIMIT = 300;
  /** The graph is only fetched while it is shown; otherwise a reload marks it
   *  stale and flipping to the Graph view loads it then. */
  let graphStale = true;

  /** Commit only the newest request FOR THE SHOWN WORKSPACE: a backfill of A
   *  that resolves after a switch to B takes the newest ticket, so the seq
   *  alone would land A's items under B. */
  const stale = (seq: number, id: string): boolean => seq !== reqSeq || id !== ws.currentId;

  async function reload(id: string): Promise<void> {
    const seq = ++reqSeq;
    loading = true;
    err = '';
    const f: MissionFilterQuery = {
      kind: kindF || undefined,
      status: statusF || undefined,
      risk: riskF || undefined,
      q: debouncedQ || undefined,
      limit: ITEM_LIMIT,
    };
    const withGraph = untrack(() => view) === 'graph';
    if (!withGraph) graphStale = true;
    try {
      const [s, its, g] = await Promise.all([
        missionControlApi.summary(id),
        missionControlApi.items(id, f),
        withGraph ? missionControlApi.graph(id, f) : Promise.resolve(null),
      ]);
      if (stale(seq, id)) return;
      summary = s;
      items = its;
      if (g) {
        graph = g;
        graphStale = false;
        needsApprovalIds = approvalSet(g);
      } else if (s.needs_approval > 0) {
        const ag = await missionControlApi.graph(id, f);
        if (stale(seq, id)) return;
        graph = ag;
        graphStale = false;
        needsApprovalIds = approvalSet(ag);
      } else {
        needsApprovalIds = new Set();
      }
    } catch (e) {
      if (stale(seq, id)) return;
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

  // Flipping to the Graph view loads a graph that went stale while hidden.
  $effect(() => {
    if (view !== 'graph') return;
    const id = untrack(() => ws.currentId);
    if (id && graphStale) void reload(id);
  });

  // Live work_graph_updated ticks: only THIS workspace's (or a reconnect
  // resync, which carries none), debounced. Every tick from every workspace
  // used to refetch summary + 300 items + the graph, once per item transition.
  // The max wait keeps a steady stream (a busy swarm) from starving the
  // refresh forever — at most one reload per LIVE_MAX_WAIT_MS meanwhile (A7).
  const LIVE_DEBOUNCE_MS = 500;
  const LIVE_MAX_WAIT_MS = 3000;
  const live = liveDebounce(() => {
    const cur = ws.currentId;
    if (cur) void reload(cur);
  }, LIVE_DEBOUNCE_MS, LIVE_MAX_WAIT_MS);
  let seenTick = untrack(() => missionControlBus.tick);
  $effect(() => {
    const tick = missionControlBus.tick;
    const evWs = missionControlBus.workspaceId;
    if (tick === seenTick) return;
    seenTick = tick;
    const id = untrack(() => ws.currentId);
    if (!id || (evWs !== '' && evWs !== id)) return;
    live.trigger();
  });
  onDestroy(() => live.cancel());

  async function runBackfill(): Promise<void> {
    const id = ws.currentId;
    if (!id) return;
    backfilling = true;
    err = '';
    try {
      await missionControlApi.backfill(id);
      if (id === ws.currentId) await reload(id);
    } catch (e) {
      if (id === ws.currentId) err = e instanceof ApiError ? e.message : 'Backfill failed';
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

  // ⌘K: the page's verbs. (Work items are projected from sessions, PRs,
  // reviews, workflows… — there is no "new work item" here; Refresh pulls in
  // anything those modules recorded.)
  $effect(() => {
    const filtered = hasFilters;
    const v = view;
    return registry.register('mission-control', [
      { id: 'mission-control.refresh', title: 'Refresh work items', group: 'Mission Control', keywords: 'backfill sync reload', run: () => void runBackfill() },
      v === 'list'
        ? { id: 'mission-control.graph', title: 'Show work graph', group: 'Mission Control', keywords: 'view dependencies', run: () => (view = 'graph') }
        : { id: 'mission-control.list', title: 'Show work list', group: 'Mission Control', keywords: 'view items', run: () => (view = 'list') },
      ...(filtered ? [{ id: 'mission-control.clear', title: 'Clear filters', group: 'Mission Control', keywords: 'reset search', run: clearFilters }] : []),
    ]);
  });
  function onChange(): void {
    const id = ws.currentId;
    if (id) void reload(id);
  }

  // ── Detail pane width (drag-resizable, persisted) ──────────────────────────
  // The shared PaneDivider owns drag, keys, reset and persistence. Applied via a
  // CSS var so the ≤1024px fullscreen sheet still wins.
  const DETAIL_W = { default: 380, min: 300, max: 720 } as const;
  let detailW = $state(loadPaneWidth('mc.detailW', DETAIL_W.default, DETAIL_W.min, DETAIL_W.max));
  // Below the 1024 breakpoint the detail is a full-viewport sheet over the page:
  // it must behave as a dialog — register as modal, trap Tab, Esc closes, focus
  // returns to the row that opened it (the same wiring as the k8s ResourceDrawer).
  const detailSheet = $derived(!viewport.isDesktop && !!selectedId);
  let detailEl = $state<HTMLElement | null>(null);
  $effect(() => {
    if (!detailSheet || !detailEl) return;
    untrack(() => ui.pushModal());
    const focus = dialogFocus(detailEl, () => select(null));
    return () => {
      focus.destroy();
      untrack(() => ui.popModal());
    };
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`mc-page`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":`Mission Control`,"subtitle":`Every agentic activity as one traceable unit`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
    
     { const $$_kniLuoYsdeeN0C = __sveltets_2_ensureComponent(NeedsYouLink); new $$_kniLuoYsdeeN0C({ target: __sveltets_2_any(), props: {}});}
     { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!ws.currentId || backfilling || loading,"onclick":runBackfill,"title":`Re-derive the graph from every source`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} backfilling ? 'Refreshing…' : 'Refresh';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
if(!ws.currentId){
   { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {              "variant":`page`,"icon":`radar`,"title":`Add a workspace to get started`,"body":`Mission Control brings together the work in your workspace. Add a project folder to get started.`,"actionLabel":`Add workspace`,"actionIcon":`plus`,"onaction":() => (ui.newWorkspaceOpen = true),}});}
}else{
 { svelteHTML.createElement("div", {  "class":`mission-control`,});selectedId;
  
   { svelteHTML.createElement("div", {   "class":`tiles`,"aria-label":`Summary`,});
     { svelteHTML.createElement("div", { "class":`tile`,});
       { svelteHTML.createElement("span", { "class":`t-val`,});summary?.total ?? '—'; }
       { svelteHTML.createElement("span", { "class":`t-lbl`,});  }
     }
     { svelteHTML.createElement("div", { "class":`tile`,});
       { svelteHTML.createElement("span", { "class":`t-val accent`,});summary?.active ?? '—'; }
       { svelteHTML.createElement("span", { "class":`t-lbl`,});  }
     }
     { svelteHTML.createElement("div", {  "class":`tile`,});(summary?.needs_approval ?? 0) > 0;
       { svelteHTML.createElement("span", {  "class":`t-val`,});(summary?.needs_approval ?? 0) > 0;summary?.needs_approval ?? '—'; }
       { svelteHTML.createElement("span", { "class":`t-lbl`,});  }
     }
     { svelteHTML.createElement("div", { "class":`tile`,});
       { svelteHTML.createElement("span", { "class":`t-val`,});summary ? fmtCost(summary.total_cost) : '—'; }
       { svelteHTML.createElement("span", { "class":`t-lbl`,});  }
     }
   }

  
   { svelteHTML.createElement("div", { "class":`toolbar`,});
     { svelteHTML.createElement("div", { "class":`filters`,});
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":kindF,"aria-label":`Filter by kind`,});/*Ωignore_startΩ*/() => kindF = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let k of __sveltets_2_ensureArray(WORK_KINDS)){k; { svelteHTML.createElement("option", { "value":k,});KIND_LABEL[k]; }}
       }
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":statusF,"aria-label":`Filter by status`,});/*Ωignore_startΩ*/() => statusF = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let s of __sveltets_2_ensureArray(WORK_STATUSES)){s; { svelteHTML.createElement("option", { "value":s,});STATUS_LABEL[s]; }}
       }
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":riskF,"aria-label":`Filter by risk`,});/*Ωignore_startΩ*/() => riskF = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let r of __sveltets_2_ensureArray(RISK_LEVELS)){r; { svelteHTML.createElement("option", { "value":r,});RISK_LABEL[r]; }}
       }
       { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input search`,"type":`search`,"placeholder":`Filter by title…`,"bind:value":q,"oninput":onQInput,"aria-label":`Filter work items by title`,});/*Ωignore_startΩ*/() => q = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(hasFilters){ { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":clearFilters,});  }}
     }
     { svelteHTML.createElement("div", {         "class":`segmented view-toggle`,"role":`tablist`,"aria-label":`View`,"tabindex":-1,"onkeydown":onTabKey,});
       { svelteHTML.createElement("button", {        "role":`tab`,"tabindex":view === 'list' ? 0 : -1,"aria-selected":view === 'list',"onclick":() => (view = 'list'),});view === 'list';
         { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`format`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {        "role":`tab`,"tabindex":view === 'graph' ? 0 : -1,"aria-selected":view === 'graph',"onclick":() => (view = 'graph'),});view === 'graph';
         { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`share`,"size":12,}});} 
       }
     }
   }

  if(err && items.length > 0){
    
     { svelteHTML.createElement("div", {   "class":`banner-err`,"role":`alert`,});
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`be-text`,});    err; }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => ws.currentId && void reload(ws.currentId),});  }
     }
  }

  
   { svelteHTML.createElement("div", { "class":`mc-body`,});
     { svelteHTML.createElement("div", { "class":`mc-main`,});
      if(items.length === 0 && loading){
         { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {      "rows":6,"height":40,"label":`work items`,}});}
      } else if (items.length === 0 && err){
        
         { svelteHTML.createElement("div", { "class":`card`,});
           { const $$_etatSdaoL6C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL6C({ target: __sveltets_2_any(), props: {        "what":`Mission Control`,"error":err,"empty":true,loading,"onretry":() => ws.currentId && void reload(ws.currentId),}});}
         }
      } else if (items.length === 0 && !loading){
         { svelteHTML.createElement("div", { "class":`card`,});
           { const $$_etatSytpmE6C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE6C({ target: __sveltets_2_any(), props: {           "icon":`radar`,"title":hasFilters ? 'No work items match these filters' : 'No work items yet',"body":hasFilters
              ? 'Try clearing the filters, or refresh to re-derive the graph from every module.'
              : 'Mission Control unifies every agentic activity. Start a session, swarm, loop, workflow, review, or story — or press Refresh to materialize existing work.',"actionLabel":hasFilters ? 'Clear filters' : backfilling ? 'Refreshing…' : 'Refresh',"onaction":hasFilters ? clearFilters : runBackfill,}});}
         }
      } else if (view === 'list'){
         { const $$_tsiLmetIkroW5C = __sveltets_2_ensureComponent(WorkItemList); new $$_tsiLmetIkroW5C({ target: __sveltets_2_any(), props: {      items,"needsApproval":needsApprovalIds,selectedId,"onOpen":select,}});}
        if(items.length >= ITEM_LIMIT){
           { svelteHTML.createElement("p", { "class":`mc-cap`,});   ITEM_LIMIT;summary && summary.total > ITEM_LIMIT ? ` of ${summary.total}` : '';        }
        }
      } else if (loading && graph.nodes.length === 0){
         { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {      "rows":6,"height":40,"label":`the work graph`,}});}
      } else if (WorkGraphLazy.error){
         { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {      "what":`the work graph`,"error":WorkGraphLazy.error,"onretry":() => WorkGraphLazy.retry(),}});}
      } else if (WorkGraphLazy.component){
         { const $$_tnenopmoc_yzaLhparGkroW5C = __sveltets_2_ensureComponent(WorkGraphLazy.component); new $$_tnenopmoc_yzaLhparGkroW5C({ target: __sveltets_2_any(), props: {      graph,selectedId,"onOpen":select,"busy":loading,}});}
      }else{
          { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {  "what":`the work graph`,"loading":true,}});}
      }
     }

    if(selectedId){
      if(!detailSheet){
         { const $$_rediviDenaP4C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP4 = new $$_rediviDenaP4C({ target: __sveltets_2_any(), props: {              width:detailW,"storageKey":`mc.detailW`,"label":`Resize the detail pane`,"min":DETAIL_W.min,"max":DETAIL_W.max,"defaultWidth":DETAIL_W.default,"invert":true,}});/*Ωignore_startΩ*/() => detailW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP4.$$bindings = 'width';}
      }
       { const $$_div4 = svelteHTML.createElement("div", {            "class":`mc-detail`,"style":`--mc-detail-w:${detailW}px`,"role":detailSheet ? 'dialog' : undefined,"aria-modal":detailSheet ? 'true' : undefined,"aria-label":detailSheet ? 'Work item details' : undefined,});detailEl = $$_div4;
         { const $$_liateDmetIkroW5C = __sveltets_2_ensureComponent(WorkItemDetail); const $$_liateDmetIkroW5 = new $$_liateDmetIkroW5C({ target: __sveltets_2_any(), props: {           "wsId":ws.currentId ?? '',"id":selectedId,"onClose":() => select(null),"onOpen":select,onChange,}});detailPane = $$_liateDmetIkroW5;}
       }
    }
   }
 }
}
 PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MissionControlPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MissionControlPage__SvelteComponent_ = ReturnType<typeof MissionControlPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default MissionControlPage__SvelteComponent_;