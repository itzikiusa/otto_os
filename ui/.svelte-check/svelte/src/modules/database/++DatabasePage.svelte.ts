///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import Badge from '../../lib/components/Badge.svelte';
import { focusOnMount } from '../../lib/focusOnMount';
import { NO_WORKSPACE } from '../../lib/labels';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// DB Explorer page (mirrors ApiPage): left sidebar = connection picker +
// SchemaTree + a Saved/History switch; main = a tab strip (Query / Builder /
// Structure / Dashboards) over the active view.
import { tick } from 'svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { paneResizer, loadPaneWidth, LIST_PANE } from '../../lib/paneResizer';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import { envTone } from '../../lib/status';
import LoadState from '../../lib/components/LoadState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import SchemaTree from './SchemaTree.svelte';
import QueryEditor from './QueryEditor.svelte';
import ConnectionComparison from './ConnectionComparison.svelte';
import Modal from '../../lib/components/Modal.svelte';
import DatabaseChanges from './DatabaseChanges.svelte';
import ResourceAccess from '../../lib/components/ResourceAccess.svelte';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import ConnectionForm from '../connections/ConnectionForm.svelte';
import ConnectionImportDialog from '../connections/ConnectionImportDialog.svelte';
import LazyMount from '../../lib/components/LazyMount.svelte';
import { lazyComponent, whenIdle } from '../../lib/lazy-component.svelte';
import { PRIMARY_SCROLLBACK } from '../../lib/components/termFlow';
import ImportDialog from './ImportDialog.svelte';
import ExportDialog from './ExportDialog.svelte';
import { stmtPreview } from '../../lib/database/sql-util';
import { databaseAccessChild } from '../../lib/access-options';
import { database, engineGlyph, type DbMainTab, type DbSideTab } from '../../lib/stores/database.svelte';
import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
import { brokers } from '../../lib/stores/brokers.svelte';
import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { popoutItems } from '../../lib/popoutMenu';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { lsGet, lsSet } from '../../lib/storage';
import { rel } from '../../lib/stores/now.svelte';
import type {
    BrokerCluster,
    Connection,
    ConnectionKind,
    ConnectionSection,
    DbSavedQuery,
    Session,
  } from '../../lib/api/types';
function $$render() {
 const sectionNode/*Ωignore_positionΩ*/ = (node: TreeNode, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const isOpen = !collapsed[node.sec.id];
  if(nodeVisible(node)){
    
    
     { svelteHTML.createElement("div", {                    "role":`presentation`,"class":`sec-head`,"onclick":(e) => {
        if (!(e.target as Element).closest('button')) toggleCollapse(node.sec.id);
      },"style":`padding-inline-start: ${depth * 14 + 2}px`,"draggable":`true`,"ondragstart":(e) => {
        draggedSectionId = node.sec.id;
        e.stopPropagation();
      },"ondragend":() => (draggedSectionId = null),"ondragover":(e) => {
        if (draggedConnId || draggedClusterId || (draggedSectionId && draggedSectionId !== node.sec.id)) e.preventDefault();
      },"ondrop":(e) => {
        e.preventDefault();
        e.stopPropagation();
        onSectionDrop(node.sec.id);
      },});(draggedSectionId && draggedSectionId !== node.sec.id) ||
        draggedConnId ||
        draggedClusterId;
       { svelteHTML.createElement("button", {           "class":`caret`,"onclick":() => toggleCollapse(node.sec.id),"aria-label":isOpen ? `Collapse ${node.sec.name}` : `Expand ${node.sec.name}`,"title":isOpen ? `Collapse ${node.sec.name}` : `Expand ${node.sec.name}`,"aria-expanded":isOpen,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":isOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
       }
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
       { svelteHTML.createElement("span", { "class":`sec-name grow ellipsis`,});node.sec.name; }
      if(nodeCount(node) > 0){ { svelteHTML.createElement("span", { "class":`count`,});nodeCount(node); }}
       { svelteHTML.createElement("div", { "class":`sec-actions`,});
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Add sub-section`,"aria-label":`Add sub-section`,"onclick":() => createSection(node.sec.id),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Rename section`,"aria-label":`Rename section`,"onclick":() => renameSection(node.sec),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Delete section`,"aria-label":`Delete section`,"onclick":() => deleteSection(node.sec),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
         }
       }
     }
    if(isOpen){
         for(let c of __sveltets_2_ensureArray(node.items)){c.id;
        ;__sveltets_2_ensureSnippet(connRow(c, depth + 1));
      }
         for(let cl of __sveltets_2_ensureArray(node.clusters)){cl.id;
        ;__sveltets_2_ensureSnippet(clusterRow(cl, depth + 1));
      }
         for(let child of __sveltets_2_ensureArray(node.children)){child.sec.id;
        ;__sveltets_2_ensureSnippet(sectionNode(child, depth + 1));
      }
    }
  }
};return __sveltets_2_any(0)}; const connRow/*Ωignore_positionΩ*/ = (c: Connection, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const isDb = database.connections.some((x) => x.id === c.id);
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                       "role":`group`,"aria-label":c.name,"class":`conn-row`,...__sveltets_2_empty({"data-connection-id":c.id}),"style":`padding-inline-start: ${depth * 14}px`,"draggable":`true`,"ondragstart":(e) => {
      draggedConnId = c.id;
      e.stopPropagation();
    },"ondragend":() => (draggedConnId = null),"oncontextmenu":(e) => { e.preventDefault(); connMenu(e, c); },});database.selectedConnId === c.id;database.openConnIds.includes(c.id);draggedConnId === c.id;
     { svelteHTML.createElement("button", {       "class":`conn-item`,"onclick":() => (isDb ? database.openConnection(c.id) : void openTerminal(c)),"title":`${c.name} · ${c.kind}${isProdConn(c) ? ' · PRODUCTION' : c.read_only ? ' · read-only' : ''} — ${isDb ? 'opens the workbench; right-click for more' : 'opens a terminal session; right-click for more'}`,});
       { svelteHTML.createElement("span", { "class":`conn-glyph ${c.kind}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":engineGlyph(c.kind),"size":12,}});} }
       { svelteHTML.createElement("span", { "class":`conn-name`,});c.name; }
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":c.kind,}});}
      if(opening[c.id]){ { svelteHTML.createElement("span", {       "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,"title":`Opening…`,}); }}
      if(envBadge(c)){ { const $$_egdaBvnE2C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE2C({ target: __sveltets_2_any(), props: {    "env":c.environment,"readOnly":c.read_only,}});}}
     }
     { svelteHTML.createElement("div", { "class":`conn-actions`,});
      
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Actions for ${c.name}`,"title":`Actions`,"onclick":(e) => connMenu(e, c),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
       }
     }
   }}
};return __sveltets_2_any(0)}; const clusterRow/*Ωignore_positionΩ*/ = (cl: BrokerCluster, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                   "role":`group`,"aria-label":cl.name,"class":`conn-row`,"style":`padding-inline-start: ${depth * 14}px`,"draggable":`true`,"ondragstart":(e) => {
      draggedClusterId = cl.id;
      e.stopPropagation();
    },"ondragend":() => (draggedClusterId = null),"oncontextmenu":(e) => { e.preventDefault(); clusterMenu(e, cl); },});draggedClusterId === cl.id;
     { svelteHTML.createElement("button", {       "class":`conn-item`,"onclick":() => openCluster(cl),"title":`${cl.name} · kafka${isProdConn(cl) ? ' · PRODUCTION' : cl.read_only ? ' · read-only' : ''} — opens here`,});
       { svelteHTML.createElement("span", { "class":`conn-glyph kafka`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":engineGlyph('kafka'),"size":12,}});} }
       { svelteHTML.createElement("span", { "class":`conn-name`,});cl.name; }
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":`kafka`,}});}
      if(envBadge(cl)){ { const $$_egdaBvnE2C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE2C({ target: __sveltets_2_any(), props: {    "env":cl.environment,"readOnly":cl.read_only,}});}}
     }
     { svelteHTML.createElement("div", { "class":`conn-actions`,});
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Actions for ${cl.name}`,"title":`Actions`,"onclick":(e) => clusterMenu(e, cl),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
       }
     }
   }}
};return __sveltets_2_any(0)};  const connSearchBox/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", { "class":`tree-search`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
     { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`tree-search-input`,"type":`text`,"bind:value":connFilter,"placeholder":`Filter connections…`,"spellcheck":`false`,"aria-label":`Filter connections`,});/*Ωignore_startΩ*/() => connFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
    if(connFilter){
       { svelteHTML.createElement("button", {       "class":`tree-search-clear`,"onclick":() => (connFilter = ''),"aria-label":`Clear filter`,"title":`Clear filter`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
    }
    if(!viewport.isPhone){
      
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => createSection(null),"aria-label":`New section`,"title":`New section`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});} }
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":!auth.isRoot,"onclick":newConnection,"aria-label":`New connection`,"title":auth.isRoot ? 'New connection (SSH, database or custom CLI)' : 'Only the owner can create connections',}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":newCluster,"aria-label":`New Kafka cluster`,"title":`New Kafka cluster`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":12,}});} }
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":!auth.isRoot,"onclick":() => (connImportOpen = true),"aria-label":`Import connections`,"title":auth.isRoot ? 'Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster' : 'Only the owner can import connections',}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});} }
    }
   }
  
  if(visibleChips.length > 2 || filtering){
   { svelteHTML.createElement("div", {     "class":`type-chips`,"role":`group`,"aria-label":`Filter by connection type`,});
       for(let chip of __sveltets_2_ensureArray(visibleChips)){chip.id;
       { svelteHTML.createElement("button", {          "class":`type-chip`,"aria-pressed":filterKind === chip.id,...__sveltets_2_empty({"data-testid":`connhub-filter-${chip.id}`}),"onclick":() => setFilter(chip.id),});filterKind === chip.id;chip.label; }
    }
   }
  }
};return __sveltets_2_any(0)};  const connListBody/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  ;__sveltets_2_ensureSnippet(connSearchBox());
  if(database.connectionsError){
    
     { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {            "what":`connections`,"variant":`compact`,"loading":database.connectionsLoading,"error":database.connectionsError,"empty":true,"onretry":() => void database.loadConnections(),}});}
  }
  if(database.connections.length === 0 && database.otherConnections.length === 0 && brokers.clusters.length === 0 && sections.length === 0 && (database.connectionsError || database.connectionsLoading)){
    if(!database.connectionsError){  { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {    "what":`connections`,"variant":`compact`,"loading":true,"empty":true,}});}}
  } else if (database.connections.length === 0 && database.otherConnections.length === 0 && brokers.clusters.length === 0 && sections.length === 0){
     { svelteHTML.createElement("div", { "class":`conn-empty`,});
        
       { svelteHTML.createElement("button", {     "class":`link`,"disabled":!auth.isRoot,"onclick":newConnection,});   }
     }
  } else if (connFilter.trim()){
    if(connMatches.length === 0 && clusterMatches.length === 0){
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {            "icon":`search`,"title":`No matches`,"body":`No connections match “${connFilter.trim()}”.`,"actionLabel":`Clear filter`,"actionKind":`secondary`,"onaction":() => (connFilter = ''),}});}
    }else{
         for(let c of __sveltets_2_ensureArray(connMatches)){c.id;
        ;__sveltets_2_ensureSnippet(connRow(c, 0));
      }
         for(let cl of __sveltets_2_ensureArray(clusterMatches)){cl.id;
        ;__sveltets_2_ensureSnippet(clusterRow(cl, 0));
      }
    }
  }else{
       for(let node of __sveltets_2_ensureArray(tree)){node.sec.id;
      ;__sveltets_2_ensureSnippet(sectionNode(node, 0));
    }

    if(sections.length > 0){
      if(!(filtering && !dragReveal && ungrouped.length + ungroupedClusters.length === 0)){
        
         { svelteHTML.createElement("div", {              "role":`group`,"aria-label":`Ungrouped`,"class":`sec-head plain`,"ondragover":(e) => {
            if (draggedConnId || draggedClusterId || draggedSectionId) e.preventDefault();
          },"ondrop":(e) => {
            e.preventDefault();
            onRootDrop();
          },"title":`Drop here to remove from a section / make a section top-level`,});draggedConnId || draggedClusterId || draggedSectionId;
           { svelteHTML.createElement("span", { "class":`caret-spacer`,}); }
           { svelteHTML.createElement("span", { "class":`sec-name grow`,});  }
          if(ungrouped.length + ungroupedClusters.length > 0){ { svelteHTML.createElement("span", { "class":`count`,});ungrouped.length + ungroupedClusters.length; }}
         }
           for(let c of __sveltets_2_ensureArray(ungrouped)){c.id;
          ;__sveltets_2_ensureSnippet(connRow(c, 1));
        }
           for(let cl of __sveltets_2_ensureArray(ungroupedClusters)){cl.id;
          ;__sveltets_2_ensureSnippet(clusterRow(cl, 1));
        }
      }
    }else{
         for(let c of __sveltets_2_ensureArray(ungrouped)){c.id;
        ;__sveltets_2_ensureSnippet(connRow(c, 0));
      }
         for(let cl of __sveltets_2_ensureArray(ungroupedClusters)){cl.id;
        ;__sveltets_2_ensureSnippet(clusterRow(cl, 0));
      }
    }
  }
};return __sveltets_2_any(0)};  const schemaSideBody/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(database.sideTab === 'saved'){
     { svelteHTML.createElement("div", { "class":`list-search`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`list-search-input`,"placeholder":`Filter saved queries…`,"bind:value":savedSearch,"aria-label":`Filter saved queries`,});/*Ωignore_startΩ*/() => savedSearch = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(savedSearch){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (savedSearch = ''),"aria-label":`Clear filter`,"title":`Clear filter`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
      }
     }
     { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {            "what":`saved queries`,"variant":`compact`,"loading":database.savedQueriesLoading,"error":database.savedQueriesError,"empty":database.savedQueries.length === 0,"onretry":() => database.loadSavedQueries(),}});}
    if(database.savedQueries.length === 0 && (database.savedQueriesLoading || database.savedQueriesError)){
      
    } else if (database.savedQueries.length === 0){
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`file`,"title":`No saved queries`,"body":`Save one from the Query tab.`,}});}
    } else if (filteredSaved.length === 0){
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {            "icon":`search`,"title":`No matches`,"body":`No saved queries match “${savedSearch}”.`,"actionLabel":`Clear filter`,"actionKind":`secondary`,"onaction":() => (savedSearch = ''),}});}
    }else{
         for(let q of __sveltets_2_ensureArray(filteredSaved)){q.id;
         { svelteHTML.createElement("div", { "class":`saved-row`,});
          if(renamingId === q.id){
             {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {             "dir":`auto`,"class":`rename-input`,"bind:value":renameDraft,"onkeydown":(e) => {
                if (e.key === 'Enter') void commitRename();
                else if (e.key === 'Escape') cancelRename();
              },"onblur":() => void commitRename(),"aria-label":`Rename saved query`,});/*Ωignore_startΩ*/() => renameDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
          }else{
             { svelteHTML.createElement("button", {     "class":`saved-open`,"onclick":() => database.openSavedQuery(q),"title":stmtPreview(q.statement, 1000),});
               { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
               { svelteHTML.createElement("span", { "class":`ellipsis`,});q.name; }
             }
             { svelteHTML.createElement("button", {       "class":`icon-btn row-del`,"onclick":() => startRename(q),"aria-label":`Rename saved query`,"title":`Rename`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} }
             { svelteHTML.createElement("button", {        "class":`icon-btn row-del`,"onclick":async () => {
                const ok = await confirmer.ask(`Delete saved query “${q.name}”? Open tabs keep their text but are no longer linked to it.`, {
                  title: 'Delete saved query',
                  confirmLabel: 'Delete query',
                  danger: true,
                });
                if (ok) void database.deleteSavedQuery(q.id);
              },"aria-label":`Delete saved query “${q.name}”…`,"title":`Delete…`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
          }
         }
      }
    }
  } else if (database.sideTab === 'history'){
     { svelteHTML.createElement("div", { "class":`list-search`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`list-search-input`,"placeholder":`Filter history…`,"bind:value":historySearch,"aria-label":`Filter query history`,});/*Ωignore_startΩ*/() => historySearch = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(historySearch){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (historySearch = ''),"aria-label":`Clear filter`,"title":`Clear filter`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
      }
     }
     { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {            "what":`query history`,"variant":`compact`,"loading":database.historyLoading,"error":database.historyError,"empty":database.history.length === 0,"onretry":() => database.loadHistory(),}});}
    if(database.history.length === 0 && (database.historyLoading || database.historyError)){
      
    } else if (database.history.length === 0){
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":`No query history yet`,"body":`Queries you run show up here.`,}});}
    } else if (filteredHistory.length === 0){
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {            "icon":`search`,"title":`No matches`,"body":`No history matches “${historySearch}”.`,"actionLabel":`Clear filter`,"actionKind":`secondary`,"onaction":() => (historySearch = ''),}});}
    }else{
         for(let h of __sveltets_2_ensureArray(filteredHistory)){h.id;
        
         { svelteHTML.createElement("button", {      "class":`hist-row`,"onclick":() => void database.openHistory(h),"title":h.error ? stmtPreview(h.error, 1000) : stmtPreview(h.statement, 1000),});!h.ok;
           { svelteHTML.createElement("span", {  "class":`hist-dot`,});h.ok; }
           { svelteHTML.createElement("span", { "class":`hist-stmt ellipsis mono`,});stmtPreview(h.statement); }
           { svelteHTML.createElement("span", { "class":`hist-meta`,});h.ok ? `${h.row_count}r` : 'err';  rel(h.created_at, { compact: true }); }
         }
      }
      if(database.canLoadMoreHistory){
         { svelteHTML.createElement("button", {     "class":`load-more`,"onclick":() => database.loadMoreHistory(),"disabled":database.historyLoadingMore,});
          database.historyLoadingMore ? 'Loading more history…' : 'Load more';
         }
      }
    }
  }else{
    const sc = database.connections.find((c) => c.id === database.selectedConnId);
    if(sc){
      
       { svelteHTML.createElement("div", {   "class":`schema-conn`,"title":`${sc.name} · ${sc.kind}`,});
         { svelteHTML.createElement("span", { "class":`conn-glyph ${sc.kind}`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":engineGlyph(sc.kind),"size":12,}});} }
         { svelteHTML.createElement("span", { "class":`schema-conn-name ellipsis`,});sc.name; }
        if(envBadge(sc)){ { const $$_egdaBvnE1C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE1C({ target: __sveltets_2_any(), props: {    "env":sc.environment,"readOnly":sc.read_only,}});}}
         { const $$_egdaB1C = __sveltets_2_ensureComponent(Badge); new $$_egdaB1C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":sc.kind,}});}
       }
    }
     { const $$_eerTamehcS0C = __sveltets_2_ensureComponent(SchemaTree); new $$_eerTamehcS0C({ target: __sveltets_2_any(), props: {}});}
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Secondary views and the SSH/Kafka/SFTP panes load on first open, not with
  // the page: xterm, the Kafka viewer (+ lag alerts) and ~4.7k lines of
  // builder/structure/diagram used to ride in the page chunk every DB open
  // paid for. The common views are warmed on idle after first paint; the
  // terminal/Kafka/SFTP chunks only when a pane of that kind opens.
  const QueryBuilderLazy = lazyComponent(() => import('./QueryBuilder.svelte'));
  const StructureLazy = lazyComponent(() => import('./StructureView.svelte'));
  const DiagramLazy = lazyComponent(() => import('./DiagramView.svelte'));
  const DashboardsLazy = lazyComponent(() => import('./Dashboards.svelte'));
  const TerminalLazy = lazyComponent(() => import('../../lib/components/Terminal.svelte'));
  const ClusterViewerLazy = lazyComponent(() => import('../brokers/ClusterViewer.svelte'));
  const ClusterFormLazy = lazyComponent(() => import('../brokers/ClusterForm.svelte'));
  const SftpLazy = lazyComponent(() => import('../connections/SftpBrowser.svelte'));
  // The assistant embeds a Terminal (xterm) — only loaded when it's opened.
  const AssistantLazy = lazyComponent(() => import('./DbAssistantPanel.svelte'));
  $effect(() =>
    whenIdle(() => {
      StructureLazy.prefetch();
      QueryBuilderLazy.prefetch();
      DashboardsLazy.prefetch();
      DiagramLazy.prefetch();
    }, 4000),
  );

  // The unified Connections hub: EVERY profile kind is created/managed in this
  // tree — DB engines open the workbench, ssh/custom open a terminal session,
  // Kafka clusters open Message Brokers.
  const ALL_KINDS: ConnectionKind[] = [
    'ssh',
    'mysql',
    'postgres',
    'redis',
    'mongodb',
    'clickhouse',
    'custom',
  ];
  let connFormOpen = $state(false);
  let editingConn = $state<Connection | null>(null);
  // Import connection profiles from other DB tools (MySQL Workbench / DBeaver /
  // DataGrip / NoSQLBooster) — the daemon reads each tool's config from disk.
  let connImportOpen = $state(false);
  // Kafka cluster form (New Cluster / edit a cluster row).
  let clusterFormOpen = $state(false);
  let editingCluster = $state<BrokerCluster | null>(null);
  // SFTP file browser over an ssh profile's row action.
  let sftpFor = $state<Connection | null>(null);
  // Spinner guard while a terminal session is being spawned for a row.
  let opening = $state<Record<string, boolean>>({});

  // ── Type-filter chips (single-select; 'kafka' matches broker clusters) ─────
  const FILTER_KEY = 'otto_connhub_filter';
  const FILTER_CHIPS: { id: string; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'ssh', label: 'SSH' },
    { id: 'mysql', label: 'MySQL' },
    { id: 'postgres', label: 'PostgreSQL' },
    { id: 'redis', label: 'Redis' },
    { id: 'mongodb', label: 'MongoDB' },
    { id: 'clickhouse', label: 'ClickHouse' },
    { id: 'kafka', label: 'Kafka' },
    { id: 'custom', label: 'Custom' },
  ];
  // Guarded storage: this runs at mount, so a throwing accessor (blocked
  // storage) must not take the whole Database page down.
  let filterKind = $state(lsGet(FILTER_KEY) || 'all');
  function setFilter(id: string): void {
    filterKind = id;
    lsSet(FILTER_KEY, id);
  }
  // Only offer kinds that exist (plus the active one, so a remembered filter
  // can always be cleared). One kind or none → no chip row at all: a filter
  // that can't narrow anything is noise.
  const presentKinds = $derived.by(() => {
    const kinds = new Set<string>([...database.connections, ...database.otherConnections].map((c) => c.kind));
    if (brokers.clusters.length > 0) kinds.add('kafka');
    return kinds;
  });
  const visibleChips = $derived(
    FILTER_CHIPS.filter((chip) => chip.id === 'all' || chip.id === filterKind || presentKinds.has(chip.id)),
  );
  const filtering = $derived(filterKind !== 'all');
  const connMatchesKind = (c: Connection): boolean => !filtering || c.kind === filterKind;
  const clusterMatchesKind = (): boolean => !filtering || filterKind === 'kafka';

  // ── Saved / History sidebar (search + inline rename) ──────────────────────
  let savedSearch = $state('');
  let historySearch = $state('');
  let renamingId = $state<string | null>(null);
  let renameDraft = $state('');
  // Lower-cased search text per row, memoized on the row's text — not
  // re-lowercasing every full statement (which can be a whole pasted script)
  // on each search keystroke.
  const searchText = new WeakMap<object, { src: string; lower: string }>();
  function lowered(row: object, text: string): string {
    const hit = searchText.get(row);
    if (hit && hit.src === text) return hit.lower;
    const lower = text.toLowerCase();
    searchText.set(row, { src: text, lower });
    return lower;
  }
  const filteredSaved = $derived.by(() => {
    const s = savedSearch.trim().toLowerCase();
    if (!s) return database.savedQueries;
    return database.savedQueries.filter(
      (q) => q.name.toLowerCase().includes(s) || lowered(q, q.statement).includes(s),
    );
  });
  const filteredHistory = $derived.by(() => {
    const s = historySearch.trim().toLowerCase();
    if (!s) return database.history;
    return database.history.filter((h) =>
      lowered(h, h.error ? `${h.statement}\n${h.error}` : h.statement).includes(s),
    );
  });
  function startRename(q: DbSavedQuery): void {
    renamingId = q.id;
    renameDraft = q.name;
  }
  async function commitRename(): Promise<void> {
    const id = renamingId;
    if (id && renameDraft.trim()) await database.renameSavedQuery(id, renameDraft);
    renamingId = null;
  }
  function cancelRename(): void {
    renamingId = null;
  }

  // ── Phone accordion ────────────────────────────────────────────────────────
  // On a phone the whole page scrolls and each major section (Connections /
  // Schema) is a collapsible, independently-scrolling block — tap a header to
  // expand/minimise so the user keeps only what they need on screen. These
  // flags are inert on desktop/tablet (the headers only render when isPhone).
  let connOpen = $state(true);
  let schemaOpen = $state(true);

  // Dock this connection as a pane in the Agents split (beside an agent), with
  // the full DB Explorer. Right-clicked from a connection tab or sidebar row.
  function openConnInAgents(c: Connection): void {
    void database.openConnection(c.id);
    ws.openInSplit(DB_PANE_ID);
    router.go('agents');
  }
  // The folder path a connection sits under, e.g. "PLATFORM / STG" — so it's
  // clear which environment (stg/prod) a connection belongs to.
  function sectionPath(c: Connection): string {
    if (!c.section_id) return '';
    const byId = new Map(sections.map((s) => [s.id, s]));
    const parts: string[] = [];
    let cur = byId.get(c.section_id);
    let guard = 0;
    while (cur && guard++ < 20) {
      parts.unshift(cur.name);
      cur = cur.parent_id ? byId.get(cur.parent_id) : undefined;
    }
    return parts.join(' / ');
  }
  // The immediate folder (sub-section), e.g. "STG" — compact, for the tab badge.
  function sectionLeaf(c: Connection): string {
    if (!c.section_id) return '';
    return sections.find((s) => s.id === c.section_id)?.name ?? '';
  }
  $effect(()=>resourceAccess.subscribe(change=>{
    if(change.type==='reset' && change.identity){connFormOpen=false;connImportOpen=false;editingConn=null;accessFor=null;sftpFor=null;changesOpen=false;}
  }));
  let changesOpen = $state(false);
  let accessFor = $state<Connection | null>(null);
  const connectionAccess = (c: Connection, operation: string, capability: 'view'|'edit'|'admin'='edit') => resourceAccess.can('connection',c.id,operation,'connections',capability);
  $effect(() => { for (const c of [...database.connections,...database.otherConnections]) void resourceAccess.load('connection',c.id); });
  // The "Open a connection" pane offers the most recently opened ones as a
  // one-click reopen instead of only pointing at the list.
  const recentConns = $derived(
    database.connections
      .filter((c) => c.last_opened_at)
      .sort((a, b) => Date.parse(b.last_opened_at!) - Date.parse(a.last_opened_at!))
      .slice(0, 5),
  );
  function connMenu(e: MouseEvent, c: Connection): void {
    const isDb = database.connections.some((x) => x.id === c.id);
    ctxMenu.show(e, [
      // Per-kind primary/secondary opens: DB kinds live in the workbench but
      // keep their CLI client; ssh adds the SFTP file browser.
      ...(isDb
        ? [
            { label: 'Open beside agents (split)', icon: 'split', action: () => openConnInAgents(c) },
            ...(connectionAccess(c,'shell') ? [{ label: 'Open terminal client', icon: 'terminal', action: () => void openTerminal(c) }] : []),
          ]
        : connectionAccess(c,'shell') ? [{ label: 'Open terminal session', icon: 'terminal', action: () => void openTerminal(c) }] : []),
      ...(c.kind === 'ssh' && connectionAccess(c,'sftp_read','view')
        ? [{ label: 'Browse files (SFTP)', icon: 'folder', action: () => (sftpFor = c) }]
        : []),
      ...(isDb && database.isWarming(c.id)
        ? [{ label: 'Stop connecting', icon: 'stop', action: () => database.stopConnecting(c.id) }]
        : []),
      ...(isDb && database.connStatus.get(c.id)?.phase === 'idle' && database.selectedConnId !== c.id
        ? [{ label: 'Connect in background', icon: 'refresh', action: () => void database.warm(c.id) }]
        : []),
      ...(isDb ? popoutItems(`database/${c.id}`, c.name) : []),
      { separator: true },
      ...(connectionAccess(c,'configure','admin') ? [{ label: 'Edit', icon: 'edit', action: () => editConnection(c) }, { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteConnection(c) }] : []),
      ...(auth.isRoot && connectionAccess(c,'configure','admin') ? [{ label: 'Duplicate without password', icon: 'copy', action: () => void duplicateConnection(c) }] : []),
      ...(auth.isRoot || connectionAccess(c,'manage_access','admin') ? [{ label: 'Access', icon: 'key', action: () => {accessFor=c;} }] : []),
    ]);
  }

  /** A Kafka cluster's menu — its sidebar row and its open tab (right-click or ⋯). */
  function clusterMenu(e: MouseEvent, cl: BrokerCluster): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'split', action: () => openCluster(cl) },
      { label: 'Open in Message Brokers page', icon: 'split', action: () => openClusterStandalone(cl) },
      { separator: true },
      { label: 'Edit', icon: 'edit', action: () => editCluster(cl) },
      { label: 'Remove…', icon: 'trash', danger: true, action: () => void deleteCluster(cl) },
    ]);
  }

  async function duplicateConnection(c: Connection): Promise<void> {
    try {
      const copy = await api.post<Connection>(`/connections/${c.id}/duplicate`, {});
      await database.loadConnections();
      editConnection(copy);
      toasts.info('Configuration duplicated', 'Set a password for the new connection. Access grants were not copied.');
    } catch (error) { toastError('Couldn’t duplicate connection', error); }
  }

  // --- Section hierarchy (THE unified tree: every kind + broker clusters) ----
  interface TreeNode {
    sec: ConnectionSection;
    items: Connection[];
    clusters: BrokerCluster[];
    children: TreeNode[];
  }
  let sections = $state<ConnectionSection[]>([]);
  let collapsed = $state<Record<string, boolean>>({});
  let draggedConnId = $state<string | null>(null);
  let draggedClusterId = $state<string | null>(null);
  let draggedSectionId = $state<string | null>(null);

  const sortByName = (a: Connection, b: Connection): number => a.name.localeCompare(b.name);
  const sortClusters = (a: BrokerCluster, b: BrokerCluster): number =>
    a.name.localeCompare(b.name);

  // Every profile the tree shows (DB kinds + ssh/custom), kind-filtered.
  const allConns = $derived(
    [...database.connections, ...database.otherConnections].filter(connMatchesKind),
  );
  const treeClusters = $derived(clusterMatchesKind() ? brokers.clusters : []);

  // Build the section tree from the flat list; `parentId = null` is the root.
  function buildTree(parentId: string | null): TreeNode[] {
    return sections
      .filter((s) => (s.parent_id ?? null) === parentId)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .map((sec) => ({
        sec,
        items: allConns.filter((c) => c.section_id === sec.id).sort(sortByName),
        clusters: treeClusters.filter((cl) => cl.section_id === sec.id).sort(sortClusters),
        children: buildTree(sec.id),
      }));
  }
  const tree = $derived(buildTree(null));
  // Known folder ids in the (global, shared) tree. A connection whose folder is
  // not among them falls back to Ungrouped, so connections never vanish.
  const knownSectionIds = $derived(new Set(sections.map((s) => s.id)));
  const ungrouped = $derived(
    allConns.filter((c) => !c.section_id || !knownSectionIds.has(c.section_id)).sort(sortByName),
  );
  const ungroupedClusters = $derived(
    treeClusters
      .filter((cl) => !cl.section_id || !knownSectionIds.has(cl.section_id))
      .sort(sortClusters),
  );
  /** Matching descendants under a node (drives count chips + hide-when-empty
   *  while a type filter narrows the tree). */
  function nodeCount(n: TreeNode): number {
    return (
      n.items.length + n.clusters.length + n.children.reduce((sum, c) => sum + nodeCount(c), 0)
    );
  }
  // Sections holding anything at all (any kind, unfiltered), ancestors
  // included. A truly empty folder isn't "filtered out" — it has nothing to
  // filter — so it stays visible under a type filter as a drop target.
  const occupiedSectionIds = $derived.by(() => {
    const parentOf = new Map(sections.map((s) => [s.id, s.parent_id ?? null]));
    const occ = new Set<string>();
    const mark = (id: string | null | undefined): void => {
      let cur = id ?? null;
      while (cur && parentOf.has(cur) && !occ.has(cur)) {
        occ.add(cur);
        cur = parentOf.get(cur) ?? null;
      }
    };
    for (const c of database.connections) mark(c.section_id);
    for (const c of database.otherConnections) mark(c.section_id);
    for (const cl of brokers.clusters) mark(cl.section_id);
    return occ;
  });
  // While anything is being dragged, a type filter reveals every folder so
  // any section is a reachable drop target. Flipped on a macrotask: mutating
  // the DOM inside `dragstart` makes WebKit abort the drag.
  let dragReveal = $state(false);
  $effect(() => {
    const dragging = !!(draggedConnId || draggedClusterId || draggedSectionId);
    if (!dragging) {
      dragReveal = false;
      return;
    }
    const t = setTimeout(() => (dragReveal = true), 0);
    return () => clearTimeout(t);
  });
  /** Under a type filter a folder shows when it has matches, is empty, has a
   *  visible sub-folder, or a drag is in progress. */
  function nodeVisible(n: TreeNode): boolean {
    return (
      !filtering ||
      dragReveal ||
      nodeCount(n) > 0 ||
      !occupiedSectionIds.has(n.sec.id) ||
      n.children.some(nodeVisible)
    );
  }

  // --- Connection search / filter --------------------------------------------
  // A filter box over the connection list (mirrors SchemaTree's "Filter schema").
  // When it has text we show a flat, name-sorted result list instead of the tree
  // so a connection is findable instantly even with a hundred of them.
  let connFilter = $state('');
  // Compact host/uri descriptor used for matching (and shown on flat results).
  function connDesc(c: Connection): string {
    const p = (c.params ?? {}) as Record<string, unknown>;
    const host = String(p.host ?? p.uri ?? p.url ?? p.path ?? '');
    const port = p.port != null && p.port !== '' ? `:${String(p.port)}` : '';
    return `${host}${port}`;
  }
  const connMatches = $derived.by(() => {
    const q = connFilter.trim().toLowerCase();
    if (!q) return [];
    return allConns
      .filter(
        (c) =>
          c.name.toLowerCase().includes(q) ||
          c.kind.toLowerCase().includes(q) ||
          connDesc(c).toLowerCase().includes(q) ||
          sectionPath(c).toLowerCase().includes(q),
      )
      .sort(sortByName);
  });
  const clusterMatches = $derived.by(() => {
    const q = connFilter.trim().toLowerCase();
    if (!q) return [];
    return treeClusters
      .filter(
        (cl) =>
          cl.name.toLowerCase().includes(q) ||
          'kafka'.includes(q) ||
          cl.bootstrap_servers.toLowerCase().includes(q),
      )
      .sort(sortClusters);
  });

  async function loadSections(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    try {
      // One global tree shared with the Connections page.
      sections = await api.get<ConnectionSection[]>(`/workspaces/${wsId}/connection-sections`);
    } catch {
      /* sections are optional — fall back to a flat list */
    }
  }

  function toggleCollapse(id: string): void {
    collapsed[id] = !collapsed[id];
  }

  async function createSection(parentId: string | null): Promise<void> {
    if (!ws.currentId) return;
    const name = await confirmer.promptText(parentId ? 'Sub-section name' : 'Section name', {
      title: parentId ? 'New sub-section' : 'New section',
      confirmLabel: 'Create',
      placeholder: 'e.g. AWS · STG',
    });
    if (!name) return;
    try {
      const sec = await api.post<ConnectionSection>(
        `/workspaces/${ws.currentId}/connection-sections`,
        { name, parent_id: parentId },
      );
      sections = [...sections, sec];
    } catch (e) {
      toastError('Couldn’t create the section', e);
    }
  }

  async function renameSection(sec: ConnectionSection): Promise<void> {
    const name = await confirmer.promptText('Rename section', {
      title: 'Rename section',
      confirmLabel: 'Rename',
      initial: sec.name,
    });
    if (!name || name === sec.name) return;
    try {
      const updated = await api.patch<ConnectionSection>(`/connection-sections/${sec.id}`, { name });
      sections = sections.map((s) => (s.id === sec.id ? updated : s));
    } catch (e) {
      toastError('Couldn’t rename', e);
    }
  }

  async function deleteSection(sec: ConnectionSection): Promise<void> {
    if (
      !(await confirmer.ask(
        `Delete section “${sec.name}”? Sub-sections are removed too and their connections become ungrouped.`,
        { title: 'Delete section', confirmLabel: 'Delete', danger: true },
      ))
    )
      return;
    try {
      await api.del(`/connection-sections/${sec.id}`);
      const removed = new Set<string>();
      const collect = (id: string): void => {
        removed.add(id);
        for (const s of sections) if (s.parent_id === id) collect(s.id);
      };
      collect(sec.id);
      sections = sections.filter((s) => !removed.has(s.id));
      // Locally drop the section_id of anything that fell out (rows never vanish).
      database.connections = database.connections.map((c) =>
        c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c,
      );
      database.otherConnections = database.otherConnections.map((c) =>
        c.section_id && removed.has(c.section_id) ? { ...c, section_id: null } : c,
      );
      brokers.clusters = brokers.clusters.map((cl) =>
        cl.section_id && removed.has(cl.section_id) ? { ...cl, section_id: null } : cl,
      );
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  // Move a connection into a folder (or null = ungrouped). Connections are
  // global, so all are assignable. Reuses the PATCH endpoint, updates in place.
  async function moveConn(c: Connection, sectionId: string | null): Promise<void> {
    if ((c.section_id ?? null) === sectionId) return;
    try {
      const saved = await api.patch<Connection>(`/connections/${c.id}`, {
        name: c.name,
        kind: c.kind,
        params: c.params,
        first_command: c.first_command,
        section_id: sectionId,
        // Preserve the guardrail flags — omitting them would reset to dev/false.
        environment: c.environment,
        read_only: c.read_only,
      });
      database.connections = database.connections.map((x) => (x.id === c.id ? saved : x));
      database.otherConnections = database.otherConnections.map((x) =>
        x.id === c.id ? saved : x,
      );
    } catch (e) {
      toastError('Couldn’t move', e);
    }
  }

  function isDescendantOf(nodeId: string, ancestorId: string): boolean {
    let cur = sections.find((s) => s.id === nodeId);
    while (cur?.parent_id) {
      if (cur.parent_id === ancestorId) return true;
      cur = sections.find((s) => s.id === cur!.parent_id);
    }
    return false;
  }

  async function reparentSection(id: string, parentId: string | null): Promise<void> {
    const sec = sections.find((s) => s.id === id);
    if (!sec || (sec.parent_id ?? null) === parentId) return;
    if (parentId && (parentId === id || isDescendantOf(parentId, id))) {
      toasts.error('Invalid move', 'Cannot nest a section inside itself');
      return;
    }
    try {
      const updated = await api.post<ConnectionSection>(`/connection-sections/${id}/move`, {
        parent_id: parentId,
      });
      sections = sections.map((s) => (s.id === id ? updated : s));
    } catch (e) {
      toastError('Couldn’t move', e);
    }
  }

  // A drop onto a section: a dragged connection/cluster files into it; a
  // dragged section nests under it. A drop onto the root zone reverses all.
  function findAnyConn(id: string): Connection | undefined {
    return (
      database.connections.find((x) => x.id === id) ??
      database.otherConnections.find((x) => x.id === id)
    );
  }
  function onSectionDrop(sectionId: string): void {
    if (draggedConnId) {
      const c = findAnyConn(draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, sectionId);
    } else if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void brokers.moveCluster(id, sectionId);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, sectionId);
    }
  }
  function onRootDrop(): void {
    if (draggedConnId) {
      const c = findAnyConn(draggedConnId);
      draggedConnId = null;
      if (c) void moveConn(c, null);
    } else if (draggedClusterId) {
      const id = draggedClusterId;
      draggedClusterId = null;
      void brokers.moveCluster(id, null);
    } else if (draggedSectionId) {
      const src = draggedSectionId;
      draggedSectionId = null;
      void reparentSection(src, null);
    }
  }

  function newConnection(): void {
    if(!auth.isRoot)return;
    editingConn = null;
    connFormOpen = true;
  }
  async function showSchemaSidebar(): Promise<void> {
    const page = document.querySelector('.db-page');
    database.toggleSidebar();
    await tick();
    const selected = [...(page?.querySelectorAll<HTMLElement>('[data-node-id]') ?? [])]
      .find((node) => node.dataset.nodeId === database.selectedObjectPath);
    (selected?.querySelector<HTMLButtonElement>('.node-label') ?? selected
      ?? page?.querySelector<HTMLInputElement>('[aria-label="Find an object"]'))?.focus();
  }

  // Point the user at the connection list from anywhere: make sure the sidebar
  // rail is actually visible (it may be collapsed) and land on the picker tab.
  function showConnections(): void {
    if (database.sidebarCollapsed) database.toggleSidebar();
    database.setSideTab('connections');
  }
  // The persistent "+" in the side-tab strip: jump to the picker AND open the
  // new-connection form — the create affordance must not live only inside the
  // Connections tab body.
  function newConnectionFromStrip(): void {
    showConnections();
    newConnection();
  }
  function editConnection(c: Connection): void {
    editingConn = c;
    connFormOpen = true;
  }

  // ⌘K: the Connections hub's verbs.
  $effect(() =>
    registry.register('connections', [
      { id: 'conn.new', title: 'New connection…', group: 'Connections', keywords: 'database ssh mysql postgres mongo redis clickhouse kafka add', disabled: !auth.isRoot, run: newConnectionFromStrip },
      { id: 'conn.import', title: 'Import connections…', group: 'Connections', keywords: 'workbench dbeaver datagrip nosqlbooster', disabled: !auth.isRoot, run: () => (connImportOpen = true) },
      { id: 'conn.list', title: 'Show the connection list', group: 'Connections', keywords: 'picker sidebar profiles', run: showConnections },
      { id: 'conn.sidebar', title: database.sidebarCollapsed ? 'Show the schema sidebar' : 'Hide the schema sidebar', group: 'Connections', shortcut: '⌘B', keywords: 'toggle collapse tree', run: () => (database.sidebarCollapsed ? void showSchemaSidebar() : database.toggleSidebar()) },
    ]),
  );
  async function onConnSaved(c: Connection): Promise<void> {
    connFormOpen = false;
    await database.loadConnections();
    // DB kinds open straight into the workbench; ssh/custom just appear in the
    // tree (opening them spawns a terminal, which the user does deliberately).
    if (database.connections.some((x) => x.id === c.id)) void database.openConnection(c.id);
  }
  function newCluster(): void {
    editingCluster = null;
    clusterFormOpen = true;
  }
  function editCluster(cl: BrokerCluster): void {
    editingCluster = cl;
    clusterFormOpen = true;
  }
  // Open a Kafka cluster as a workbench tab (in place, next to the DB tabs) —
  // the brokers store owns the tab list + selection; `focusKafka` points the main
  // area at it. No navigation: the viewer renders right here.
  function openCluster(cl: BrokerCluster): void {
    brokers.select(cl.id);
    database.focusKafka(cl.id);
  }
  // Escape hatch: open the cluster in the standalone Message Brokers page.
  function openClusterStandalone(cl: BrokerCluster): void {
    brokers.select(cl.id);
    router.go('brokers');
  }
  async function deleteCluster(cl: BrokerCluster): Promise<void> {
    if (
      // Same verb + consequence as the Message Brokers page and ClusterViewer's
      // "Remove" button, which lands here.
      !(await confirmer.ask(
        `Remove cluster “${cl.name}”? Its saved settings and Keychain secrets are removed from Otto; topics on the broker are not touched.`,
        { title: 'Remove cluster', confirmLabel: 'Remove', danger: true },
      ))
    )
      return;
    try {
      await brokers.remove(cl.id);
    } catch (e) {
      toastError('Couldn’t remove', e);
    }
  }

  /** Open an ssh/custom profile (or a DB kind's CLI client) as a live terminal
   *  tab in the workbench — the terminal renders in place, seamlessly, like the
   *  DB kinds. The session is also registered in the Agents list. */
  async function openTerminal(c: Connection): Promise<void> {
    // Already open as a workbench tab → just focus it (don't spawn a 2nd session).
    if (database.sshTabs.some((t) => t.connId === c.id)) {
      database.focusSsh(c.id);
      return;
    }
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.error(NO_WORKSPACE, 'The database session is attached to a workspace — create or pick one.');
      return;
    }
    opening[c.id] = true;
    announce(`Opening ${c.name}…`);
    try {
      const session = await api.long.post<Session>(`/connections/${c.id}/open`, {
        workspace_id: wsId,
      });
      // Open it in place as a workbench tab (like the DB kinds). Deliberately NOT
      // registered via `ws.addSession` — that would navigate to the Agents view
      // and surface the terminal there. The connection lives only in this hub; the
      // session is killed when the tab closes (see `closeSshTerminal`).
      database.addSshTab({ connId: c.id, sessionId: session.id, name: c.name, kind: c.kind });
    } catch (e) {
      toastError('Couldn’t open', e);
    } finally {
      opening[c.id] = false;
    }
  }

  /** Close an SSH/custom terminal tab and kill its underlying session so it
   *  doesn't linger on the daemon (it isn't tracked in the Agents list). */
  async function closeSshTerminal(connId: string): Promise<void> {
    const tab = database.sshTabs.find((t) => t.connId === connId);
    database.closeSshTab(connId);
    if (tab) {
      try {
        await api.del(`/sessions/${tab.sessionId}`);
      } catch {
        // best-effort: the tab is gone from the workbench regardless
      }
    }
  }
  async function deleteConnection(c: Connection): Promise<void> {
    if (
      !(await confirmer.ask(`Delete connection “${c.name}”? Its Keychain secret is removed too.`, {
        title: 'Delete connection',
        confirmLabel: 'Delete',
        danger: true,
      }))
    )
      return;
    try {
      await api.del(`/connections/${c.id}`);
      if (database.openConnIds.includes(c.id)) database.closeConnection(c.id);
      await database.loadConnections();
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  // Load connections + workspace-scoped saved/dashboards when the workspace changes.
  // The workbench itself is GLOBAL — open connection tabs survive workspace
  // switches. Restore the persisted workbench (open tabs + focus) ONLY after
  // loadConnections resolves — the restore filters against the loaded list.
  $effect(() => {
    if (ws.currentId) {
      void (async () => {
        await database.loadConnections();
        connsSettled = true;
        await database.restoreWorkbench();
        // `#/database/<connId>` (a pop-out window, a link) opens that tab.
        const deep = router.module === 'database' ? router.parts[1] : undefined;
        if (deep && database.connections.some((c) => c.id === deep)) await database.openConnection(deep);
      })();
      void loadSections();
      void brokers.load(ws.currentId); // clusters render in the same tree
      void database.loadSavedQueries();
      void database.loadDashboards();
    }
  });

  /** The first connection load has finished (so "nothing listed" is real, not
   *  "not fetched yet") — gates the empty-hub layout below. */
  let connsSettled = $state(false);
  /** Nothing to list at all: the list pane is hidden and one page-level
   *  EmptyState owns the "New connection" CTA (no duplicate link in the list). */
  const hubEmpty = $derived(
    connsSettled &&
      !database.connectionsLoading &&
      !database.connectionsError &&
      database.connections.length === 0 &&
      database.otherConnections.length === 0 &&
      brokers.clusters.length === 0 &&
      sections.length === 0,
  );

  /** Tooltip for the connection health chip: "Connected · <version> · <ms> ms". */
  function healthTitle(st: { serverVersion?: string; latencyMs?: number }): string {
    const parts = ['Connected'];
    if (st.serverVersion) parts.push(st.serverVersion);
    if (st.latencyMs != null) parts.push(`${st.latencyMs} ms`);
    return parts.join(' · ');
  }

  // Sidebar views. On a phone the connection list is its own accordion, so
  // the strip there starts at Schema ("connections" shows as Schema).
  const SIDE_TABS: TabItem<DbSideTab>[] = [
    { id: 'connections', label: 'Connections' },
    { id: 'schema', label: 'Schema' },
    { id: 'saved', label: 'Saved' },
    { id: 'history', label: 'History' },
  ];
  const MOBILE_SIDE_TABS = SIDE_TABS.slice(1);
  const mainTabs: { id: DbMainTab; label: string; icon: IconName; show: () => boolean }[] = [
    { id: 'query', label: 'Query', icon: 'terminal', show: () => true },
    { id: 'builder', label: 'Builder', icon: 'layers', show: () => database.supportsBuilder },
    { id: 'structure', label: 'Structure', icon: 'columns', show: () => true },
    // ERD is table/collection-oriented; Redis (keys, no table model) is excluded.
    { id: 'diagram', label: 'Diagram', icon: 'shapes', show: () => database.capabilities?.engine !== 'redis' },
    { id: 'dashboards', label: 'Dashboards', icon: 'chart', show: () => true },
  ];
  const visibleTabs = $derived(mainTabs.filter((t) => t.show()));
  // ── DB Assistant split (resizable, persisted) ────────────────────────────────
  // When open, the DB Assistant panel sits BESIDE the editor/results, separated by
  // a draggable divider so the user can enlarge the agent's shell. Mirrors the
  // query-editor's own resizable-pane idiom (pointer drag + localStorage px).
  let assistW = $state(loadAssistW());
  function loadAssistW(): number {
    const v = Number(lsGet('db.assistW'));
    return Number.isFinite(v) && v > 280 ? v : 460;
  }
  function persistAssistW(): void {
    lsSet('db.assistW', String(Math.round(assistW)));
  }
  const assistMaxW = (): number => Math.max(320, (typeof window !== 'undefined' ? window.innerWidth : 1280) - 360);
  const pxText = (v: number): string => `${Math.round(v)} pixels wide`;
  function setAssistW(w: number): void {
    assistW = w;
    persistAssistW();
  }
  function startAssistResize(e: PointerEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = assistW;
    const maxW = assistMaxW();
    const onMove = (ev: PointerEvent): void => {
      // The panel is pinned to the right edge, so dragging LEFT widens it.
      assistW = Math.max(300, Math.min(maxW, startW + (startX - ev.clientX)));
    };
    const onUp = (): void => {
      persistAssistW();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    // A cancelled gesture (system gesture, focus loss) ends the drag too —
    // else pointermove stays attached and the pane follows the cursor.
    window.addEventListener('pointercancel', onUp);
  }

  // ── Connection sidebar width (resizable, persisted) ───────────────────────────
  // The tablet/desktop sidebar is drag-resizable so long, deeply-nested connection
  // names ("DB MySQL - Platform Aggregates Prod") get the horizontal room they need;
  // the chosen width survives reloads. Mirrors the assist-pane idiom above. (On
  // phones the sidebar is a full-width band — the width binding is skipped there.)
  // Documented exception to LIST_PANE: the default is 300 (not 280) and the max
  // tracks the window (up to 640), because this one pane also hosts the schema
  // tree, Saved and History — deeper, wider content than a plain list.
  const SIDE_DEFAULT = 300;
  let sideW = $state(loadPaneWidth('db.sideW', SIDE_DEFAULT, LIST_PANE.min, 640));
  // Leave room for the editor/results area; cap so the sidebar can't eat the page.
  const sideMaxW = (): number => Math.min(640, Math.max(360, (typeof window !== 'undefined' ? window.innerWidth : 1280) - 420));
  // Open connections as top-level tabs (Workbench-style), resolved to their
  // Connection records for name + engine glyph.
  const openConns = $derived(
    database.openConnIds
      .map((id) => database.connections.find((c) => c.id === id))
      .filter((c): c is NonNullable<typeof c> => c != null),
  );

  // The unified workbench renders three kinds of tab: DB connections (openConns),
  // Kafka clusters (brokers store), and ssh/custom terminals (database.sshTabs).
  const hasAnyTab = $derived(
    openConns.length > 0 || brokers.openClusters.length > 0 || database.sshTabs.length > 0,
  );
  // No auto-open on arrival: opening a connection connects to the server (it
  // may be prod), so the empty main pane shows the collection summary instead.
  // "6 connections · 2 prod · 1 Kafka cluster" — what the empty main pane says.
  const hubSummary = $derived.by(() => {
    const dbs = database.connections.length + database.otherConnections.length;
    const prod = [...database.connections, ...database.otherConnections].filter((c) => c.environment === 'prod').length;
    const k = brokers.clusters.length;
    const parts = [`${plural(dbs, 'connection')}`];
    if (prod > 0) parts.push(`${prod} prod`);
    if (k > 0) parts.push(plural(k, 'Kafka cluster'));
    return `${parts.join(' · ')}.`;
  });
  // An empty workbench (fresh start, after a restore that found nothing, or the
  // last tab closed) always surfaces the picker — a schema/saved/history side
  // tab with no connection behind it is a dead end.
  $effect(() => {
    if (!hasAnyTab && database.sideTab !== 'connections') database.setSideTab('connections');
  });
  // The cluster / ssh session backing the active non-DB pane (null when a DB tab
  // is focused, or when the backing tab was closed out from under the pane).
  const activeCluster = $derived(
    database.activePane?.kind === 'kafka'
      ? (brokers.openClusters.find((c) => c.id === database.activePane!.id) ?? null)
      : null,
  );
  const activeSsh = $derived(
    database.activePane?.kind === 'ssh'
      ? (database.sshTabs.find((t) => t.connId === database.activePane!.id) ?? null)
      : null,
  );
  // Roving tabindex over the connection strip: the selected tab is the one Tab
  // stop; with none selected, the first tab is, so the strip stays reachable.
  const connTabActive = $derived(
    activeCluster != null ||
      activeSsh != null ||
      (database.activePane === null && openConns.some((c) => c.id === database.selectedConnId)),
  );
  const firstConnTab = $derived(
    openConns[0] ? `db:${openConns[0].id}`
      : brokers.openClusters[0] ? `kafka:${brokers.openClusters[0].id}`
      : database.sshTabs[0] ? `ssh:${database.sshTabs[0].connId}`
      : '',
  );
  const connTabStop = (on: boolean, key: string): number => (on || (!connTabActive && firstConnTab === key) ? 0 : -1);

  // The open-connections strip scrolls sideways with its scrollbar hidden, so a
  // tab opened (or focused) past the right edge would be selected but invisible —
  // with nothing hinting the strip scrolls. Bring the active tab into view
  // whenever the selection or the set of open tabs changes.
  let connTabsEl = $state<HTMLElement | null>(null);
  $effect(() => {
    void database.selectedConnId;
    void database.activePane;
    void openConns.length;
    void brokers.openClusters.length;
    void database.sshTabs.length;
    const strip = connTabsEl;
    if (!strip) return;
    requestAnimationFrame(() =>
      strip.querySelector<HTMLElement>('.conn-tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' }),
    );
  });

  // Close a Kafka cluster tab; follow the brokers store's neighbour reselection
  // (or drop back to the DB workbench when no clusters remain open).
  function closeKafkaTab(id: string): void {
    const wasActive = database.activePane?.kind === 'kafka' && database.activePane.id === id;
    brokers.close(id);
    if (wasActive) {
      if (brokers.selectedId) database.focusKafka(brokers.selectedId);
      else database.activePane = null;
    }
  }

  // ── Environment guardrail (danger styling) ─────────────────────────────────
  // Prod connections are dangerous; read-only are locked. Both get a badge, and
  // the selected one draws a red rail down the main area as a constant reminder.
  // Structural pick so broker clusters (same guard fields) share the badges.
  type EnvGuarded = Pick<Connection, 'environment' | 'read_only'>;
  const isProdConn = (c: EnvGuarded): boolean => c.environment === 'prod';
  const isGuardedConn = (c: EnvGuarded): boolean => c.environment === 'prod' || c.read_only;
  // Whether a row gets the shared <EnvBadge> — the same rule as Brokers /
  // Kubernetes / AWS: prod and staging are badged, and so is read-only (RO);
  // dev (the default) stays unbadged. Uses `envTone` so env aliases agree with
  // the badge itself.
  function envBadge(c: EnvGuarded): boolean {
    return envTone(c.environment).key !== 'dev' || c.read_only;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`db-root`,});

 { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { "title":`Connections`,leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(!viewport.isPhone && !hubEmpty){
       { svelteHTML.createElement("button", {             "class":`icon-btn`,"onclick":() => (database.sidebarCollapsed ? void showSchemaSidebar() : database.toggleSidebar()),"aria-label":database.sidebarCollapsed ? 'Show schema sidebar' : 'Hide sidebar',"title":database.sidebarCollapsed ? 'Show sidebar (⌘B)' : 'Hide sidebar (⌘B)',"aria-expanded":!database.sidebarCollapsed,"aria-controls":`db-side`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`sidebar`,"size":16,}});}
       }
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(!viewport.isPhone){
      if(!hubEmpty){
       { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":!auth.isRoot,"onclick":() => (connImportOpen = true),"title":auth.isRoot ? 'Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster' : 'Only the owner can import connections',});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});} 
       }
      }
      if(database.connections.length > 0 || database.otherConnections.length > 0 || brokers.clusters.length > 0){
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":!auth.isRoot,"onclick":newConnectionFromStrip,"title":auth.isRoot ? 'New connection (SSH, database or custom CLI)' : 'Only the owner can create connections',});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
         }
      }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
 { svelteHTML.createElement("div", { "class":`db-page`,});
   { svelteHTML.createElement("aside", {        "id":`db-side`,"class":`db-side`,"style":viewport.isPhone || database.sidebarCollapsed ? '' : `width:${sideW}px`,});!viewport.isPhone && (database.sidebarCollapsed || hubEmpty);
    if(viewport.isPhone){
      
       { svelteHTML.createElement("div", { "class":`conn-head acc-head`,});
         { svelteHTML.createElement("button", {     "class":`acc-toggle`,"onclick":() => (connOpen = !connOpen),"aria-expanded":connOpen,});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":connOpen ? 'chevronDown' : 'chevronRight',"size":14,}});}
           { svelteHTML.createElement("span", { "class":`conn-head-title`,});  }
          if(database.connections.length > 0){ { svelteHTML.createElement("span", { "class":`acc-count`,});database.connections.length; }}
         }
         { svelteHTML.createElement("div", { "class":`head-btns`,});
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => createSection(null),"aria-label":`New section`,"title":`New section`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});} }
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":!auth.isRoot,"onclick":newConnection,"aria-label":`New connection`,"title":auth.isRoot ? 'New connection' : 'Only the owner can create connections',}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});} }
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":!auth.isRoot,"onclick":() => (connImportOpen = true),"aria-label":`Import connections`,"title":auth.isRoot ? 'Import connections from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster' : 'Only the owner can import connections',}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":13,}});} }
         }
       }
       { svelteHTML.createElement("div", {  "class":`conn-list`,});!connOpen;
        ;__sveltets_2_ensureSnippet(connListBody());
       }

      if(database.selectedConnId){
        
         { svelteHTML.createElement("div", { "class":`conn-head acc-head`,});
           { svelteHTML.createElement("button", {     "class":`acc-toggle`,"onclick":() => (schemaOpen = !schemaOpen),"aria-expanded":schemaOpen,});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":schemaOpen ? 'chevronDown' : 'chevronRight',"size":14,}});}
             { svelteHTML.createElement("span", { "class":`conn-head-title`,});   }
           }
          if(schemaOpen && database.sideTab === 'schema'){
             { svelteHTML.createElement("div", { "class":`head-btns`,});
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => database.refreshSchema(),"title":`Refresh schema`,"aria-label":`Refresh schema`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
             }
          }
         }
         { svelteHTML.createElement("div", {  "class":`side-switch`,});!schemaOpen;
           { const $$_sbaT5C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT5C({ target: __sveltets_2_any(), props: {        "label":`Sidebar view`,"tabs":MOBILE_SIDE_TABS,"value":database.sideTab === 'connections' ? 'schema' : database.sideTab,"onchange":(id) => database.setSideTab(id),}});}
         }
         { svelteHTML.createElement("div", {  "class":`side-body`,});!schemaOpen;
          ;__sveltets_2_ensureSnippet(schemaSideBody());
         }
      }
    }else{
      
       { svelteHTML.createElement("div", { "class":`side-switch`,});
         { const $$_sbaT5C = __sveltets_2_ensureComponent(Tabs); const $$_sbaT5 = new $$_sbaT5C({ target: __sveltets_2_any(), props: {       "label":`Sidebar view`,"tabs":SIDE_TABS,"value":database.sideTab,"onchange":(id) => database.setSideTab(id),trailing:() => { async ()/*Ωignore_positionΩ*/ => {
            if(database.sideTab === 'schema' && database.selectedConnId){
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => database.refreshSchema(),"title":`Refresh schema`,"aria-label":`Refresh schema`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} }
            }
          };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {trailing} = $$_sbaT5.$$prop_def;/*Ωignore_endΩ*/
          
         Tabs}
       }
       { svelteHTML.createElement("div", { "class":`side-body`,});
        if(database.sideTab === 'connections'){
           { svelteHTML.createElement("div", { "class":`conn-list`,});;__sveltets_2_ensureSnippet(connListBody()); }
        } else if (database.selectedConnId){
          ;__sveltets_2_ensureSnippet(schemaSideBody());
        }else{
          
           { svelteHTML.createElement("div", { "class":`side-empty`,});
             { const $$_etatSytpmE6C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE6C({ target: __sveltets_2_any(), props: {      "icon":`file`,"title":`No connection open`,"body":`Open a connection to browse its schema.`,}});}
             { svelteHTML.createElement("div", { "class":`side-empty-actions`,});
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => database.setSideTab('connections'),});  }
               { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":!auth.isRoot,"onclick":newConnection,"title":auth.isRoot ? undefined : 'Only the owner can create connections',});  }
             }
           }
        }
       }
    }
   }

  if(!viewport.isPhone && !database.sidebarCollapsed && !hubEmpty){
     { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {           width:sideW,"storageKey":`db.sideW`,"label":`Resize connections sidebar`,"max":sideMaxW(),"defaultWidth":SIDE_DEFAULT,}});/*Ωignore_startΩ*/() => sideW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
  }

   { svelteHTML.createElement("div", {   "class":`db-main`,});database.isProd;database.isGuarded && !database.isProd;
    if(!hasAnyTab){
      
      
      if(database.connections.length === 0 && database.otherConnections.length === 0 && brokers.clusters.length === 0){
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":viewport.isPhone ? 'panel' : 'page',"icon":`plug`,"title":`No connections yet`,"body":`Add a database (MySQL, PostgreSQL, Redis, MongoDB, ClickHouse), an SSH host or a custom CLI, then open it here to query, browse its schema or get a terminal.`,"actionLabel":auth.isRoot ? 'New connection' : undefined,"actionIcon":`plus`,"onaction":auth.isRoot ? newConnection : undefined,}});
          if(auth.isRoot){
             { svelteHTML.createElement("div", { "class":`empty-alt`,});
               { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":newCluster,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":engineGlyph('kafka'),"size":12,}});}     }
               { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => (connImportOpen = true),}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}     }
             }
          }else{
             { svelteHTML.createElement("p", { "class":`empty-note`,});      }
          }
         EmptyState}
      }else{
        
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"variant":viewport.isPhone ? 'panel' : 'page',"icon":`db`,"title":`Open a connection`,"body":`${hubSummary} Choose one ${viewport.isPhone ? 'above' : 'on the left'} to open it here.`,"actionLabel":database.sidebarCollapsed || database.sideTab !== 'connections' ? 'Show connections' : undefined,"onaction":showConnections,}});
          if(recentConns.length > 0){
             { svelteHTML.createElement("div", {   "class":`recent-conns`,...__sveltets_2_empty({"data-testid":`db-recent-conns`}),});
               { svelteHTML.createElement("p", { "class":`empty-note`,});  }
                 for(let c of __sveltets_2_ensureArray(recentConns)){c.id;
                 { svelteHTML.createElement("button", {     "class":`btn ghost small`,"onclick":() => void database.openConnection(c.id),"title":`Open ${c.name}`,});
                   { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":engineGlyph(c.kind),"size":12,}});} c.name;
                 }
              }
             }
          }
         EmptyState}
      }
    }else{
      
      
       { const $$_div4 = svelteHTML.createElement("div", {          "class":`conn-tabs`,"role":`tablist`,"aria-label":`Open connections`,"tabindex":-1,"onkeydown":onTabKey,});connTabsEl = $$_div4;
           for(let c of __sveltets_2_ensureArray(openConns)){c.id;
          const st = database.connStatus.get(c.id);
          const on = database.activePane === null && database.selectedConnId === c.id;
          const stText = st?.phase === 'connecting' ? 'Connecting…' : st?.phase === 'error' ? (st.error ? `Couldn’t connect: ${st.error}` : 'Couldn’t connect — the server gave no reason') : st?.phase === 'idle' ? 'Not connected yet' : '';
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {         "class":`conn-tab`,"role":`presentation`,"oncontextmenu":(e) => { e.preventDefault(); connMenu(e, c); },});on;isProdConn(c);isGuardedConn(c) && !isProdConn(c);
            
             { svelteHTML.createElement("button", {             "class":`conn-tab-main`,"role":`tab`,"aria-selected":on,"aria-describedby":stText ? `conn-st-${c.id}` : undefined,"tabindex":connTabStop(on, `db:${c.id}`),"onclick":() => database.openConnection(c.id),"title":`${c.name} — right-click to open beside agents`,});
               { svelteHTML.createElement("span", { "class":`conn-tab-glyph ${c.kind}`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":engineGlyph(c.kind),"size":12,}});} }
              if(sectionLeaf(c)){ { svelteHTML.createElement("span", {   "class":`conn-tab-path mono`,"title":`Folder: ${sectionPath(c)}`,});sectionLeaf(c); }}
               { svelteHTML.createElement("span", { "class":`conn-tab-name ellipsis`,});c.name; }
              if(envBadge(c)){ { const $$_egdaBvnE7C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE7C({ target: __sveltets_2_any(), props: {    "env":c.environment,"readOnly":c.read_only,}});}}
             }
            if(stText){ { svelteHTML.createElement("span", {   "id":`conn-st-${c.id}`,"class":`sr-only`,});stText; }}
            if(st?.phase === 'connecting'){
               { svelteHTML.createElement("span", {         "class":`conn-tab-spin spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,"title":database.isWarming(c.id) ? 'Connecting in the background… (right-click to stop)' : 'Connecting…',...__sveltets_2_empty({"data-testid":`conn-tab-connecting`}),}); }
            } else if (st?.phase === 'error'){
               { svelteHTML.createElement("span", {     "class":`conn-tab-dot`,"aria-hidden":`true`,"title":st.error,}); }
            } else if (st?.phase === 'idle'){
               { svelteHTML.createElement("span", {       "class":`conn-tab-idle`,"aria-hidden":`true`,"title":`Not connected yet — opens on click`,...__sveltets_2_empty({"data-testid":`conn-tab-idle`}),}); }
            }
             { svelteHTML.createElement("button", {         "class":`conn-tab-more`,"onclick":(e) => connMenu(e, c),"aria-haspopup":`menu`,"aria-label":`More actions for ${c.name}`,"title":`More actions for ${c.name}`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});}
             }
             { svelteHTML.createElement("button", {         "class":`conn-tab-close`,"onclick":(e) => {
                e.stopPropagation();
                database.closeConnection(c.id);
              },"aria-label":`Close connection tab`,"title":`Close connection tab`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }}
        }
           for(let cl of __sveltets_2_ensureArray(brokers.openClusters)){cl.id;
          const on = database.activePane?.kind === 'kafka' && database.activePane.id === cl.id;
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "class":`conn-tab`,"role":`presentation`,"oncontextmenu":(e) => { e.preventDefault(); clusterMenu(e, cl); },});on;isProdConn(cl);
             { svelteHTML.createElement("button", {           "class":`conn-tab-main`,"role":`tab`,"aria-selected":on,"tabindex":connTabStop(on, `kafka:${cl.id}`),"onclick":() => openCluster(cl),"title":cl.name,});
               { svelteHTML.createElement("span", { "class":`conn-tab-glyph kafka`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":engineGlyph('kafka'),"size":12,}});} }
               { svelteHTML.createElement("span", { "class":`conn-tab-name ellipsis`,});cl.name; }
              if(envBadge(cl)){ { const $$_egdaBvnE7C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE7C({ target: __sveltets_2_any(), props: {    "env":cl.environment,"readOnly":cl.read_only,}});}}
             }
             { svelteHTML.createElement("button", {         "class":`conn-tab-more`,"onclick":(e) => clusterMenu(e, cl),"aria-haspopup":`menu`,"aria-label":`More actions for ${cl.name}`,"title":`More actions for ${cl.name}`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});}
             }
             { svelteHTML.createElement("button", {       "class":`conn-tab-close`,"onclick":(e) => { e.stopPropagation(); closeKafkaTab(cl.id); },"aria-label":`Close cluster tab`,"title":`Close cluster tab`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }}
        }
           for(let s of __sveltets_2_ensureArray(database.sshTabs)){s.connId;
          const on = database.activePane?.kind === 'ssh' && database.activePane.id === s.connId;
           { svelteHTML.createElement("div", {    "class":`conn-tab`,"role":`presentation`,});on;
             { svelteHTML.createElement("button", {           "class":`conn-tab-main`,"role":`tab`,"aria-selected":on,"tabindex":connTabStop(on, `ssh:${s.connId}`),"onclick":() => database.focusSsh(s.connId),"title":s.name,});
               { svelteHTML.createElement("span", { "class":`conn-tab-glyph ${s.kind}`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":engineGlyph(s.kind),"size":12,}});} }
               { svelteHTML.createElement("span", { "class":`conn-tab-name ellipsis`,});s.name; }
             }
             { svelteHTML.createElement("button", {       "class":`conn-tab-close`,"onclick":(e) => { e.stopPropagation(); void closeSshTerminal(s.connId); },"aria-label":`Close terminal tab`,"title":`Close terminal tab`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
        }
       }

      if(database.activePane?.kind === 'kafka'){
        if(activeCluster){
           { const $$_tnuoMyzaL4C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL4C({ target: __sveltets_2_any(), props: {      "lazy":ClusterViewerLazy,"what":`the Kafka cluster viewer`,"props":{ cluster: activeCluster, onEdit: editCluster, onRemove: (c: BrokerCluster) => void deleteCluster(c) },}});}
        }else{
           { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {      "icon":`box`,"title":`Cluster closed`,"body":`This Kafka cluster tab is no longer open.`,}});}
        }
      } else if (database.activePane?.kind === 'ssh'){
        if(activeSsh){
           { svelteHTML.createElement("div", { "class":`term-pane`,});
            activeSsh.sessionId; {
               { const $$_tnuoMyzaL5C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL5C({ target: __sveltets_2_any(), props: {      "lazy":TerminalLazy,"what":`the terminal`,"props":{ sessionId: activeSsh.sessionId, scrollback: PRIMARY_SCROLLBACK },}});}
            }
           }
        }else{
           { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {      "icon":`terminal`,"title":`Session closed`,"body":`This terminal tab is no longer open.`,}});}
        }
      } else if (database.selectedConnId){
      if(database.isGuarded){
         { svelteHTML.createElement("div", {  "class":`guard-banner`,});database.isProd;
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":database.isProd ? 'zap' : 'key',"size":13,}});}
           { svelteHTML.createElement("span", {});
            if(database.isProd){              }else{         }
           }
         }
      }

       { svelteHTML.createElement("div", {  "class":`main-tabs`,});visibleTabs.length > 3;
        
         { svelteHTML.createElement("div", {         "class":`segmented view-switch`,"role":`tablist`,"aria-label":`Workbench view`,"tabindex":-1,"onkeydown":onTabKey,});
             for(let t of __sveltets_2_ensureArray(visibleTabs)){t.id;
             { svelteHTML.createElement("button", {              "class":`mt`,"role":`tab`,"aria-selected":database.mainTab === t.id,"tabindex":database.mainTab === t.id ? 0 : -1,"title":t.label,"onclick":() => database.setMainTab(t.id),});database.mainTab === t.id;
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":t.icon,"size":12,}});}t.label;
             }
          }
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("div", { "class":`conn-status`,});
          if(database.capabilities){
             { svelteHTML.createElement("span", {   "class":`cap-chip mono`,"title":`Engine`,});database.capabilities.engine; }
          }
          if(database.activeConnStatus?.phase === 'connecting'){
             { svelteHTML.createElement("span", {   "class":`conn-state`,"title":`Connecting…`,}); { svelteHTML.createElement("span", {     "class":`conn-tab-spin spinner`,"style":`--spinner-size: 12px`,"aria-hidden":`true`,}); } { svelteHTML.createElement("span", { "class":`lbl`,});  } }
          } else if (database.activeConnStatus?.phase === 'error'){
             { svelteHTML.createElement("span", {   "class":`conn-state err`,"title":database.activeConnStatus.error,});  }
          } else if (database.activeConnStatus?.phase === 'ready'){
             { svelteHTML.createElement("span", {   "class":`conn-state ok`,"title":healthTitle(database.activeConnStatus),});
               { svelteHTML.createElement("span", { "class":`health-dot`,}); }
              if(database.activeConnStatus.serverVersion){
                 { svelteHTML.createElement("span", { "class":`health-ver mono ellipsis`,});database.activeConnStatus.serverVersion; }
              }
              if(database.activeConnStatus.latencyMs != null){
                 { svelteHTML.createElement("span", { "class":`health-lat`,});database.activeConnStatus.latencyMs;  }
              }
             }
          }
          if(['mysql','postgres'].includes(database.connections.find(c=>c.id===database.selectedConnId)?.kind ?? '')){
             { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":()=>changesOpen=true,"title":`Reviewed schema changes for this connection`,"aria-label":`Schema changes`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});} { svelteHTML.createElement("span", { "class":`lbl`,});  }
             }
          }
           { svelteHTML.createElement("button", {         "class":`btn small ghost`,"onclick":() => database.testConnection(),"disabled":database.testing,"title":`Test this connection`,"aria-label":database.testing ? 'Testing connection' : 'Test connection',});
             { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`plug`,"size":12,}});} { svelteHTML.createElement("span", { "class":`lbl`,});database.testing ? 'Testing…' : 'Test'; }
           }
          if(database.testResult){
             { svelteHTML.createElement("span", {    "class":`test-dot`,"title":database.testResult.message,});database.testResult.ok; }
          }
         }
       }

      
       { svelteHTML.createElement("div", {  "class":`main-split`,});database.assistOpen;
         { svelteHTML.createElement("div", { "class":`main-body`,});
          database.accessRevision; {
           { const $$_nosirapmoCnoitcennoC6C = __sveltets_2_ensureComponent(ConnectionComparison); new $$_nosirapmoCnoitcennoC6C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"enabled":database.mainTab === 'query',}});
          if(database.mainTab === 'query'){
             { const $$_rotidEyreuQ7C = __sveltets_2_ensureComponent(QueryEditor); new $$_rotidEyreuQ7C({ target: __sveltets_2_any(), props: {}});}
          } else if (database.mainTab === 'builder'){
             { const $$_tnuoMyzaL7C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL7C({ target: __sveltets_2_any(), props: {    "lazy":QueryBuilderLazy,"what":`the query builder`,}});}
          } else if (database.mainTab === 'structure'){
             { const $$_tnuoMyzaL7C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL7C({ target: __sveltets_2_any(), props: {    "lazy":StructureLazy,"what":`the structure view`,}});}
          } else if (database.mainTab === 'diagram'){
             { const $$_tnuoMyzaL7C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL7C({ target: __sveltets_2_any(), props: {    "lazy":DiagramLazy,"what":`the diagram`,}});}
          }else{
             { const $$_tnuoMyzaL7C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL7C({ target: __sveltets_2_any(), props: {    "lazy":DashboardsLazy,"what":`dashboards`,}});}
          }
           ConnectionComparison}
          }
         }
        if(database.assistOpen){
          
           {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: assistW, min: 300, max: assistMaxW(), invert: true, onChange: setAssistW, text: pxText })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {               "class":`assist-divider`,"role":`separator`,"aria-orientation":`vertical`,"aria-label":`Resize assistant`,"title":`Drag or use ←/→ to resize the assistant`,"onpointerdown":startAssistResize,}); }}
           { svelteHTML.createElement("aside", {   "class":`assist-pane`,"style":`width:${assistW}px`,});
             { const $$_tnuoMyzaL6C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL6C({ target: __sveltets_2_any(), props: {      "lazy":AssistantLazy,"what":`the assistant`,"variant":`panel`,}});}
           }
        }
       }
      }else{
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {           "icon":`db`,"title":`Pick a connection`,"body":`${hubSummary} Choose one on the left to open it here.`,"actionLabel":`Show connections`,"onaction":showConnections,}});}
      }
    }
   }
 }
 PageBody}
 }













if(changesOpen && database.selectedConnId){ { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Database changes`,"width":1040,"onclose":() => changesOpen=false,}}); { const $$_segnahCesabataD1C = __sveltets_2_ensureComponent(DatabaseChanges); new $$_segnahCesabataD1C({ target: __sveltets_2_any(), props: {    "connectionId":database.selectedConnId,"node":database.activeDb,}});} Modal}}

if(accessFor){ { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Access · ${accessFor.name}`,"width":820,"onclose":() => accessFor=null,}}); { const $$_sseccAecruoseR1C = __sveltets_2_ensureComponent(ResourceAccess); new $$_sseccAecruoseR1C({ target: __sveltets_2_any(), props: {    "kind":`connection`,"resourceId":accessFor.id,}});} Modal}}

if(connFormOpen){
   { const $$_mroFnoitcennoC0C = __sveltets_2_ensureComponent(ConnectionForm); new $$_mroFnoitcennoC0C({ target: __sveltets_2_any(), props: {         "existing":editingConn,"kinds":ALL_KINDS,"onclose":() => (connFormOpen = false),"onsaved":onConnSaved,}});}
}

if(clusterFormOpen){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {      "lazy":ClusterFormLazy,"what":`the cluster form`,"quiet":true,"props":{ cluster: editingCluster, onclose: () => (clusterFormOpen = false) },}});}
}

if(sftpFor){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {      "lazy":SftpLazy,"what":`the SFTP browser`,"quiet":true,"props":{ conn: sftpFor, onclose: () => (sftpFor = null) },}});}
}


if(connImportOpen && ws.currentId){
   { const $$_golaiDtropmInoitcennoC0C = __sveltets_2_ensureComponent(ConnectionImportDialog); new $$_golaiDtropmInoitcennoC0C({ target: __sveltets_2_any(), props: {       "wsId":ws.currentId,"onclose":() => (connImportOpen = false),"onimported":() => {
      void database.loadConnections();
    },}});}
}


if(database.importDialogOpen && database.selectedConnId){
  database.importTable; {
     { const $$_golaiDtropmI0C = __sveltets_2_ensureComponent(ImportDialog); new $$_golaiDtropmI0C({ target: __sveltets_2_any(), props: {}});}
  }
}


if(database.exportRequest){
  const req = database.exportRequest;
   { const $$_golaiDtropxE0C = __sveltets_2_ensureComponent(ExportDialog); new $$_golaiDtropxE0C({ target: __sveltets_2_any(), props: {                   "statement":req.statement,"connectionId":req.connId,"node":req.node,"canExport":resourceAccess.can('connection', req.connId, 'db_export', 'database', 'view', databaseAccessChild(req.node ?? undefined)),"initialFormat":req.format,"initialMaxRows":req.maxRows,"requestedBy":req.agentLabel,"ondone":(r) => {
      req.done({ exported: true, path: r.local_path, rows: r.rows, bytes: r.bytes });
    },"onclose":() => {
      if (database.exportRequest === req) database.exportRequest = null;
      req.done({ exported: false });
    },}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DatabasePage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DatabasePage__SvelteComponent_ = ReturnType<typeof DatabasePage__SvelteComponent_>;
/*Ωignore_endΩ*/export default DatabasePage__SvelteComponent_;