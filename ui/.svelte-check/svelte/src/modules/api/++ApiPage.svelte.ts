///<reference types="svelte" />
;// The API module page (workbench archetype).
//
//   PageHeader: API · [Environment: Staging ▾] [Import…] [Sync with Git…] [New request]
//   ┌ left pane ─────────────────┬ main ─────────────────────────────────────┐
//   │ Collections|Automations|History │ tabs: requests + Environments/automation │
//   │ (things you pick)          │ request editor ↕ response                 │
//   └────────────────────────────┴───────────────────────────────────────────┘
//
// The left pane only lists things; whatever you EDIT (a request, the
// environments, an automation) opens in the main area. A brand-new
// workspace shows an onboarding empty state instead of an empty editor.

import { LIST_PANE, loadPaneWidth, paneResizer, RESIZE_TITLE_VERTICAL } from '../../lib/paneResizer';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { untrack, tick } from 'svelte';
import { scrollBehavior } from '../../lib/motion';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import RequestBuilder from './RequestBuilder.svelte';
import ResponseViewer from './ResponseViewer.svelte';
import CollectionsTree from './CollectionsTree.svelte';
import HistoryList from './HistoryList.svelte';
import AutomationsView from './AutomationsView.svelte';
import AutomationEditor from './AutomationEditor.svelte';
import EnvironmentsView from './EnvironmentsView.svelte';
import ImportDialog from './ImportDialog.svelte';
import GitSyncDialog from './GitSyncDialog.svelte';
import MethodTag from './MethodTag.svelte';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { apiStream } from '../../lib/stores/apiStream.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { startMouseDrag } from '../../lib/dragCursor';
import { viewport } from '../../lib/stores/viewport.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { registry } from '../../lib/commands.svelte';
import { keyContext } from '../../lib/keys';
import { router } from '../../lib/router.svelte';
import type { Id } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  type Side = 'collections' | 'automations' | 'history';
  const SIDE_KEY = 'otto_api_side';
  const SIDES: { id: Side; label: string; icon: IconName }[] = [
    { id: 'collections', label: 'Collections', icon: 'folder' },
    { id: 'automations', label: 'Automations', icon: 'zap' },
    { id: 'history', label: 'History', icon: 'clock' },
  ];
  function readSide(): Side {
    try {
      const v = localStorage.getItem(SIDE_KEY);
      return v === 'automations' || v === 'history' ? v : 'collections';
    } catch {
      return 'collections';
    }
  }
  let side = $state<Side>(readSide());
  function setSide(s: Side): void {
    side = s;
    try { localStorage.setItem(SIDE_KEY, s); } catch { /* per-device convenience only */ }
  }
  // What the main area shows.
  type View = { kind: 'request' } | { kind: 'environments'; envId: Id | null } | { kind: 'automation'; id: Id };
  let view = $state<View>({ kind: 'request' });
  let builderTab = $state<'params' | 'headers' | 'body' | 'auth' | 'settings' | 'scripts' | 'docs'>('params');
  // Phone: push navigation between the list and the editor.
  let phonePane = $state<'list' | 'main'>('main');

  let automationEditor: { approveLeave: () => Promise<boolean> } | undefined = $state();
  let environmentEditor: { approveLeave: () => Promise<boolean>; create: () => Promise<void> } | undefined = $state();
  let viewTransition = 0;
  async function changeView(next: View): Promise<boolean> {
    if (view.kind === 'automation' && next.kind === 'automation' && view.id === next.id) return true;
    if (view.kind === 'environments' && next.kind === 'environments' && (next.envId === null || view.envId === next.envId)) return true;
    const generation = ++viewTransition;
    if (view.kind === 'automation' && automationEditor && !(await automationEditor.approveLeave())) return false;
    if (view.kind === 'environments' && environmentEditor && !(await environmentEditor.approveLeave())) return false;
    if (generation !== viewTransition) return false;
    view = next;
    phonePane = 'main';
    return true;
  }
  async function showRequest(): Promise<void> {
    await changeView({ kind: 'request' });
  }
  async function switchRequestTab(i: number): Promise<void> {
    const id = apiClient.tabs[i]?.tabId;
    if (!(await changeView({ kind: 'request' }))) return;
    const current = apiClient.tabs.findIndex((tab) => tab.tabId === id);
    if (current >= 0) apiClient.switchTab(current);
  }
  /** New request: reuse the active tab while it is an untouched blank
   *  draft (no pile of empty tabs), else open a new one; focus the URL. */
  async function newRequest(): Promise<void> {
    if (!(await changeView({ kind: 'request' }))) return;
    started = true;
    const d = apiClient.draft;
    if (d.requestId || apiClient.isDirty(d)) apiClient.newDraft();
    showRequest();
    requestAnimationFrame(() => document.querySelector<HTMLInputElement>('input[aria-label="Request URL"]')?.focus());
  }
  /** Protect unsent scratch work as well as changes to saved requests. */
  async function closeRequestTab(i: number): Promise<void> {
    const t = apiClient.tabs[i];
    if (t && apiClient.isDirty(t)) {
      const ok = await confirmer.ask(`“${apiClient.tabLabel(t)}” has unsaved changes. Close the tab and discard them?${t.requestId ? " The saved request is kept." : " This unsaved draft will be removed."}`, {
        title: 'Discard changes?',
        confirmLabel: 'Discard changes',
        danger: true,
      });
      if (!ok) return;
    }
    // The tab list may have shifted while the dialog was open.
    const at = t?.tabId ? apiClient.tabs.findIndex((x) => x.tabId === t.tabId) : i;
    if (at >= 0) apiClient.closeTab(at);
  }
  async function openEnvironments(envId: Id | null = null): Promise<void> {
    await changeView({ kind: 'environments', envId });
  }
  async function openAutomation(id: Id): Promise<void> {
    await changeView({ kind: 'automation', id });
  }

  // Load everything when the workspace changes. Keyed on the workspace ONLY:
  // loadAll's synchronous prologue reads (and rewrites) the open tabs, which
  // must not become dependencies of this effect.
  $effect(() => {
    if (ws.currentId) {
      untrack(() => {
        view = { kind: 'request' };
        void apiClient.loadAll();
        void apiClient.loadAutomations();
      });
    }
  });

  // Agent UI control: an agent editing/sending a request brings the request
  // editor to the front (apiClient.showRequestView, lib/uiCommands/api.ts).
  $effect(() => {
    if (apiClient.requestViewTick > 0) untrack(() => showRequest());
  });

  // Open on an item: the first time this device opens a workspace's API client
  // (no persisted tab slot yet — e.g. requests came from Git sync or another
  // Mac), land on the most recently edited saved request instead of an empty
  // "Untitled" draft. Once tabs have been persisted (including "I closed them
  // all") they are restored as the user left them; this never overrides that.
  // The slot key mirrors apiClient's `otto_api_tabs_v1:<workspace>`.
  let autoOpenedFor: string | null = null;
  $effect(() => {
    const wid = ws.currentId;
    const reqs = apiClient.requests;
    // A `#/api/<id>` link wins over the "most recent" landing.
    if (routePending) return;
    if (!wid || autoOpenedFor === wid || apiClient.loading || reqs.length === 0) return;
    if (reqs[0].workspace_id !== wid) return;
    untrack(() => {
      autoOpenedFor = wid;
      let persisted = true;
      try { persisted = localStorage.getItem(`otto_api_tabs_v1:${wid}`) !== null; } catch { /* unknown → leave as is */ }
      const only = apiClient.tabs.length === 1 ? apiClient.tabs[0] : null;
      if (persisted || !only || only.requestId || apiClient.isDirty(only)) return;
      const latest = reqs.reduce((a, b) => ((b.updated_at ?? '') > (a.updated_at ?? '') ? b : a));
      void apiClient.openRequest(latest.id);
    });
  });

  // ── URL ↔ open request ─────────────────────────────────────────────────────
  // The URL carries the saved request in the editor (`#/api/<requestId>`) so a
  // reload / share / link lands on it. Route → page: once the list knows the
  // id, open it in the request editor; page → route: switching tabs or opening
  // a request rewrites the URL in place (an unsaved draft or another view is
  // plain `#/api`). Until a routed id is resolved the page leaves the URL alone,
  // so restoring the last tabs can't overwrite the link being opened.
  let routePending = $state<string | null>(untrack(() => (router.parts[0] === 'api' ? (router.parts[1] ?? null) : null)));
  $effect(() => {
    const [mod, id] = router.parts;
    if (mod !== 'api') return;
    if (!id) {
      routePending = null;
      return;
    }
    const reqs = apiClient.requests;
    const loaded = !apiClient.loading && (reqs.length === 0 || reqs[0].workspace_id === ws.currentId);
    if (!reqs.some((r) => r.id === id)) {
      // Unknown once loaded (deleted / another workspace): let the page own the URL.
      if (loaded) routePending = null;
      return;
    }
    untrack(() => {
      if (view.kind === 'request' && apiClient.draft.requestId === id) {
        routePending = null;
        return;
      }
      void changeView({ kind: 'request' })
        .then((ok) => (ok ? apiClient.openRequest(id) : false))
        .finally(() => (routePending = null));
    });
  });
  let routedReq: string | null = null;
  $effect(() => {
    const id = view.kind === 'request' ? (apiClient.draft.requestId ?? null) : null;
    if (routePending) return;
    const was = routedReq;
    routedReq = id;
    if (!id && !was) return;
    untrack(() => {
      if (router.module === 'api' && (router.parts[1] ?? null) !== id) router.replace(id ? `api/${id}` : 'api');
    });
  });

  // ── onboarding ─────────────────────────────────────────────────────────────
  // A workspace with nothing in it (no saved requests, collections, history or
  // edited tab) opens on "Create your first request", not an empty editor.
  let started = $state(false);
  const onboarding = $derived(
    !started &&
      !apiClient.listsPending &&
      !apiClient.loadError &&
      apiClient.collections.length === 0 &&
      apiClient.requests.length === 0 &&
      apiClient.history.length === 0 &&
      apiClient.tabs.length === 1 &&
      !apiClient.isDirty(apiClient.tabs[0]),
  );
  function tryExample(): void {
    started = true;
    apiClient.newDraft();
    apiClient.draft = { ...apiClient.draft, name: 'Example: get JSON', method: 'GET', url: 'https://httpbin.org/json' };
    showRequest();
  }

  // ── environment switcher (header) ─────────────────────────────────────────
  async function newEnvironment(): Promise<void> {
    if (view.kind === 'environments' && environmentEditor) return environmentEditor.create();
    const name = await confirmer.promptText('Name', { title: 'New environment', confirmLabel: 'Create', initial: '' });
    if (!name) return;
    const saved = await apiClient.saveEnvironment({ name }, undefined);
    if (saved) openEnvironments(saved.id);
  }
  function envMenu(e: MouseEvent): void {
    const canEdit = ws.myRole !== 'viewer';
    const items: MenuItem[] = apiClient.environments.map((env) => ({
      label: env.name,
      icon: env.is_active ? 'check' : 'dot',
      disabled: !canEdit,
      action: () => void apiClient.activateEnvironment(env.id),
    }));
    if (items.length) items.push({ separator: true });
    items.push({ label: 'Manage environments…', icon: 'globe', action: () => openEnvironments() });
    if (canEdit) items.push({ label: 'New environment…', icon: 'plus', action: () => void newEnvironment() });
    ctxMenu.show(e, items);
  }

  let importOpen = $state(false);
  let gitOpen = $state(false);

  // ── drag-to-resize: sidebar width + builder height (both persisted) ────────
  // The sidebar is the shared list pane (PaneDivider + LIST_PANE); it keeps the
  // old storage key so a width saved before the switch carries over (clamped).
  // Overlay cursor + one size write per frame + one localStorage write on
  // release (lib/dragCursor.ts) — no per-mousemove persist.
  const SIDE_W_KEY = 'otto_api_side_width';
  let sideW = $state(loadPaneWidth(SIDE_W_KEY, LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let builderEl: HTMLDivElement | null = $state(null);
  function startBuilderResize(e: MouseEvent): void {
    const startY = e.clientY;
    // Until the first drag the height is CSS-driven — seed from the rendered
    // height so the divider doesn't jump on grab.
    const startH = ui.apiBuilderHeight || builderEl?.offsetHeight || 300;
    startMouseDrag(e, {
      cursor: 'row-resize',
      onMove: (ev) => ui.setApiBuilderHeight(startH + (ev.clientY - startY), false),
      onEnd: () => ui.setApiBuilderHeight(ui.apiBuilderHeight),
    });
  }
  // ── keyboard + ⌘K ─────────────────────────────────────────────────────────
  // ⌘T → new request tab and ⌘D → duplicate, instead of the global
  // new-session / split chords (which would act on the Agents page unseen).
  $effect(() => {
    keyContext.pageChords = (e) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || ui.overlayOpen) return false;
      const k = e.key.toLowerCase();
      if (k === 't') { e.preventDefault(); newRequest(); return true; }
      if (k === 'd') { e.preventDefault(); apiClient.duplicateDraft(); showRequest(); return true; }
      return false;
    };
    return () => { keyContext.pageChords = null; };
  });
  $effect(() => {
    return registry.register('api', [
      { id: 'api.new', title: 'New API request', group: 'API', shortcut: '⌘T', keywords: 'http rest call postman', run: newRequest },
      { id: 'api.duplicate', title: 'Duplicate API request', group: 'API', shortcut: '⌘D', keywords: 'copy clone', run: () => { apiClient.duplicateDraft(); showRequest(); } },
      { id: 'api.import', title: 'Import into API…', group: 'API', keywords: 'curl postman openapi swagger har', run: () => (importOpen = true) },
      { id: 'api.envs', title: 'Manage API environments', group: 'API', keywords: 'variables base_url staging production secrets', run: () => openEnvironments() },
      { id: 'api.history', title: 'Show API history', group: 'API', keywords: 'recent sent requests', run: () => { setSide('history'); phonePane = 'list'; } },
    ]);
  });

  const activeEnvName = $derived(apiClient.activeEnv?.name ?? null);
  const automationName = $derived(view.kind === 'automation' ? (apiClient.automations.find((a) => a.id === (view as { id: Id }).id)?.name ?? 'Automation') : '');
  let responsePane = $state<HTMLElement | null>(null);
  let previousResultTab: string | undefined;
  let previousResult: unknown;
  $effect(() => {
    const tab = apiClient.draft?.tabId ?? undefined;
    const streamResult = apiStream.tabId === tab && (apiStream.status === 'open' || apiStream.status === 'error') ? apiStream.status : null;
    const result = apiClient.lastError || apiClient.lastResponse || streamResult;
    const changed = tab === previousResultTab && result && result !== previousResult;
    previousResultTab = tab;
    previousResult = result;
    if (changed && viewport.isPhone) untrack(() => {
      void tick().then(() => {
        if (apiClient.draft?.tabId === tab && viewport.isPhone) responsePane?.scrollIntoView({ behavior: scrollBehavior(), block: 'start' });
      });
    });
  });
  function showTimeoutSettings(): void {
    builderTab = 'settings';
    if (viewport.isPhone) void tick().then(() => builderEl?.scrollIntoView({ behavior: scrollBehavior(), block: 'start' }));
  }

  const showList = $derived(!viewport.isPhone || phonePane === 'list');
  const showMain = $derived(!viewport.isPhone || phonePane === 'main');
;
async () => {

 { svelteHTML.createElement("div", { "class":`api-root`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":`API`,"subtitle":`Requests run from Otto’s daemon, not the browser`,leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if(viewport.isPhone && phonePane === 'main' && !onboarding){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (phonePane = 'list'),"aria-label":`Show saved requests`,"title":`Show saved requests`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});} }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`btn small env-btn`,...__sveltets_2_empty({"data-keep":true}),"onclick":envMenu,"aria-haspopup":`menu`,"aria-label":`Environment: ${activeEnvName ?? 'none'}`,"title":`Environment: where ${'{{variables}}'} get their values`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});}
        if(!viewport.isPhone){ { svelteHTML.createElement("span", { "class":`env-k`,});  }}
         { svelteHTML.createElement("span", {  "class":`env-v`,});!activeEnvName;activeEnvName ?? 'None'; }
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
       }
       { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-icon":`download`}),...__sveltets_2_empty({"data-overflow":`0`}),"onclick":() => (importOpen = true),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}  }
       { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-icon":`branch`}),...__sveltets_2_empty({"data-overflow":`-1`}),"onclick":() => (gitOpen = true),"disabled":ws.myRole === 'viewer',"title":ws.myRole === 'viewer' ? 'Viewers can’t sync API collections with Git' : undefined,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}   }
      if(!onboarding){
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":newRequest,"title":`New request (⌘T)`,"aria-label":`New request`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}if(!viewport.isPhone){ } }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
    if(onboarding){
       { svelteHTML.createElement("div", { "class":`onboard`,});
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`send`,"title":`Create your first request`,"body":'Call any HTTP API and see the status, headers and body. Save requests into collections, keep base URLs and tokens in environments as {{variables}}, and chain requests into automated checks.',"actionLabel":`New request`,"actionIcon":`plus`,"onaction":newRequest,}});
           { svelteHTML.createElement("div", { "class":`onboard-more`,});
             { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (importOpen = true),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});}      }
             { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":tryExample,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":13,}});}    }
           }
           { svelteHTML.createElement("p", { "class":`onboard-note`,});                        }
         EmptyState}
       }
    }else{
      
       { svelteHTML.createElement("div", {   "class":`api-page`,"aria-busy":apiClient.listsPending,});
        if(showList){
           { svelteHTML.createElement("aside", {    "class":`api-side`,"aria-label":`Collections, automations and history`,});__sveltets_2_ensureType(String, Number, viewport.isPhone ? null : `${sideW}px`);
             { svelteHTML.createElement("div", {     "class":`segmented side-seg`,"role":`tablist`,"aria-label":`Show`,});
                 for(let s of __sveltets_2_ensureArray(SIDES)){s.id;
                 { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":side === s.id,"tabindex":side === s.id ? 0 : -1,"onclick":() => setSide(s.id),"onkeydown":onTabKey,"title":s.label,});side === s.id;
                   { svelteHTML.createElement("span", { "class":`seg-label`,});s.label; }
                 }
              }
             }
             { svelteHTML.createElement("div", { "class":`side-body`,});
              if(side === 'collections'){
                 { const $$_eerTsnoitcelloC5C = __sveltets_2_ensureComponent(CollectionsTree); new $$_eerTsnoitcelloC5C({ target: __sveltets_2_any(), props: {  "onopen":showRequest,}});}
              } else if (side === 'automations'){
                 { const $$_weiVsnoitamotuA5C = __sveltets_2_ensureComponent(AutomationsView); new $$_weiVsnoitamotuA5C({ target: __sveltets_2_any(), props: {    "selectedId":view.kind === 'automation' ? view.id : null,"onselect":openAutomation,}});}
              }else{
                 { const $$_tsiLyrotsiH5C = __sveltets_2_ensureComponent(HistoryList); new $$_tsiLyrotsiH5C({ target: __sveltets_2_any(), props: {  "onopen":showRequest,}});}
              }
             }
           }

          if(!viewport.isPhone){
             { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:sideW,"storageKey":SIDE_W_KEY,"label":`Resize the sidebar`,}});/*Ωignore_startΩ*/() => sideW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
          }
        }

        if(showMain){
           { svelteHTML.createElement("div", { "class":`api-main`,});
             { svelteHTML.createElement("div", { "class":`req-tabs`,});
               { svelteHTML.createElement("div", { "class":`req-tablist scroll-thin`,});
                    for(let t of __sveltets_2_ensureArray(apiClient.tabs)){let i = 1;t.tabId ?? i;
                  const active = view.kind === 'request' && apiClient.activeTab === i;
                  const st = apiClient.tabStatus(t.tabId);
                   { svelteHTML.createElement("div", {  "class":`req-tab`,});active;
                     { svelteHTML.createElement("button", {         "class":`req-tab-main`,"aria-label":`${apiClient.tabLabel(t)}${apiClient.isDirty(t) ? ', unsaved changes' : ''}${st === 'ok' ? ', last send succeeded' : st === 'fail' ? ', last send failed' : st === 'sending' ? ', sending' : ''}`,"aria-current":active ? 'true' : undefined,"onclick":() => switchRequestTab(i),"title":apiClient.isDirty(t) ? `${apiClient.tabLabel(t)} (unsaved changes)` : apiClient.tabLabel(t),});
                       { const $$_gaTdohteM8C = __sveltets_2_ensureComponent(MethodTag); new $$_gaTdohteM8C({ target: __sveltets_2_any(), props: {  "method":t.kind === 'http' || t.kind === 'sse' ? t.method : t.kind === 'grpc' ? 'gRPC' : 'WS',}});}
                       { svelteHTML.createElement("span", { "class":`req-tab-label`,});apiClient.tabLabel(t); }
                      if(apiClient.isDirty(t)){ { svelteHTML.createElement("span", {   "class":`req-tab-dirty`,"aria-hidden":`true`,});  }}
                      if(st === 'sending'){ { svelteHTML.createElement("span", {       "class":`spinner`,"style":`--spinner-size: 9px`,"aria-hidden":`true`,"title":`Sending…`,}); }
                      } else if (st){ { svelteHTML.createElement("span", {     "class":`req-tab-status ${st}`,"aria-label":st === 'ok' ? 'Last send succeeded' : 'Last send failed',"title":st === 'ok' ? 'Last send succeeded' : 'Last send failed',}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":st === 'ok' ? 'check' : 'x',"size":12,}});} }}
                     }
                     { svelteHTML.createElement("button", {       "class":`req-tab-close icon-btn`,"title":`Close tab ${apiClient.tabLabel(t)}`,"aria-label":`Close tab ${apiClient.tabLabel(t)}`,"onclick":() => void closeRequestTab(i),}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                   }
                }
                if(view.kind !== 'request'){
                   { svelteHTML.createElement("div", { "class":`req-tab active special`,});
                     { svelteHTML.createElement("button", {   "class":`req-tab-main`,"aria-current":`true`,});
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":view.kind === 'environments' ? 'globe' : 'zap',"size":12,}});}
                       { svelteHTML.createElement("span", { "class":`req-tab-label`,});view.kind === 'environments' ? 'Environments' : automationName; }
                     }
                     { svelteHTML.createElement("button", {       "class":`req-tab-close icon-btn`,"title":`Close`,"aria-label":`Close ${view.kind === 'environments' ? 'environments' : 'automation'}`,"onclick":showRequest,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                   }
                }
               }
               { svelteHTML.createElement("button", {       "class":`req-tab-new icon-btn`,"title":`New request (⌘T)`,"aria-label":`New request tab`,"onclick":newRequest,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
             }

            if(view.kind === 'environments'){
              view.envId; {
                 { const $$_weiVstnemnorivnE4C = __sveltets_2_ensureComponent(EnvironmentsView); const $$_weiVstnemnorivnE4 = new $$_weiVstnemnorivnE4C({ target: __sveltets_2_any(), props: {    "initialId":view.envId,}});environmentEditor = $$_weiVstnemnorivnE4;}
              }
            } else if (view.kind === 'automation'){
              view.id; {
                 { const $$_rotidEnoitamotuA4C = __sveltets_2_ensureComponent(AutomationEditor); const $$_rotidEnoitamotuA4 = new $$_rotidEnoitamotuA4C({ target: __sveltets_2_any(), props: {      "automationId":view.id,"ondeleted":showRequest,}});automationEditor = $$_rotidEnoitamotuA4;}
              }
            }else{
               { const $$_div4 = svelteHTML.createElement("div", {      "class":`builder-pane`,});builderEl = $$_div4;__sveltets_2_ensureType(String, Number, !viewport.isPhone && ui.apiBuilderHeight > 0 ? `${ui.apiBuilderHeight}px` : null);__sveltets_2_ensureType(String, Number, !viewport.isPhone && ui.apiBuilderHeight > 0 ? 'none' : null);
                 { const $$_redliuBtseuqeR5C = __sveltets_2_ensureComponent(RequestBuilder); const $$_redliuBtseuqeR5 = new $$_redliuBtseuqeR5C({ target: __sveltets_2_any(), props: {   tab:builderTab,}});/*Ωignore_startΩ*/() => builderTab = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_redliuBtseuqeR5.$$bindings = 'tab';}
               }
              if(!viewport.isPhone){
                
                 {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ onDragStart: startBuilderResize, value: ui.apiBuilderHeight || builderEl?.offsetHeight || 300, min: 180, max: Math.round((typeof window === 'undefined' ? 900 : window.innerHeight) * 0.8), orientation: 'horizontal', step: 10, bigStep: 40, onChange: (h) => ui.setApiBuilderHeight(h), onReset: () => ui.resetApiBuilderHeight(), text: (v) => `${Math.round(v)} pixels tall` })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`resizer row`,"role":`separator`,"aria-label":`Resize the request and response panes`,"title":RESIZE_TITLE_VERTICAL,}); }}
              }
               { const $$_section4 = svelteHTML.createElement("section", {     "class":`resp-pane`,"aria-label":`Response`,});responsePane = $$_section4;
                 { const $$_reweiVesnopseR5C = __sveltets_2_ensureComponent(ResponseViewer); new $$_reweiVesnopseR5C({ target: __sveltets_2_any(), props: {  "onsettings":showTimeoutSettings,}});}
               }
            }
           }
        }
       }
    }
   PageBody}
 }

if(importOpen){
   { const $$_golaiDtropmI0C = __sveltets_2_ensureComponent(ImportDialog); new $$_golaiDtropmI0C({ target: __sveltets_2_any(), props: {      "initial":`curl`,"onclose":() => (importOpen = false),"onimported":() => { started = true; showRequest(); },}});}
}
if(gitOpen){
   { const $$_golaiDcnyStiG0C = __sveltets_2_ensureComponent(GitSyncDialog); new $$_golaiDcnyStiG0C({ target: __sveltets_2_any(), props: {  "onclose":() => (gitOpen = false),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ApiPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ApiPage__SvelteComponent_ = ReturnType<typeof ApiPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default ApiPage__SvelteComponent_;