///<reference types="svelte" />
;
import { NO_WORKSPACE } from '../../lib/labels';
import { toastError } from '../../lib/toastError';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { actionOperation, readOperation } from './permissions';
// k9s-like cluster workspace. Top bar: cluster switcher · namespace combobox
// · text filter · refresh / auto-refresh · k9s · key hints. Left: kinds rail
// (Argo Rollouts / ArgoCD Apps only when the capability probe says so; a
// <select> on phones). Center: ResourceTable. Right: ResourceDrawer behind a
// hand-rolled splitter (a full sheet on phones). Cluster / kind / selected
// row live in the URL; namespace + filter + cache live in the store.
import { untrack } from 'svelte';
import { router } from '../../lib/router.svelte';
import { k8s } from '../../lib/stores/k8s.svelte';
import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { toasts } from '../../lib/toast.svelte';
import { keyContext } from '../../lib/keys';
import { api } from '../../lib/api/client';
import { k8sApi } from '../../lib/api/k8s';
import type { K8sCluster, K8sResourceKind, K8sRow } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Modal from '../../lib/components/Modal.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import NamespacePicker from './NamespacePicker.svelte';
import ResourceTable from './ResourceTable.svelte';
import ResourceDrawer from './ResourceDrawer.svelte';
import ScaleDialog from './ScaleDialog.svelte';
import SyncDialog from './SyncDialog.svelte';
import InstallPanel from './InstallPanel.svelte';
import type { ActionDef } from './actions';
import { actionsFor, runAction } from './actions';
import { formatAge, kindDef, visibleKinds } from './k8s-util';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import K8sViewSwitch from './K8sViewSwitch.svelte';
import { paneResizer, pxWide, RESIZE_TITLE } from '../../lib/paneResizer';
import { monitorPath } from '../../lib/k8s/viewState';
import { SINGLE_KEYS, SINGLE_KEYS_PREF, inWorkspace } from './workspaceKeys';
import { lsGet, lsSet } from '../../lib/storage';
import SettingToggle from '../settings/SettingToggle.svelte';
;

  interface Props {
    cluster: K8sCluster;
  };function $$render() {
  const filterBox/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", { "class":`filter`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
     { const $$_input1 = svelteHTML.createElement("input", {                 "dir":`ltr`,"class":`filter-in`,"placeholder":`Filter rows…`,"bind:value":k8s.filter,"aria-label":`Filter rows`,"aria-keyshortcuts":`/`,"title":`Filter rows (/)`,...__sveltets_2_empty({"data-testid":`k8s-filter`}),});filterEl = $$_input1;/*Ωignore_startΩ*/() => k8s.filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
    if(k8s.filter){ { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (k8s.filter = ''),"aria-label":`Clear filter`,"title":`Clear filter`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }}
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { cluster }: Props = $props();

  function can(operation: string, ns = k8s.selected?.ns || k8s.namespace): boolean {
    return resourceAccess.can('k8s_cluster', cluster.id, operation, 'kubernetes',
      ['exec', 'apply', 'scale', 'restart', 'delete', 'k9s'].includes(operation) ? 'edit' : 'view', ns ? `namespace:${ns}` : undefined);
  }
  function canAction(action: ActionDef, row: K8sRow): boolean {
    const ns = action.id.startsWith('argocd_') ? '' : row.namespace;
    return can(actionOperation(action.id), ns);
  }
  const canEdit = $derived(['exec', 'apply', 'scale', 'restart', 'delete'].some(op => can(op)));
  const canK9s = $derived(resourceAccess.can('k8s_cluster', cluster.id, 'k9s', 'kubernetes', 'edit'));
  // The distinct namespaces on screen, as one string: the 10 s list refresh
  // replaces `k8s.rows`, but the load effect below re-runs only when the SET
  // of namespaces changed, not on every refresh of 5k rows.
  const rowNamespaces = $derived.by(() => {
    const set = new Set<string>();
    for (const row of k8s.rows) if (row.namespace) set.add(row.namespace);
    return [...set].sort().join('\n');
  });
  $effect(() => {
    void resourceAccess.load('k8s_cluster', cluster.id);
    const namespaces = new Set([k8s.namespace, k8s.selected?.ns, ...(rowNamespaces ? rowNamespaces.split('\n') : [])]);
    for (const ns of namespaces) if (ns) void resourceAccess.load('k8s_cluster', cluster.id, `namespace:${ns}`);
  });
  $effect(() => {
    const access = resourceAccess.get('k8s_cluster', cluster.id);
    if (access?.mode === 'enforced' && !k8s.namespace && k8s.namespaces.length && !can(readOperation(k8s.kind), '')) {
      const first = k8s.namespaces[0].name;
      untrack(() => k8s.setNamespace(first));
    }
  });
  const isAdmin = $derived(auth.can('kubernetes', 'admin'));
  const kinds = $derived(visibleKinds(k8s.caps));
  const kind = $derived(k8s.kind);
  const def = $derived(kindDef(kind));
  const clusterScoped = $derived(!!def.clusterScoped);
  const allNs = $derived(k8s.namespace === '' && !clusterScoped);
  const sel = $derived(k8s.selected);
  const selRow = $derived(k8s.selectedRow);
  const rowsForKey = $derived(k8s.rowsKey === k8s.currentKey ? k8s.rows : []);

  let filterEl = $state<HTMLInputElement | null>(null);
  let nsPicker = $state<NamespacePicker | null>(null);
  let hintsOpen = $state(false);
  /** The workspace root: shortcuts act only while focus is inside it. */
  let wspEl: HTMLDivElement | undefined = $state();
  /** Single-key shortcuts on/off (per device; the ? sheet has the switch). */
  let singleKeys = $state(lsGet(SINGLE_KEYS_PREF) !== '0');
  let k9sInstallOpen = $state(false);
  let k9sOpening = $state(false);
  let scaleFor = $state<{ row: K8sRow; def: ActionDef } | null>(null);
  let syncFor = $state<{ row: K8sRow; def: ActionDef } | null>(null);
  /** Set by the `s` shortcut so the drawer's Terminal tab opens the shell at once. */
  let autoExec = $state(false);

  // Drawer width (desktop) — persisted; clamped so neither pane collapses.
  const DRAWER_KEY = 'otto_k8s_drawer_w';
  let drawerW = $state(
    (() => {
      try {
        const v = Number(localStorage.getItem(DRAWER_KEY));
        return v >= 320 ? v : 520;
      } catch {
        return 520;
      }
    })(),
  );
  let resizing = $state(false);
  let bodyEl = $state<HTMLDivElement | null>(null);
  let bodyWidth = $state(0);
  // A useful table needs 280px beside the 168px kind rail and 320px detail.
  const detailSheet = $derived(viewport.isPhone || (bodyWidth > 0 && bodyWidth < 780));
  // Also ≤ 55% of the row: DockedDrawer caps its column there, and the
  // splitter's value must match what's drawn.
  const maxDrawerW = $derived(Math.max(320, Math.min(bodyWidth - 168 - 280 - 6, Math.floor((bodyWidth - 168) * 0.55))));
  const visibleDrawerW = $derived(Math.min(drawerW, maxDrawerW));
  function saveDrawerWidth(): void {
    try { localStorage.setItem(DRAWER_KEY, String(Math.round(drawerW))); } catch { /* ignore */ }
  }
  function startResize(e: PointerEvent): void {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = visibleDrawerW;
    const rtl = getComputedStyle(e.currentTarget as HTMLElement).direction === 'rtl';
    const onMove = (ev: PointerEvent): void => {
      // Pointer motion follows the physical side occupied by the drawer.
      drawerW = Math.max(320, Math.min(maxDrawerW, startW + (ev.clientX - startX) * (rtl ? 1 : -1)));
    };
    const onUp = (): void => {
      resizing = false;
      saveDrawerWidth();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  // --- routing helpers ----------------------------------------------------------
  const base = $derived(`kubernetes/${encodeURIComponent(cluster.id)}`);
  function goKind(k: K8sResourceKind): void {
    router.go(`${base}/${k}`);
  }
  /** `exec` = open the pod's shell straight away (the `s` key / "Shell" menu
   *  item). One-shot: every other open clears it, so moving through rows with
   *  j/k while the drawer sits on Terminal never opens a shell unasked. */
  function openRow(r: K8sRow, tab?: K8sDrawerTab, exec = false): void {
    autoExec = exec;
    if (tab) k8s.drawerTab = tab;
    router.go(`${base}/${kind}/${encodeURIComponent(r.namespace || '-')}/${encodeURIComponent(r.name)}`);
  }
  /** From a workload drawer to one of its pods: switch the table to Pods and
   *  route to that pod so its own drawer (Logs / Terminal / Metrics) opens. */
  function openPod(podNs: string, pod: string, tab?: K8sDrawerTab): void {
    autoExec = false;
    k8s.drawerTab = tab ?? 'overview';
    if (podNs && podNs !== k8s.namespace && k8s.namespace !== '') k8s.setNamespace(podNs);
    router.go(`${base}/pods/${encodeURIComponent(podNs || '-')}/${encodeURIComponent(pod)}`);
  }
  function closeDrawer(): void {
    autoExec = false;
    // A single click selects WITHOUT touching the URL, so navigating to the
    // kind route alone is a no-op when nothing deeper was ever routed to —
    // clear the selection explicitly as well.
    k8s.select(null);
    router.go(`${base}/${kind}`);
  }
  /** Console → Monitor with this workload's row expanded (K-2). */
  function openInMonitor(wlNs: string, workload: string): void {
    k8s.saveMonitorUi(cluster.id, { expanded: `${wlNs}/${workload}`, filter: workload });
    router.go(monitorPath(cluster.id));
  }
  // KS-4: after an action the drawer re-reads its detail once the change
  // has had a moment to land (a restart / scale shows in the next read).
  let drawerNonce = $state(0);
  let drawerTimer: ReturnType<typeof setTimeout> | null = null;
  function afterAction(): void {
    void k8s.loadResources(true);
    if (drawerTimer) clearTimeout(drawerTimer);
    drawerTimer = setTimeout(() => {
      drawerTimer = null;
      drawerNonce += 1;
    }, 1500);
  }
  $effect(() => () => {
    if (drawerTimer) clearTimeout(drawerTimer);
  });
  // K-1: the filter + drawer tab are remembered per cluster (and per kind for
  // the filter), so they survive a module switch, the Monitor and a reload.
  $effect(() => {
    void k8s.filter;
    void k8s.drawerTab;
    untrack(() => k8s.rememberConsoleUi());
  });
  function switchCluster(id: string): void {
    if (id && id !== cluster.id) router.go(`kubernetes/${encodeURIComponent(id)}`);
  }

  // --- data loading ---------------------------------------------------------------
  // Reload whenever (cluster, kind, namespace) changes; the load itself is
  // untracked so store writes during the fetch can't re-trigger it.
  $effect(() => {
    void k8s.currentKey;
    if (!k8s.clusterId) return;
    untrack(() => void k8s.loadResources());
  });
  $effect(() => {
    k8s.startAutoRefresh();
    return () => k8s.stopAutoRefresh();
  });
  // Kinds that need a capability that's gone (or a CRD kind on a cluster
  // without it) fall back to pods.
  $effect(() => {
    const ks = kinds;
    const k = kind;
    if (k8s.caps && !ks.some((x) => x.id === k)) untrack(() => goKind('pods'));
  });

  // --- actions --------------------------------------------------------------------
  async function doAction(a: ActionDef, r: K8sRow): Promise<void> {
    if (!canAction(a, r)) return;
    if (a.needs === 'scale') {
      scaleFor = { row: r, def: a };
      return;
    }
    if (a.needs === 'sync') {
      syncFor = { row: r, def: a };
      return;
    }
    const resp = await runAction(cluster.id, kind, r, a);
    if (resp?.output && a.id === 'rollout_status') toasts.info('Rollout status', resp.output);
    if (resp) afterAction();
  }

  function rowMenu(e: MouseEvent | KeyboardEvent, r: K8sRow): void {
    const items: MenuItem[] = [
      { label: 'Details', icon: 'info', action: () => openRow(r, 'overview') },
      { label: 'Manifest (YAML)', icon: 'file', action: () => openRow(r, 'manifest') },
      { label: 'Describe', icon: 'note', action: () => openRow(r, 'describe') },
    ];
    if (kind === 'pods') {
      if (can('logs', r.namespace)) items.push({ label: 'Logs', icon: 'file', action: () => openRow(r, 'logs') });
      if (can('exec', r.namespace)) items.push({ label: 'Shell (exec)', icon: 'terminal', action: () => openRow(r, 'terminal', true) });
    } else if (r.extra?.selector) {
      items.push({ label: 'Pods', icon: 'box', action: () => openRow(r, 'pods') });
      if (can('logs', r.namespace)) items.push({ label: 'Logs (all pods)', icon: 'file', action: () => openRow(r, 'logs') });
    }
    const acts = actionsFor(kind, r).filter(a => canAction(a, r));
    if (acts.length) {
      items.push({ separator: true });
      for (const a of acts) items.push({ label: a.label, icon: a.icon, danger: a.danger, action: () => void doAction(a, r) });
    }
    ctxMenu.show(e, items);
  }

  // --- k9s -------------------------------------------------------------------------
  async function openK9s(): Promise<void> {
    if (!canK9s || k9sOpening) return;
    if (!k8s.status?.k9s.installed) {
      if (isAdmin) k9sInstallOpen = true;
      else toasts.warn('k9s is not installed', 'Ask an Otto admin to install it from the Kubernetes overview.');
      return;
    }
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.error(NO_WORKSPACE, 'The k9s session is attached to a workspace.');
      return;
    }
    k9sOpening = true;
    try {
      const s = await k8sApi.k9s(cluster.id, { workspace_id: wsId, ns: k8s.namespace || null });
      k8s.k9sSessionId = s.id;
    } catch (e) {
      toastError('Couldn’t start k9s', e);
    } finally {
      k9sOpening = false;
    }
  }
  async function closeK9s(): Promise<void> {
    const id = k8s.k9sSessionId;
    k8s.k9sSessionId = null;
    if (!id) return;
    // The auto-refresh skipped its ticks while k9s covered the table.
    void k8s.loadResources(true);
    try {
      await api.del(`/sessions/${id}`);
    } catch {
      /* best-effort */
    }
  }

  // --- keyboard (k9s muscle memory) ---------------------------------------------------
  function isTyping(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el) return false;
    const tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
  }
  $effect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (keyContext.terminalFocused || ui.modalCount > 0 || ctxMenu.open || k8s.k9sSessionId) return;
      // Only while focus is in the workspace (or on nothing): a letter typed
      // in the sidebar or another pane is not a workspace command.
      if (!inWorkspace(e.target, wspEl)) return;
      if (!singleKeys && SINGLE_KEYS.has(e.key)) return;
      if (e.key === 'Escape') {
        // The open namespace combobox owns Escape and preserves its focus.
        if ((e.target as HTMLElement | null)?.closest('[role="combobox"][aria-expanded="true"]')) return;
        if (isTyping(e.target)) {
          (e.target as HTMLElement).blur();
          return;
        }
        if (k8s.selected) {
          e.preventDefault();
          closeDrawer();
        }
        return;
      }
      if (isTyping(e.target)) return;
      const row = k8s.selectedRow ?? (k8s.selected ? null : k8s.filteredRows[0] ?? null);
      switch (e.key) {
        case '/':
          e.preventDefault();
          filterEl?.focus();
          filterEl?.select();
          break;
        case '?':
          // The shell binds `?` too (global cheat-sheet). We listen in the
          // capture phase and stop propagation so only the module sheet opens.
          e.preventDefault();
          e.stopPropagation();
          hintsOpen = !hintsOpen;
          break;
        case 'n':
          e.preventDefault();
          nsPicker?.focus();
          break;
        case 'r':
          e.preventDefault();
          void k8s.loadResources();
          break;
        case 'l':
          if (kind === 'pods' && row) {
            e.preventDefault();
            openRow(row, 'logs');
          }
          break;
        case 's':
          if (kind === 'pods' && row && can('exec', row.namespace)) {
            e.preventDefault();
            openRow(row, 'terminal', true);
          }
          break;
        case 'd':
          if (row) {
            e.preventDefault();
            openRow(row, 'describe');
          }
          break;
        case 'y':
          if (row) {
            e.preventDefault();
            openRow(row, 'manifest');
          }
          break;
        case 'j':
        case 'k': {
          const rows = k8s.filteredRows;
          if (!rows.length) break;
          e.preventDefault();
          const i = k8s.selectedRow ? rows.indexOf(k8s.selectedRow) : -1;
          const j = e.key === 'j' ? Math.min(rows.length - 1, i + 1) : Math.max(0, i - 1);
          const next = rows[j];
          if (next) {
            if (k8s.selected) openRow(next);
            else k8s.select({ ns: next.namespace, name: next.name });
          }
          break;
        }
        case 'Enter':
          // Native controls own Enter; the workspace shortcut is for the table.
          if ((e.target as HTMLElement | null)?.closest('button, a[href], summary, [role="button"], [role="tab"]')) return;
          if (row) {
            e.preventDefault();
            openRow(row);
          }
          break;
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  const lastLoaded = $derived(k8s.rowsLoadedAt ? formatAge(Math.floor((Date.now() - k8s.rowsLoadedAt) / 1000)) : '');
  const drawerOpen = $derived(!!sel);
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {     "class":`wsp`,...__sveltets_2_empty({"data-testid":`k8s-workspace`}),});cluster.environment === 'prod';wspEl = $$_div0;
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":cluster.name,"class":`k8s-topbar`,leading:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => router.go('kubernetes'),"title":`All clusters`,"aria-label":`Back to clusters`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});} }
     { svelteHTML.createElement("span", {   "class":`dot`,"style":`background:${cluster.color || 'var(--accent)'}`,}); }
  };return __sveltets_2_any(0)},titleContent:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("select", {         "class":`input cluster-sel`,"aria-label":`Cluster`,"value":cluster.id,"onchange":(e) => switchCluster((e.currentTarget as HTMLSelectElement).value),...__sveltets_2_empty({"data-testid":`k8s-cluster-switcher`}),});
         for(let c of __sveltets_2_ensureArray(k8s.clusters)){c.id; { svelteHTML.createElement("option", { "value":c.id,});c.name; }}
     }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_egdaBvnE0C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE0C({ target: __sveltets_2_any(), props: {  "env":cluster.environment,}});}
    if(k8s.caps?.server_version){ { svelteHTML.createElement("span", {   "class":`ver mono`,"title":`Server version`,});k8s.caps.server_version; }}
  };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_hctiwSweiVs8K0C = __sveltets_2_ensureComponent(K8sViewSwitch); new $$_hctiwSweiVs8K0C({ target: __sveltets_2_any(), props: {    "clusterId":cluster.id,"view":`resources`,}});}
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => void k8s.loadResources(),"title":`Refresh (r)`,"aria-label":`Refresh`,...__sveltets_2_empty({"data-icon":`refresh`}),...__sveltets_2_empty({"data-label":`Refresh`}),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});} }
    if(canK9s){
       { svelteHTML.createElement("button", {             "class":`btn small`,"onclick":() => void openK9s(),"disabled":k9sOpening,"title":`Open k9s in a terminal`,...__sveltets_2_empty({"data-testid":`k8s-k9s-btn`}),...__sveltets_2_empty({"data-icon":`terminal`}),...__sveltets_2_empty({"data-label":`Open k9s`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});} 
       }
    }
     { svelteHTML.createElement("button", {              "class":`pill-toggle`,...__sveltets_2_empty({"data-overflow":`-2`}),...__sveltets_2_empty({"data-icon":`clock`}),...__sveltets_2_empty({"data-label":k8s.autoRefresh ? 'Stop auto-refresh' : 'Auto-refresh every 10 s'}),"onclick":() => k8s.setAutoRefresh(!k8s.autoRefresh),"aria-pressed":k8s.autoRefresh,"title":`Auto-refresh every 10 s`,});k8s.autoRefresh;
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} 
     }
     { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-3`}),...__sveltets_2_empty({"data-icon":`command`}),...__sveltets_2_empty({"data-label":`Keyboard shortcuts`}),"onclick":() => (hintsOpen = true),"title":`Keyboard shortcuts (?)`,"aria-label":`Keyboard shortcuts`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`command`,"size":14,}});} }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, titleContent, badge, tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
  
  
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
  if(k8s.k9sSessionId){
     { svelteHTML.createElement("div", { "class":`k9s`,});
       { svelteHTML.createElement("div", { "class":`k9s-bar`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":13,}});}
         { svelteHTML.createElement("span", {});  cluster.name;k8s.namespace ? ` · ${k8s.namespace}` : ''; }
         { svelteHTML.createElement("span", { "class":`spacer`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void closeK9s(),});  }
       }
       { svelteHTML.createElement("div", { "class":`k9s-term`,});
        k8s.k9sSessionId; {
            { const $$_lanimreTyzaL4C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL4C({ target: __sveltets_2_any(), props: {  "sessionId":k8s.k9sSessionId,"preferDom":true,"autoFocus":true,"forceDark":true,}});}
        }
       }
     }
  }else{
     { const $$_div2 = svelteHTML.createElement("div", {    "class":`body`,});bodyEl = $$_div2;bodyWidth= $$_div2.clientWidth;resizing;
      if(viewport.isPhone){
        
         { svelteHTML.createElement("div", { "class":`kinds-mobile`,});
           { svelteHTML.createElement("select", {         "class":`input`,"aria-label":`Resource kind`,"value":kind,"onchange":(e) => goKind((e.currentTarget as HTMLSelectElement).value as K8sResourceKind),...__sveltets_2_empty({"data-testid":`k8s-kinds`}),});
               for(let k of __sveltets_2_ensureArray(kinds)){k.id; { svelteHTML.createElement("option", { "value":k.id,});k.label; }}
           }
          ;__sveltets_2_ensureSnippet(filterBox());
         }
      }else{
         { svelteHTML.createElement("nav", {     "class":`kinds`,"aria-label":`Resource kinds`,...__sveltets_2_empty({"data-testid":`k8s-kinds`}),});
             for(let k of __sveltets_2_ensureArray(kinds)){k.id;
             { svelteHTML.createElement("button", {       "class":`kind`,"onclick":() => goKind(k.id),"aria-current":k.id === kind ? 'page' : undefined,});k.id === kind;!!k.requires;
              k.label;
              if(k.id === kind && !k8s.rowsLoading && !(k8s.rowsError && !rowsForKey.length)){ { svelteHTML.createElement("span", { "class":`cnt mono`,});rowsForKey.length; }}
             }
          }
         }
      }

       { svelteHTML.createElement("div", { "class":`center`,});
        
         { svelteHTML.createElement("div", {     "class":`tbl-bar`,"role":`toolbar`,"aria-label":`Resource table`,});
           { svelteHTML.createElement("span", { "class":`ns-keep`,}); { const $$_rekciPecapsemaN6C = __sveltets_2_ensureComponent(NamespacePicker); const $$_rekciPecapsemaN6 = new $$_rekciPecapsemaN6C({ target: __sveltets_2_any(), props: {             "allowAll":can(readOperation(kind), ''),"value":k8s.namespace,"namespaces":k8s.namespaces,"error":k8s.namespacesError,"disabled":clusterScoped,"onchange":(ns) => k8s.setNamespace(ns),}});nsPicker = $$_rekciPecapsemaN6;} }
          if(!viewport.isPhone){
            ;__sveltets_2_ensureSnippet(filterBox());
          }
           { svelteHTML.createElement("span", {     "class":`meta dim`,"role":`status`,"title":k8s.rowsLoadedAt ? new Date(k8s.rowsLoadedAt).toLocaleTimeString() : '',});
            if(k8s.rowsLoading){ } else if (lastLoaded){k8s.filteredRows.length;k8s.filter ? `/${rowsForKey.length}` : '';  lastLoaded; }
           }
         }
         { const $$_elbaTecruoseR4C = __sveltets_2_ensureComponent(ResourceTable); new $$_elbaTecruoseR4C({ target: __sveltets_2_any(), props: {                                  kind,"rows":k8s.filteredRows,"total":rowsForKey.length,"hasMetrics":k8s.hasMetrics,"allNamespaces":allNs,"loading":k8s.rowsLoading,"error":k8s.rowsError,"selected":sel,"onselect":(r) => k8s.select({ ns: r.namespace, name: r.name }),"onopen":(r) => openRow(r),"onmenu":rowMenu,"onretry":() => void k8s.loadResources(),"filter":k8s.filter,"onclearfilter":() => { k8s.filter = ''; filterEl?.focus(); },"scrollKey":k8s.currentKey,"initialScroll":(key) => k8s.scrollFor(key),"onscrolled":(key, top) => k8s.saveScroll(key, top),}});}
       }

      if(drawerOpen && sel){
        if(!detailSheet){
          
           {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: visibleDrawerW, min: 320, max: maxDrawerW, invert: true, onChange: (w) => { drawerW = w; saveDrawerWidth(); }, text: pxWide })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {               "class":`splitter`,"role":`separator`,"aria-orientation":`vertical`,"aria-label":`Resize details`,"title":RESIZE_TITLE,"onpointerdown":startResize,}); }}
        }
        `${cluster.id}/${kind}/${sel.ns}/${sel.name}`; {
             { const $$_rewarDecruoseR3C = __sveltets_2_ensureComponent(ResourceDrawer); new $$_rewarDecruoseR3C({ target: __sveltets_2_any(), props: {                                "modal":detailSheet,"width":`${visibleDrawerW}px`,"clusterId":cluster.id,kind,"ns":sel.ns,"name":sel.name,"row":selRow,"tab":k8s.drawerTab,canEdit,autoExec,"onautoexec":() => (autoExec = false),"ontab":(t) => (k8s.drawerTab = t),"onclose":closeDrawer,"onopenpod":openPod,"onaction":(a, r) => void doAction(a, r),"reloadNonce":drawerNonce,"onmonitor":openInMonitor,}});}
        }
      }
     }
  }
   PageBody}
 }

if(scaleFor){
   { const $$_golaiDelacS0C = __sveltets_2_ensureComponent(ScaleDialog); new $$_golaiDelacS0C({ target: __sveltets_2_any(), props: {         "row":scaleFor.row,"kindLabel":def.singular,"onclose":() => (scaleFor = null),"onsubmit":(n) => {
      const f = scaleFor!;
      scaleFor = null;
      void runAction(cluster.id, kind, f.row, f.def, { replicas: n }).then((r) => r && afterAction());
    },}});}
}
if(syncFor){
   { const $$_golaiDcnyS0C = __sveltets_2_ensureComponent(SyncDialog); new $$_golaiDcnyS0C({ target: __sveltets_2_any(), props: {       "row":syncFor.row,"onclose":() => (syncFor = null),"onsubmit":(p) => {
      const f = syncFor!;
      syncFor = null;
      void runAction(cluster.id, kind, f.row, f.def, p).then((r) => r && afterAction());
    },}});}
}
if(k9sInstallOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Install k9s`,"width":520,"onclose":() => (k9sInstallOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (k9sInstallOpen = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!k8s.status?.k9s.installed,"onclick":() => { k9sInstallOpen = false; void openK9s(); },});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
      { const $$_lenaPllatsnI1C = __sveltets_2_ensureComponent(InstallPanel); new $$_lenaPllatsnI1C({ target: __sveltets_2_any(), props: {  "tool":`k9s`,"compact":true,}});}
    
   Modal}
}
if(hintsOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Keyboard shortcuts`,"width":420,"onclose":() => (hintsOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (hintsOpen = false),});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("dl", { "class":`hints`,});
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});   }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});   }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  }   { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});    }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});      }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});  }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});  }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});  }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});     }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});  }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});      }
       { svelteHTML.createElement("dt", {}); { svelteHTML.createElement("kbd", {});  } } { svelteHTML.createElement("dd", {});  }
     }
     { svelteHTML.createElement("p", { "class":`hints-scope`,});               }
     { const $$_elggoTgnitteS1C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS1C({ target: __sveltets_2_any(), props: {           "label":`Single-key shortcuts`,"hint":`Turn off if you use a screen reader or speech input, or type into the page with letter keys. Enter and Esc keep working.`,"checked":singleKeys,"onchange":(v) => {
        singleKeys = v;
        lsSet(SINGLE_KEYS_PREF, v ? '1' : '0');
      },"testid":`k8s-single-keys`,}});}
    
   Modal}
}




};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ClusterWorkspace__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ClusterWorkspace__SvelteComponent_ = ReturnType<typeof ClusterWorkspace__SvelteComponent_>;
/*Ωignore_endΩ*/export default ClusterWorkspace__SvelteComponent_;