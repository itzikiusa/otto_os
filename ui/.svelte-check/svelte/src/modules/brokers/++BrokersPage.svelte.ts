///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import Badge from '../../lib/components/Badge.svelte';
import { toastError } from '../../lib/toastError';
import { CLUSTER_VIEWS, type ClusterView } from './types';
import Icon from '../../lib/components/Icon.svelte';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
import { api } from '../../lib/api/client';
import { brokers } from '../../lib/stores/brokers.svelte';
import { brokersPagePort } from '../../lib/uiCommands/brokers';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import type { BrokerCluster, ConnectionSection, TestClusterResp } from '../../lib/api/types';
import ClusterForm from './ClusterForm.svelte';
import OverviewTab from './OverviewTab.svelte';
import TopicsTab from './TopicsTab.svelte';
import GroupsTab from './GroupsTab.svelte';
import SchemaTab from './SchemaTab.svelte';
import ReplayPanel from './ReplayPanel.svelte';
import LagAlertsPanel from './LagAlertsPanel.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import { onTabKey } from '../../lib/tabKeys';
import { router } from '../../lib/router.svelte';
import { untrack } from 'svelte';
function $$render() {
 const sectionNode/*Ωignore_positionΩ*/ = (node: TreeNode, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const isOpen = !collapsed[node.sec.id];
  
  
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                       "role":`presentation`,"class":`sec-head`,"onclick":(e) => {
      if (!(e.target as Element).closest('button')) collapsed[node.sec.id] = !collapsed[node.sec.id];
    },"style":`padding-inline-start: ${depth * 14 + 6}px`,"draggable":`true`,"ondragstart":(e) => {
      draggedSectionId = node.sec.id;
      e.stopPropagation();
    },"ondragend":() => (draggedSectionId = null),"ondragover":(e) => {
      if (draggedClusterId || (draggedSectionId && draggedSectionId !== node.sec.id))
        e.preventDefault();
    },"ondrop":(e) => {
      e.preventDefault();
      e.stopPropagation();
      onSectionDrop(node.sec.id);
    },"oncontextmenu":(e) => openMenu(e, 'section', node.sec.id),});(draggedSectionId && draggedSectionId !== node.sec.id) || draggedClusterId;
     { svelteHTML.createElement("button", {           "class":`caret`,"onclick":() => (collapsed[node.sec.id] = !collapsed[node.sec.id]),"title":isOpen ? 'Collapse' : 'Expand',"aria-label":isOpen ? `Collapse ${node.sec.name}` : `Expand ${node.sec.name}`,"aria-expanded":isOpen,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":isOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
     }
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}
     { svelteHTML.createElement("span", { "class":`sec-name grow`,});node.sec.name; }
    if(node.items.length > 0){ { svelteHTML.createElement("span", { "class":`count`,});node.items.length; }}
   }}
  if(isOpen){
       for(let child of __sveltets_2_ensureArray(node.children)){child.sec.id;
      ;__sveltets_2_ensureSnippet(sectionNode(child, depth + 1));
    }
       for(let c of __sveltets_2_ensureArray(node.items)){c.id;
      ;__sveltets_2_ensureSnippet(clusterRow(c, depth + 1));
    }
  }
};return __sveltets_2_any(0)}; const clusterRow/*Ωignore_positionΩ*/ = (c: BrokerCluster, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                   "role":`group`,"aria-label":c.name,"class":`cluster`,"style":`padding-inline-start: ${depth * 14 + 6}px`,"draggable":`true`,"ondragstart":(e) => {
      draggedClusterId = c.id;
      e.stopPropagation();
    },"ondragend":() => (draggedClusterId = null),"oncontextmenu":(e) => openMenu(e, 'cluster', c.id),});brokers.selectedId === c.id;
     { svelteHTML.createElement("button", {       "class":`cluster-open`,"onclick":() => brokers.select(c.id),"aria-current":brokers.selectedId === c.id ? 'true' : undefined,"title":c.name,});
       { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${c.color || 'var(--accent)'}`,}); }
       { svelteHTML.createElement("span", { "class":`cn`,});c.name; }
     }
     { const $$_egdaBvnE1C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE1C({ target: __sveltets_2_any(), props: {  "env":c.environment,}});}
     { svelteHTML.createElement("button", {       "class":`icon-btn cluster-more`,"aria-label":`Actions for ${c.name}`,"title":`Actions`,"onclick":(e) => openMenu(e, 'cluster', c.id),});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
     }
   }}
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  type Tab = ClusterView;
  let tab = $state<Tab>('overview');
  let formOpen = $state(false);
  let editTarget = $state<BrokerCluster | null>(null);
  let testing = $state(false);

  // Mobile (≤640px): the cluster list and the cluster content stack vertically
  // and each is a collapsible, independently-scrollable section. These toggles
  // only affect the phone layout (the headers/carets are hidden on desktop).
  let clustersOpen = $state(true);
  let contentOpen = $state(true);
  // Auto-collapse the cluster list after picking a cluster on a phone so the
  // content gets the screen; expand it again when nothing is selected.
  $effect(() => {
    if (brokers.selectedId) clustersOpen = false;
    else clustersOpen = true;
  });

  // ---- resizable cluster sidebar --------------------------------------------
  // Drag the divider between the cluster list and the content to resize it; the
  // chosen width survives reloads. Mirrors the Database page's sidebar resizer.
  // (On phones the list is a full-width stacked band — the width var is ignored
  // by the media query there and the divider is hidden.)
  let sideW = $state(loadPaneWidth('brokers.sideW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  // Tablet/desktop: the header `sidebar` icon hides the list pane (the one
  // collapse affordance, as on Database / Canvas / Skills Lab). Phone keeps its
  // stacked accordions.
  let listHidden = $state(false);

  $effect(() => {
    const id = ws.currentId;
    if (id) void brokers.load(id);
  });

  $effect(() => {
    // reset to overview when the selected cluster changes
    void brokers.selectedId;
    tab = 'overview';
  });
  // Agent UI control (lib/uiCommands/brokers.ts) switches the sub-tab here.
  $effect(() => brokersPagePort.bind({ setTab: (v) => (tab = v) }));

  const selected = $derived(brokers.selected);

  // The URL carries the open cluster (`#/brokers/<id>`) so a reload / share /
  // link lands on it. Route → page: a link (or Back/Forward) opens that cluster
  // once the list knows it; page → route: picking a cluster rewrites the URL in
  // place.
  $effect(() => {
    const [mod, id] = router.parts;
    if (mod !== 'brokers' || !id || !brokers.clusters.some((c) => c.id === id)) return;
    untrack(() => {
      if (id !== brokers.selectedId) brokers.select(id);
    });
  });
  let routedId: string | null = null;
  $effect(() => {
    const id = brokers.selectedId;
    const was = routedId;
    routedId = id;
    if (!id && !was) return;
    untrack(() => {
      if (router.module === 'brokers' && (router.parts[1] ?? null) !== id) router.replace(id ? `brokers/${id}` : 'brokers');
    });
  });

  // Never show an empty "pick a cluster" pane when clusters exist: open the
  // last-selected cluster (or the first) on load, and again whenever the last
  // open tab is closed. Not on a phone — selecting collapses the cluster list
  // there, which is the first screen.
  let autoPickedFor = $state<string | null>(null);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || viewport.isPhone || brokers.selectedId) return;
    if (brokers.loading || brokers.clusters.length === 0) return;
    const first = autoPickedFor !== wsId;
    autoPickedFor = wsId;
    const id = first
      ? initialSelection('brokers', brokers.clusters, (c) => c.id)
      : [...brokers.clusters].sort(byName)[0]?.id;
    if (id) untrack(() => brokers.select(id));
  });
  $effect(() => {
    if (brokers.selectedId) rememberSelection('brokers', brokers.selectedId);
  });
  // Nothing to list (no clusters, no sections): hide the list pane — the one
  // page-level empty state owns the page and its "Add a cluster" CTA.
  const isEmpty = $derived(!brokers.loading && !brokers.loadError && brokers.clusters.length === 0 && brokers.sections.length === 0);


  async function testConn(c: BrokerCluster) {
    testing = true;
    try {
      const r = await api.post<TestClusterResp>(`/brokers/clusters/${c.id}/test`, {});
      if (r.ok) toasts.success('Connected', `${r.message} · ${r.latency_ms}ms`);
      else toasts.error('Couldn’t connect', r.message);
    } catch (e) {
      toastError('Couldn’t run the test', e);
    } finally {
      testing = false;
    }
  }

  async function removeCluster(c: BrokerCluster) {
    const ok = await confirmer.ask(
      `Remove cluster "${c.name}"? Topics on the broker are not touched.`,
      { title: 'Remove cluster', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      await brokers.remove(c.id);
      toasts.success('Cluster removed');
    } catch (e) {
      toastError('Couldn’t remove', e);
    }
  }

  // ---- warm tunnel on cluster select ----------------------------------------
  // When a cluster with an SSH tunnel is opened, fire the /test endpoint in the
  // background so the SOCKS proxy warms up and the tunnel pill shows quickly.
  let warmingId = $state<string | null>(null);
  let tunnelReady = $state(false);
  $effect(() => {
    const c = brokers.selected;
    tunnelReady = false;
    if (!c?.ssh) return;
    warmingId = c.id;
    const id = c.id;
    void api.post<TestClusterResp>(`/brokers/clusters/${id}/test`, {})
      .then((r) => {
        if (warmingId === id) tunnelReady = r.ok;
      })
      .catch(() => {
        // silent — tunnel pill stays grey until a real op succeeds
      });
  });

  function openEdit(c: BrokerCluster) {
    editTarget = c;
    formOpen = true;
  }
  function openAdd() {
    editTarget = null;
    formOpen = true;
  }

  // ---- sidebar sections (grouping tree) ------------------------------------
  interface TreeNode {
    sec: ConnectionSection;
    items: BrokerCluster[];
    children: TreeNode[];
  }
  const byName = (a: BrokerCluster, b: BrokerCluster) => a.name.localeCompare(b.name);

  function buildTree(parentId: string | null): TreeNode[] {
    return brokers.sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((sec) => ({
        sec,
        items: brokers.clusters.filter((c) => c.section_id === sec.id).sort(byName),
        children: buildTree(sec.id),
      }));
  }
  const tree = $derived(buildTree(null));
  const knownSectionIds = $derived(new Set(brokers.sections.map((s) => s.id)));
  const ungrouped = $derived(
    brokers.clusters.filter((c) => !c.section_id || !knownSectionIds.has(c.section_id)).sort(byName),
  );

  let collapsed = $state<Record<string, boolean>>({});
  let draggedClusterId = $state<string | null>(null);
  let draggedSectionId = $state<string | null>(null);
  // Right-click context menu (cluster row or section header) — routed through
  // the global ctxMenu overlay, which clamps into the viewport.
  function openMenu(e: MouseEvent, kind: 'cluster' | 'section', id: string): void {
    if (kind === 'cluster') {
      const c = brokers.clusters.find((x) => x.id === id);
      if (!c) return;
      ctxMenu.show(e, [
        { label: 'Open in tab', action: () => brokers.select(c.id) },
        { label: 'Test', action: () => void testConn(c) },
        { label: 'Edit…', action: () => openEdit(c) },
        { label: 'Remove…', icon: 'trash', danger: true, action: () => void removeCluster(c) },
      ]);
    } else {
      const s = brokers.sections.find((x) => x.id === id);
      if (!s) return;
      ctxMenu.show(e, [
        { label: 'New sub-section', action: () => void newSection(s.id) },
        { label: 'Rename…', action: () => void renameSec(s) },
        { label: 'Delete…', danger: true, action: () => void delSec(s) },
      ]);
    }
  }

  async function newSection(parentId: string | null): Promise<void> {
    const name = await confirmer.promptText(parentId ? 'Sub-section name' : 'Section name', {
      title: parentId ? 'New sub-section' : 'New section',
      confirmLabel: 'Create',
      placeholder: 'e.g. Production',
    });
    if (!name) return;
    try {
      await brokers.createSection(parentId, name);
    } catch (e) {
      toastError('Couldn’t create the section', e);
    }
  }

  async function renameSec(sec: ConnectionSection): Promise<void> {
    const name = await confirmer.promptText('Rename section', {
      title: 'Rename section',
      confirmLabel: 'Rename',
      initial: sec.name,
    });
    if (!name || name === sec.name) return;
    try {
      await brokers.renameSection(sec.id, name);
    } catch (e) {
      toastError('Couldn’t rename', e);
    }
  }

  async function delSec(sec: ConnectionSection): Promise<void> {
    if (
      !(await confirmer.ask(
        `Delete section “${sec.name}”? Sub-sections are removed too and their clusters become ungrouped.`,
        { title: 'Delete section', confirmLabel: 'Delete', danger: true },
      ))
    )
      return;
    try {
      await brokers.deleteSection(sec.id);
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  function isDescendantOf(nodeId: string, ancestorId: string): boolean {
    let cur = brokers.sections.find((s) => s.id === nodeId);
    while (cur?.parent_id) {
      if (cur.parent_id === ancestorId) return true;
      cur = brokers.sections.find((s) => s.id === cur!.parent_id);
    }
    return false;
  }

  async function moveCluster(id: string, sectionId: string | null): Promise<void> {
    try {
      await brokers.moveCluster(id, sectionId);
    } catch (e) {
      toastError('Couldn’t move', e);
    }
  }

  async function reparent(id: string, parentId: string | null): Promise<void> {
    const sec = brokers.sections.find((s) => s.id === id);
    if (!sec || (sec.parent_id ?? null) === parentId) return;
    if (parentId && (parentId === id || isDescendantOf(parentId, id))) {
      toasts.error('Invalid move', 'Cannot nest a section inside itself');
      return;
    }
    try {
      await brokers.reparentSection(id, parentId);
    } catch (e) {
      toastError('Couldn’t move', e);
    }
  }

  function onSectionDrop(sectionId: string): void {
    if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void moveCluster(id, sectionId);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparent(src, sectionId);
    }
  }
  function onRootDrop(): void {
    if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void moveCluster(id, null);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparent(src, null);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`brokers-root`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {      "class":`cluster-head`,"title":selected?.name ?? 'Message Brokers',"subtitle":selected?.bootstrap_servers,leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(!viewport.isPhone && !isEmpty){
       { svelteHTML.createElement("button", {             "class":`icon-btn`,"onclick":() => (listHidden = !listHidden),"aria-label":listHidden ? 'Show cluster list' : 'Hide cluster list',"title":listHidden ? 'Show cluster list' : 'Hide cluster list',"aria-expanded":!listHidden,"aria-controls":`brokers-cluster-list`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`sidebar`,"size":16,}});}
       }
    }
    if(selected){
       { svelteHTML.createElement("button", {           "class":`content-toggle`,"onclick":() => (contentOpen = !contentOpen),"aria-expanded":contentOpen,"aria-label":contentOpen ? 'Collapse details' : 'Expand details',"title":contentOpen ? 'Collapse details' : 'Expand details',});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":contentOpen ? 'chevronDown' : 'chevronRight',"size":14,}});}
       }
       { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${selected.color || 'var(--accent)'}`,}); }
    }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(selected){
       { const $$_egdaBvnE0C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE0C({ target: __sveltets_2_any(), props: {  "env":selected.environment,}});}
      if(selected.read_only){ { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":`Read-only`,}});}}
      if(selected.ssh){
         { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {        "tone":tunnelReady ? 'ok' : 'neutral',"dot":true,"live":!tunnelReady,"title":tunnelReady ? 'SSH tunnel connected' : 'SSH tunnel warming…',"label":tunnelReady ? 'Tunnel' : 'Connecting…',}});}
      }
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(selected){
       { svelteHTML.createElement("button", {       "class":`btn small danger`,...__sveltets_2_empty({"data-overflow":`-1`}),"onclick":() => removeCluster(selected),"title":`Remove this cluster profile from Otto (topics on the broker are untouched)`,});  }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openEdit(selected),});  }
       { svelteHTML.createElement("button", {     "class":`btn small`,...__sveltets_2_empty({"data-keep":true}),"onclick":() => testConn(selected),"disabled":testing,});
        testing ? 'Testing…' : 'Test';
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
 { svelteHTML.createElement("div", { "class":`brokers-page`,});
  if(!isEmpty && (viewport.isPhone || !listHidden)){
   { svelteHTML.createElement("aside", {      "id":`brokers-cluster-list`,"class":`clusters`,"style":`--clusters-w:${sideW}px`,});!clustersOpen;
     { svelteHTML.createElement("div", { "class":`aside-head`,});
       { svelteHTML.createElement("button", {         "class":`sec-toggle`,"onclick":() => (clustersOpen = !clustersOpen),"aria-expanded":clustersOpen,"title":clustersOpen ? 'Collapse clusters' : 'Expand clusters',});
         { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":clustersOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}
         { svelteHTML.createElement("span", { "class":`title`,});  }
        if(brokers.clusters.length > 0){ { svelteHTML.createElement("span", { "class":`hcount`,});brokers.clusters.length; }}
       }
       { svelteHTML.createElement("div", { "class":`head-btns`,});
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => newSection(null),"aria-label":`New section`,"title":`New section`,});
           { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}
         }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":openAdd,"aria-label":`Add cluster`,"title":`Add cluster`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});} }
       }
     }
     { svelteHTML.createElement("div", { "class":`cluster-list`,});
      
       { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"what":`clusters`,"variant":`compact`,"loading":brokers.loading,"error":brokers.loadError,"empty":brokers.clusters.length === 0 && brokers.sections.length === 0,"rows":3,"onretry":() => ws.currentId && void brokers.load(ws.currentId),}});
           for(let node of __sveltets_2_ensureArray(tree)){node.sec.id;
          ;__sveltets_2_ensureSnippet(sectionNode(node, 0));
        }

        
        if(brokers.sections.length > 0 || draggedClusterId || draggedSectionId){
         { svelteHTML.createElement("div", {              "role":`group`,"aria-label":`Ungrouped`,"class":`sec-head plain`,"ondragover":(e) => {
            if (draggedClusterId || draggedSectionId) e.preventDefault();
          },"ondrop":(e) => {
            e.preventDefault();
            onRootDrop();
          },"title":`Clusters with no section (drop here to remove from a section / make a section top-level)`,});draggedClusterId || draggedSectionId;
           { svelteHTML.createElement("span", { "class":`caret-spacer`,}); }
           { svelteHTML.createElement("span", { "class":`sec-name grow`,});  }
          if(ungrouped.length > 0){ { svelteHTML.createElement("span", { "class":`count`,});ungrouped.length; }}
         }
        }
           for(let c of __sveltets_2_ensureArray(ungrouped)){c.id;
          ;__sveltets_2_ensureSnippet(clusterRow(c, brokers.sections.length > 0 ? 1 : 0));
        }

       LoadState}
     }
   }

   { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:sideW,"storageKey":`brokers.sideW`,"label":`Resize cluster list`,}});/*Ωignore_startΩ*/() => sideW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
  }

   { svelteHTML.createElement("main", {  "class":`cluster-main`,});!contentOpen;
    if(brokers.openClusters.length > 0){
       { svelteHTML.createElement("div", {     "class":`tabstrip`,"role":`tablist`,"aria-label":`Open clusters`,});
           for(let c of __sveltets_2_ensureArray(brokers.openClusters)){c.id;
          
           { svelteHTML.createElement("div", {    "class":`ctab`,"role":`presentation`,});brokers.selectedId === c.id;
             { svelteHTML.createElement("button", {               "class":`ctab-main`,"role":`tab`,"aria-selected":brokers.selectedId === c.id,"tabindex":brokers.selectedId === c.id ? 0 : -1,"title":c.name,"onclick":() => brokers.select(c.id),"onkeydown":onTabKey,});
               { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${c.color || 'var(--accent)'}`,}); }
               { svelteHTML.createElement("span", { "class":`ctab-name`,});c.name; }
             }
             { svelteHTML.createElement("button", {         "class":`ctab-x`,"aria-label":`Close ${c.name}`,"title":`Close tab`,"onclick":(e) => {
                e.stopPropagation();
                brokers.close(c.id);
              },});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
        }
       }
    }
    if(selected){
       { svelteHTML.createElement("div", {         "class":`tabs`,"role":`tablist`,"aria-label":`Kafka cluster views`,"tabindex":-1,"onkeydown":onTabKey,});
           for(let v of __sveltets_2_ensureArray(CLUSTER_VIEWS)){v.id;
           { svelteHTML.createElement("button", {         "role":`tab`,"aria-selected":tab === v.id,"tabindex":tab === v.id ? 0 : -1,"onclick":() => (tab = v.id),});tab === v.id;v.label; }
        }
       }

       { svelteHTML.createElement("div", { "class":`tab-body`,});
        selected.id; {
          if(tab === 'overview'){
             { const $$_baTweivrevO5C = __sveltets_2_ensureComponent(OverviewTab); new $$_baTweivrevO5C({ target: __sveltets_2_any(), props: {  "clusterId":selected.id,}});}
          } else if (tab === 'topics'){
             { const $$_baTscipoT5C = __sveltets_2_ensureComponent(TopicsTab); new $$_baTscipoT5C({ target: __sveltets_2_any(), props: {  "cluster":selected,}});}
          } else if (tab === 'groups'){
             { const $$_baTspuorG5C = __sveltets_2_ensureComponent(GroupsTab); new $$_baTspuorG5C({ target: __sveltets_2_any(), props: {  "cluster":selected,}});}
          } else if (tab === 'schema'){
             { const $$_baTamehcS5C = __sveltets_2_ensureComponent(SchemaTab); new $$_baTamehcS5C({ target: __sveltets_2_any(), props: {  "cluster":selected,}});}
          } else if (tab === 'replay'){
             { const $$_lenaPyalpeR5C = __sveltets_2_ensureComponent(ReplayPanel); new $$_lenaPyalpeR5C({ target: __sveltets_2_any(), props: {  "cluster":selected,}});}
          } else if (tab === 'alerts'){
             { const $$_lenaPstrelAgaL5C = __sveltets_2_ensureComponent(LagAlertsPanel); new $$_lenaPstrelAgaL5C({ target: __sveltets_2_any(), props: {  "cluster":selected,}});}
          }
        }
       }
    }else{
      if(isEmpty){
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`box`,"title":`Connect a Kafka cluster`,"body":`Browse topics, peek messages, inspect consumer-group lag, and watch broker CPU / RAM.`,"actionLabel":`Add a cluster`,"actionIcon":`plus`,"onaction":openAdd,}});}
      }
      
    }
   }
 }
 PageBody}
 }






if(formOpen){
   { const $$_mroFretsulC0C = __sveltets_2_ensureComponent(ClusterForm); new $$_mroFretsulC0C({ target: __sveltets_2_any(), props: {    "cluster":editTarget,"onclose":() => (formOpen = false),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrokersPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrokersPage__SvelteComponent_ = ReturnType<typeof BrokersPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrokersPage__SvelteComponent_;