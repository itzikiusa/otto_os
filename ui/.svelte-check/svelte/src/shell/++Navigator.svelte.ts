///<reference types="svelte" />
;
import LazyMount from '../lib/components/LazyMount.svelte';
import { ShareModalChunk } from './overlays.svelte';
import { rowMenu } from '../lib/rowMenu';
import Skeleton from '../lib/components/Skeleton.svelte';
import { focusOnMount } from '../lib/focusOnMount';
import { toastError } from '../lib/toastError';
import { loadErrorText } from '../lib/loadError';
import { closeConfirm } from './closeConfirm';
// Expanded 240px navigator: modules in foldable macOS source-list sections
// (Agents with its nested session lists in Work), workspaces section,
// user/settings at the bottom.
import { navBadge } from '../lib/navBadge';
import Icon from '../lib/components/Icon.svelte';
import NotificationBell from './NotificationBell.svelte';
import StatusDot from '../lib/components/StatusDot.svelte';
import ProviderIcon, { hasProviderIcon } from '../lib/components/ProviderIcon.svelte';
import { router } from '../lib/router.svelte';
import { navPending } from '../lib/navPending.svelte';
import { ui } from '../lib/stores/ui.svelte';
import { RAIL_MIN, RAIL_MAX } from '../lib/stores/ui.svelte';
import { paneResizer, pxWide, RESIZE_TITLE } from '../lib/paneResizer';
import { startMouseDrag } from '../lib/dragCursor';
import { ws, SCRATCH_WORKSPACE_ID } from '../lib/stores/workspace.svelte';
import { auth } from '../lib/stores/auth.svelte';
import { plugins } from '../lib/stores/plugins.svelte';
import { activity } from '../lib/stores/activity.svelte';
import { proof } from '../lib/stores/proof.svelte';
import ProofStatusChip from '../lib/components/ProofStatusChip.svelte';
import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
import { sidePane, splitMenuItems, navClick, SPLIT_HINT } from '../lib/stores/sidePane.svelte';
import { popoutItems } from '../lib/popoutMenu';
import { sessionOrder, applyOrder, applyTokenOrder } from '../lib/stores/sessionOrder.svelte';
import { sessionUsage } from '../lib/stores/sessionUsage.svelte';
import { pollWhileVisible } from '../lib/poll';
import { sessionUsageLabel } from '../lib/sessionUsage';
import { viewport } from '../lib/stores/viewport.svelte';
import { confirmer } from '../lib/confirm.svelte';
import { toasts } from '../lib/toast.svelte';
import { copyText } from '../lib/clipboard';
import type { WorkspaceWithRole } from '../lib/api/types';
import { tick, untrack } from 'svelte';
import {
    FAVORITES_ID,
    activeNavId,
    availableModules,
    moveAmong,
    moveWithinGroup,
    reorderAmong,
    resolveGroupOrder,
    resolveOrder,
    sidebarSections, agentsLast,
    visibleOrder,
    type SidebarModule,
    type SidebarPluginEntry,
    type SidebarSection,
  } from '../lib/sidebar';
import type { Session } from '../lib/api/types';
import { events } from '../lib/events.svelte';
import { sessionState, type SessionStateInfo } from '../lib/status';
function $$render() {
 const sideMark/*Ωignore_positionΩ*/ = (id: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(sidePane.active && sidePane.key === id){
     { svelteHTML.createElement("span", {         "class":`side-mark`,"role":`img`,"title":`Open in the side pane`,"aria-label":`Open in the side pane`,...__sveltets_2_empty({"data-testid":`side-mark-${id}`}),});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`columns`,"size":12,}});}
     }
  }
};return __sveltets_2_any(0)}; const simpleRow/*Ωignore_positionΩ*/ = (m: SidebarModule)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                     "class":`nav-item`,"aria-current":isActive(m.id) ? 'page' : undefined,...__sveltets_2_empty({"data-nav-id":m.id}),"aria-busy":navPending.id === m.id || undefined,"title":sidePane.supported ? SPLIT_HINT : undefined,"onclick":(e) => navClick(e, m.id, m.label),"oncontextmenu":(e) => moduleMenu(e, m),...favRowProps(m.id),});isActive(m.id);dragOverId === m.id && dropSide === 'before';dragOverId === m.id && dropSide === 'after';dragId === m.id;
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":m.icon,"size":14,}});}
     { svelteHTML.createElement("span", { "class":`grow`,});m.label; }
    ;__sveltets_2_ensureSnippet(sideMark(m.id));
    if(navBadge(m.id)){
      const b = navBadge(m.id)!;
       { svelteHTML.createElement("span", { "class":`sr-only`,});b.spoken; } { svelteHTML.createElement("span", {       "class":`count-chip ${b.tone}`,"aria-hidden":`true`,"title":b.title,...__sveltets_2_empty({"data-testid":m.id === 'assistant' && b.tone === 'needs' ? 'assistant-needs-badge' : undefined}),});b.count; }
    }
   }}
};return __sveltets_2_any(0)}; const editRow/*Ωignore_positionΩ*/ = (m: SidebarModule, first: boolean, last: boolean)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const fav = isFav(m.id);
  
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                            "role":`presentation`,"class":`edit-row`,"draggable":true,"ondragstart":(e) => onDragStart(e, m.id),"ondragover":(e) => onDragOver(e, m.id),"ondragleave":() => onDragLeave(m.id),"ondrop":(e) => onDrop(e, m.id),"ondragend":onDragEnd,"oncontextmenu":(e) => ctxMenu.show(e, favoriteMenuItems(m)),"onkeydown":(e) => {
      // ⌥↑ / ⌥↓ from any control in the row (the arrows show on hover /
      // focus, so the label keeps its room).
      if (!e.altKey || e.metaKey || e.ctrlKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
      if ((e.key === 'ArrowUp' && first) || (e.key === 'ArrowDown' && last)) return;
      e.preventDefault();
      move(m, e.key === 'ArrowUp' ? -1 : 1);
    },...__sveltets_2_empty({"data-testid":`sidebar-edit-row-${m.id}`}),});isHidden(m.id);dragId === m.id;dragOverId === m.id && dropSide === 'before';dragOverId === m.id && dropSide === 'after';
     { svelteHTML.createElement("span", {     "class":`grip`,"title":`Drag to reorder`,"aria-hidden":`true`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`grip`,"size":14,}});} }
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":m.icon,"size":14,}});}
     { svelteHTML.createElement("span", { "class":`grow ellipsis`,});m.label; }
     { svelteHTML.createElement("span", { "class":`edit-actions`,});
       { svelteHTML.createElement("button", {              "class":`row-action star-toggle`,"onclick":() => toggleFavorite(m.id),"title":fav ? 'Remove from Favorites' : 'Add to Favorites',"aria-label":`Favorite ${m.label}`,"aria-pressed":fav,...__sveltets_2_empty({"data-testid":`sidebar-fav-${m.id}`}),});fav;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":13,}});}
       }
       { svelteHTML.createElement("button", {             "class":`row-action mv`,"onclick":() => move(m, -1),"disabled":first,"title":`Move ${m.label} up`,"aria-label":`Move ${m.label} up`,"aria-keyshortcuts":`Alt+ArrowUp`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}
       }
       { svelteHTML.createElement("button", {             "class":`row-action mv`,"onclick":() => move(m, 1),"disabled":last,"title":`Move ${m.label} down`,"aria-label":`Move ${m.label} down`,"aria-keyshortcuts":`Alt+ArrowDown`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}
       }
       { svelteHTML.createElement("button", {           "class":`row-action`,"onclick":() => ui.toggleSidebarHidden(m.id),"title":isHidden(m.id) ? `Show ${m.label}` : `Hide ${m.label}`,"aria-label":isHidden(m.id) ? `Show ${m.label}` : `Hide ${m.label}`,...__sveltets_2_empty({"data-testid":`sidebar-hide-${m.id}`}),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":isHidden(m.id) ? 'eyeOff' : 'eye',"size":13,}});}
       }
     }
   }}
};return __sveltets_2_any(0)}; const agentsBlock/*Ωignore_positionΩ*/ = (m: SidebarModule)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", {      "class":`nav-item-row agents-row`,});dragOverId === m.id && dropSide === 'before';dragOverId === m.id && dropSide === 'after';dragId === m.id;
     {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                  "class":`nav-item`,"aria-current":router.module === 'agents' || router.module === '' ? 'page' : undefined,...__sveltets_2_empty({"data-nav-id":m.id}),"aria-busy":navPending.id === m.id || undefined,"title":sidePane.supported ? SPLIT_HINT : undefined,"onclick":(e) => navClick(e, 'agents', m.label),"oncontextmenu":(e) => ctxMenu.show(e, [
        { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
        { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
        { label: 'Add workspace…', icon: 'folder', action: () => (ui.newWorkspaceOpen = true) },
        { separator: true },
        ...splitItems(m),
        ...favoriteMenuItems(m),
        { separator: true },
        customizeItem(),
      ]),...favRowProps(m.id),});router.module === 'agents' || router.module === '';
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`grow`,});  }
      ;__sveltets_2_ensureSnippet(sideMark('agents'));
      if(ws.workingCount > 0){
         { svelteHTML.createElement("span", { "class":`count-chip working`,});ws.workingCount; }
      }
     }}
    if(selectable.some((s) => ws.canEditSession(s))){
       { svelteHTML.createElement("button", {              "class":`icon-btn twisty sel-toggle`,"onclick":() => setAgentSelMode(!agentSelMode),"title":agentSelMode ? 'Done selecting' : 'Select sessions to archive or delete',"aria-label":`Select sessions`,"aria-pressed":agentSelMode,...__sveltets_2_empty({"data-testid":`agents-select-toggle`}),});agentSelMode;
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":agentSelMode ? 'check' : 'square',"size":12,}});}
       }
    }
    if(fAgents.length > 1){
       { svelteHTML.createElement("button", {            "class":`icon-btn twisty sort-toggle`,"onclick":openSortMenu,"title":sessionOrder.mode === 'manual' ? 'Manual order — click to change' : tokensSort ? 'Sorted by tokens — click to change' : 'Sort sessions',"aria-label":`Sort sessions`,...__sveltets_2_empty({"data-testid":`agents-sort-toggle`}),});sessionOrder.mode === 'manual' || tokensSort;
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":sessionOrder.mode === 'manual' ? 'grip' : 'arrowDown',"size":12,}});}
       }
    }
     { svelteHTML.createElement("button", {            "class":`icon-btn twisty all-ws-toggle`,"onclick":() => ws.setAllWorkspaces(!ws.allWorkspaces),"title":ws.allWorkspaces
        ? 'Showing sessions from all workspaces — click for current only'
        : 'Show sessions from all workspaces',"aria-label":`Toggle all-workspaces session list`,"aria-pressed":ws.allWorkspaces,});ws.allWorkspaces;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});}
     }
     { svelteHTML.createElement("button", {           "class":`icon-btn twisty`,"onclick":() => (agentsOpen = !agentsOpen),"aria-label":`Toggle session list`,"aria-expanded":agentsOpen,"title":agentsOpen ? 'Hide sessions' : 'Show sessions',});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":agentsOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
     }
   }

  
   { svelteHTML.createElement("div", { "class":`agents-sub`,});
    
    if(ws.current){
      const cur = ws.current;
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                "class":`ws-chip`,"onclick":openWsSwitcher,"oncontextmenu":(e) => ctxMenu.show(e, workspaceMenuItems(cur)),"aria-haspopup":`menu`,"title":`${ws.current.name}\n${ws.current.root_path}\nClick to switch workspace`,"aria-label":`Current workspace: ${ws.current.name}. Switch workspace`,...__sveltets_2_empty({"data-testid":`agents-current-ws`}),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}
         { svelteHTML.createElement("span", { "class":`ellipsis`,});ws.current.name; }
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
       }}
    }
    if(q ? fAgents.length > 0 : agentsOpen){
       { svelteHTML.createElement("div", {   "class":`nested`,...__sveltets_2_empty({"data-testid":`agents-list`}),});
        if(agentSelMode){
           { svelteHTML.createElement("div", {   "class":`arch-tools`,...__sveltets_2_empty({"data-testid":`agents-select-tools`}),});
             { svelteHTML.createElement("label", {   "class":`arch-all`,"title":`Select all sessions`,});
               { svelteHTML.createElement("input", {          "type":`checkbox`,"aria-label":`Select all sessions`,"checked":agentSelIds.length > 0 && agentSelIds.length === selectable.length,"indeterminate":agentSelIds.length > 0 && agentSelIds.length < selectable.length,"onchange":agentSelectAll,});}
               { svelteHTML.createElement("span", {});agentSelIds.length > 0 ? `${agentSelIds.length} selected` : 'Select all'; }
             }
             { svelteHTML.createElement("button", {           "class":`row-action arch-del-sel`,"disabled":agentSelIds.length === 0,"title":`Archive selected sessions`,"aria-label":`Archive selected sessions`,...__sveltets_2_empty({"data-testid":`agents-archive-selected`}),"onclick":() => void archiveSelectedAgents(),});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":12,}});} { svelteHTML.createElement("span", {});  }
             }
             { svelteHTML.createElement("button", {           "class":`row-action danger arch-del-sel`,"disabled":agentSelIds.length === 0,"title":`Delete selected sessions`,"aria-label":`Delete selected sessions`,...__sveltets_2_empty({"data-testid":`agents-delete-selected`}),"onclick":() => void deleteSelectedAgents(),});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} { svelteHTML.createElement("span", {});  }
             }
           }
        }
           for(let s of __sveltets_2_ensureArray(fAgents)){s.id;
          ;__sveltets_2_ensureSnippet(sessionRow(s, undefined, true));
        }
           { svelteHTML.createElement("div", { "class":`nested-empty`,});       }
        
       }
    }

    
    if(q ? fScratch.length > 0 : agentsOpen && (ws.scratchSessions.length > 0 || ws.current === null)){
      
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {            "role":`presentation`,"class":`ws-group-label`,"title":`Sessions not tied to any workspace`,...__sveltets_2_empty({"data-testid":`scratch-group`}),"oncontextmenu":(e) => ctxMenu.show(e, [
          { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
        ]),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`home`,"size":13,}});}
         { svelteHTML.createElement("span", { "class":`ellipsis`,});  }
       }}
       { svelteHTML.createElement("div", { "class":`nested`,});
           for(let s of __sveltets_2_ensureArray(fScratch)){s.id;
          ;__sveltets_2_ensureSnippet(sessionRow(s));
        }
           { svelteHTML.createElement("div", { "class":`nested-empty`,});       }
        
       }
    }

    
    if(ws.allWorkspaces && (agentsOpen || q)){
         for(let g of __sveltets_2_ensureArray(ws.otherWsGroups)){g.ws.id;
        const rows = g.sessions.filter(matches);
        if(rows.length > 0){
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {            "role":`presentation`,"class":`ws-group-label`,"title":`Sessions in workspace “${g.ws.name}” — right-click for actions`,...__sveltets_2_empty({"data-testid":`ws-group-${g.ws.id}`}),"oncontextmenu":(e) => ctxMenu.show(e, workspaceMenuItems(g.ws)),});
             { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}
             { svelteHTML.createElement("span", { "class":`ellipsis`,});g.ws.name; }
           }}
           { svelteHTML.createElement("div", { "class":`nested`,});
               for(let s of __sveltets_2_ensureArray(rows)){s.id;
              ;__sveltets_2_ensureSnippet(sessionRow(s, g.ws.id));
            }
           }
        }
      }
    }

    if(q ? fTelegram.length > 0 : ws.telegramSessions.length > 0){
       { svelteHTML.createElement("div", { "class":`nav-item-row`,});
         { svelteHTML.createElement("button", {     "class":`nav-item`,"onclick":() => (telegramOpen = !telegramOpen),"aria-expanded":telegramOpen,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`grow`,});  }
           { svelteHTML.createElement("span", { "class":`count-chip`,});ws.telegramSessions.length; }
         }
         { svelteHTML.createElement("button", {           "class":`icon-btn twisty`,"onclick":() => (telegramOpen = !telegramOpen),"aria-label":`Toggle Telegram list`,"aria-expanded":telegramOpen,"title":telegramOpen ? 'Hide Telegram sessions' : 'Show Telegram sessions',});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":telegramOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
         }
       }
      if(telegramOpen || q){
         { svelteHTML.createElement("div", { "class":`nested`,});
             for(let s of __sveltets_2_ensureArray(visTelegram)){s.id;
            ;__sveltets_2_ensureSnippet(sessionRow(s));
          }
             { svelteHTML.createElement("div", { "class":`nested-empty`,});  }
          
          if(!q && fTelegram.length > CHANNEL_CAP){
             { svelteHTML.createElement("button", {   "class":`show-more`,"onclick":() => (telegramShowAll = !telegramShowAll),});
              telegramShowAll ? 'Show less' : `Show ${fTelegram.length - CHANNEL_CAP} more`;
             }
          }
         }
      }
    }

    if(q ? fSlack.length > 0 : ws.slackSessions.length > 0){
       { svelteHTML.createElement("div", { "class":`nav-item-row`,});
         { svelteHTML.createElement("button", {     "class":`nav-item`,"onclick":() => (slackOpen = !slackOpen),"aria-expanded":slackOpen,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`slack`,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`grow`,});  }
           { svelteHTML.createElement("span", { "class":`count-chip`,});ws.slackSessions.length; }
         }
         { svelteHTML.createElement("button", {           "class":`icon-btn twisty`,"onclick":() => (slackOpen = !slackOpen),"aria-label":`Toggle Slack list`,"aria-expanded":slackOpen,"title":slackOpen ? 'Hide Slack sessions' : 'Show Slack sessions',});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":slackOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
         }
       }
      if(slackOpen || q){
         { svelteHTML.createElement("div", { "class":`nested`,});
             for(let s of __sveltets_2_ensureArray(visSlack)){s.id;
            ;__sveltets_2_ensureSnippet(sessionRow(s));
          }
             { svelteHTML.createElement("div", { "class":`nested-empty`,});  }
          
          if(!q && fSlack.length > CHANNEL_CAP){
             { svelteHTML.createElement("button", {   "class":`show-more`,"onclick":() => (slackShowAll = !slackShowAll),});
              slackShowAll ? 'Show less' : `Show ${fSlack.length - CHANNEL_CAP} more`;
             }
          }
         }
      }
    }

    
    if(ws.hasArchived || ws.archivedSessions.length > 0){
       { svelteHTML.createElement("button", {       "class":`nav-item nested-item arch-toggle`,"aria-expanded":archivedOpen,...__sveltets_2_empty({"data-testid":`archived-toggle`}),"onclick":toggleArchived,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":13,}});}
         { svelteHTML.createElement("span", { "class":`grow`,});  }
        if(ws.archivedLoaded){
           { svelteHTML.createElement("span", { "class":`count-chip`,});ws.archivedSessions.length;ws.archivedHasMore ? '+' : ''; }
        }
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":archivedOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
       }
      if(archivedOpen){
         { svelteHTML.createElement("div", { "class":`nested`,});
          if(ws.myRole !== 'viewer' && ws.archivedSessions.length > 1){
             { svelteHTML.createElement("div", { "class":`arch-tools`,});
               { svelteHTML.createElement("label", {   "class":`arch-all`,"title":`Select all archived sessions`,});
                 { svelteHTML.createElement("input", {          "type":`checkbox`,"aria-label":`Select all archived sessions`,"checked":archSelCount > 0 && archSelCount === ws.archivedSessions.length,"indeterminate":archSelCount > 0 && archSelCount < ws.archivedSessions.length,"onchange":archSelectAll,});}
                 { svelteHTML.createElement("span", {});archSelCount > 0 ? `${archSelCount} selected` : 'Select all'; }
               }
               { svelteHTML.createElement("button", {           "class":`row-action danger arch-del-sel`,"disabled":archSelCount === 0,"title":`Delete selected sessions`,"aria-label":`Delete selected sessions`,...__sveltets_2_empty({"data-testid":`archived-delete-selected`}),"onclick":() => void deleteSelectedArchived(),});
                 { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} { svelteHTML.createElement("span", {}); archSelCount > 0 ? ` (${archSelCount})` : ''; }
               }
             }
          }
             for(let s of __sveltets_2_ensureArray(ws.archivedSessions)){s.id;
             { svelteHTML.createElement("div", {  "class":`nested-row`,});archSel.has(s.id);
              if(ws.canEditSession(s)){
                 { svelteHTML.createElement("input", {          "type":`checkbox`,"class":`arch-check`,"checked":archSel.has(s.id),"onchange":() => toggleArchSel(s.id),"aria-label":`Select ${s.title}`,});}
              }
              
               {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {          "role":`presentation`,"class":`nav-item nested-item archived`,"title":s.title,"oncontextmenu":(e) => ctxMenu.show(e, [
                  ...(ws.canEditSession(s) ? [
                    { label: 'Unarchive', icon: 'refresh', action: () => ws.unarchiveSession(s.id) },
                    { label: 'Delete…', icon: 'trash', danger: true as const, action: () => void deleteSession(s.id) },
                  ] : []),
                  { separator: true },
                  { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
                  { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
                ]),});
                 { const $$_toDsutatS4C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS4C({ target: __sveltets_2_any(), props: {  "status":`exited`,}});}
                 { svelteHTML.createElement("span", { "class":`grow ellipsis`,});s.title; }
                if(hasProviderIcon(s.provider)){
                   { svelteHTML.createElement("span", {   "class":`provider-ico`,"title":s.provider,}); { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":s.provider,"size":13,}});} }
                }else{
                   { svelteHTML.createElement("span", { "class":`provider`,});s.provider; }
                }
               }}
              if(ws.canEditSession(s)){
                 { svelteHTML.createElement("button", {       "class":`row-action`,"title":`Restore session`,"aria-label":`Restore session`,"onclick":() => ws.unarchiveSession(s.id),});
                   { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
                 }
                 { svelteHTML.createElement("button", {       "class":`row-action danger`,"title":`Delete session`,"aria-label":`Delete session`,"onclick":() => void deleteSession(s.id),});
                   { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
                 }
              }
             }
          }
          if(archivedError){
             { svelteHTML.createElement("div", {   "class":`arch-state`,"role":`alert`,});
               { svelteHTML.createElement("span", { "class":`grow`,});    }
               { svelteHTML.createElement("button", {   "class":`show-more`,"onclick":() => void loadArchivedPage(ws.archivedLoaded),});  }
             }
          } else if (ws.archivedLoading){
             { svelteHTML.createElement("div", { "class":`arch-state`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":2,"height":22,"label":`archived sessions`,}});} }
          } else if (ws.archivedLoaded && ws.archivedSessions.length === 0){
             { svelteHTML.createElement("div", { "class":`arch-state`,});   }
          } else if (ws.archivedHasMore){
             { svelteHTML.createElement("button", {     "class":`show-more`,...__sveltets_2_empty({"data-testid":`archived-load-more`}),"onclick":() => void loadArchivedPage(true),});  }
          }
         }
      }
    }
   }
};return __sveltets_2_any(0)}; const sessionRow/*Ωignore_positionΩ*/ = (s: Session, otherWs?: string, reorderable = false)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const status = ws.statusMap[s.id] ?? s.status;
  const sum = activity.summary(s.id);
  const proofRow = proof.summaryFor('session', s.id);
  const needsYou = ws.needsYou[s.id] === true;
  const st = sessionState(s, status, needsYou, { stale: staleEvents });
  const resumable = st.resumable;
  const dnd = rowsDraggable && reorderable;
  
   { svelteHTML.createElement("div", {                    "role":`presentation`,"class":`nested-row`,"draggable":dnd,"ondragstart":dnd ? (e) => onRowDragStart(e, s.id) : undefined,"ondragover":dnd ? (e) => onRowDragOver(e, s.id) : undefined,"ondragleave":() => { if (rowDragOverId === s.id) rowDragOverId = null; },"ondrop":dnd ? (e) => onRowDrop(e, s.id) : undefined,"ondragend":onRowDragEnd,});st.key === 'needs-you';agentSelMode && agentSel.has(s.id);rowDragOverId === s.id;
    if(agentSelMode && !otherWs){
       { svelteHTML.createElement("input", {          "type":`checkbox`,"class":`arch-check`,"checked":agentSel.has(s.id),"onchange":() => toggleAgentSel(s.id),"aria-label":`Select ${s.title}`,});}
    }
    if(renamingId === s.id){
       {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {             "dir":`auto`,"aria-label":`Rename session`,"class":`nav-rename`,"bind:value":draft,"onblur":commitRename,"onkeydown":(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renamingId = null;
        },});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
    }else{
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                  "class":`nav-item nested-item`,"aria-current":!otherWs && router.module === 'agents' && ws.activeSessionId === s.id ? 'true' : undefined,"onclick":() => (otherWs ? void ws.openInWorkspace(otherWs, s.id) : openSession(s.id)),"ondblclick":() => startRename(s.id, s.title),"oncontextmenu":(e) => ctxMenu.show(e, [
          // Same order as the tab and pane menus: Rename · Share… · Open in
          // new window, then Restart · Archive · Delete, then New session.
          ...(ws.canEditSession(s) ? [{ label: 'Rename', icon: 'edit', action: () => startRename(s.id, s.title) }] : []),
          ...(otherWs ? [] : [{ label: 'Share…', icon: 'share', action: () => (shareSessionId = s.id) }]),
          ...(otherWs ? [] : popoutItems(`agents/${s.id}`, s.title)),
          ...(reorderable && fAgents.length > 1
            ? [{ separator: true }, { label: 'Move to top', icon: 'arrowUp', action: () => { const ids = fAgents.map((x) => x.id); sessionOrder.dragTo(ids, s.id, ids[0]); } }]
            : []),
          { separator: true },
          ...(ws.canEditSession(s) ? [
            // In-progress agent only: respawn a stuck PTY (provider resume when
            // possible). Idle/exited/reconnectable sessions have their own paths.
            ...(s.kind === 'agent' && (status === 'running' || status === 'working')
              ? [{ label: 'Restart session', icon: 'refresh', action: () => void restartAgent(s.id) }]
              : []),
            { label: 'Archive', icon: 'archive', action: () => void ws.requestArchive(s.id).catch((e: unknown) => toastError('Couldn’t archive the session', e)) },
            { label: 'Delete…', icon: 'trash', danger: true as const, action: () => void deleteSession(s.id) },
          ] : []),
          { separator: true },
          { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
          { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
        ]),"title":rowTip(s.title, st, sum, sessionUsageLabel(usageRows[s.id])),...__sveltets_2_empty({"data-state":st.key}),});!otherWs && router.module === 'agents' && ws.activeSessionId === s.id;resumable;
        
        if(resumable){
           { svelteHTML.createElement("span", {       "class":`susp-dot`,"role":`img`,"aria-label":st.label,"title":st.hint,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
           }
        }else{
           { const $$_toDsutatS2C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS2C({ target: __sveltets_2_any(), props: {  "state":st,}});}
        }
         { svelteHTML.createElement("span", { "class":`grow ellipsis`,});s.title; }
        if(st.key === 'needs-you'){
           { svelteHTML.createElement("span", {       "class":`needs-you-dot`,"role":`img`,"title":`Waiting on you`,"aria-label":`Needs you`,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":12,}});}
           }
        }
        if((sum && sum.total > 0) || proofRow){
           { svelteHTML.createElement("span", { "class":`row-secondary`,});
            if(sum && sum.total > 0){
               { svelteHTML.createElement("span", {       "class":`task-chip`,"title":sum.in_progress ? `Now: ${sum.in_progress}` : `${sum.done}/${sum.total} tasks done`,});sum.done === sum.total;sum.in_progress != null;sum.done; sum.total; }
            }
            if(proofRow){
                { const $$_pihCsutatSfoorP3C = __sveltets_2_ensureComponent(ProofStatusChip); new $$_pihCsutatSfoorP3C({ target: __sveltets_2_any(), props: {    "status":proofRow.status,"risk":proofRow.risk_score,"compact":true,}});}
            }
           }
        }
        if(hasProviderIcon(s.provider)){
           { svelteHTML.createElement("span", {   "class":`provider-ico`,"title":s.provider,}); { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {    "provider":s.provider,"size":13,}});} }
        }else{
           { svelteHTML.createElement("span", { "class":`provider`,});s.provider; }
        }
       }}
      if(ws.canEditSession(s)){
         { svelteHTML.createElement("button", {         "class":`row-action`,"title":ws.closeTabTitle(s.id, 'session'),"aria-label":`Close session`,"onclick":() => void ws.requestCloseTab(s.id),});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
      }
    }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Load the per-session task roll-up for the current workspace (sidebar chips);
  // it then stays fresh from the events WS (tasks_updated / trail_appended).
  $effect(() => {
    const w = ws.currentId;
    if (w) void activity.loadSummary(w);
  });

  // Load the per-work-item proof roll-up so each session row can show an inline
  // proof chip; kept fresh from the events WS (proof_pack_updated). Scoped to
  // the sessions this sidebar lists: only their packs are read (never every
  // pack in the workspace), and only sessions not asked for before.
  $effect(() => {
    const w = ws.currentId;
    const keys = ws.sessions.map((s) => `session:${s.id}`);
    if (w) void proof.loadSummary(w, keys);
  });

  // One session vocabulary (lib/status.ts): a row's dot, tooltip and resume
  // affordance all come from `sessionState`. While the events socket is down
  // the live claims are stale — "Reconnecting…", no pulse (patterns.md §1).
  const staleEvents = $derived(events.state !== 'connected');
  /** Session-row tooltip: a long title made one very wide native tooltip that
   *  WKWebView pinned against the window edge and clipped over the page. Keep
   *  it short and put the state + secondary signals on their own lines. */
  function rowTip(title: string, st: SessionStateInfo, tasks: { done: number; total: number; in_progress?: string | null } | null, usage?: string | null): string {
    const t = title.length > 80 ? `${title.slice(0, 79).trimEnd()}…` : title;
    const lines = [t, st.hint ?? st.label];
    if (tasks && tasks.total > 0) {
      lines.push(tasks.in_progress ? `Now: ${tasks.in_progress} · ${tasks.done}/${tasks.total} tasks` : `${tasks.done}/${tasks.total} tasks done`);
    }
    if (usage) lines.push(`${usage} (30 days)`);
    if (!st.resumable) lines.push('Double-click to rename');
    return lines.join('\n');
  }

  let agentsOpen = $state(true);
  // Channel groups (ticket/chat sessions) start collapsed — at ticketing volume
  // they can number in the dozens, so the header + count is shown by default and
  // the user expands on demand.
  let telegramOpen = $state(false);
  let slackOpen = $state(false);
  let archivedOpen = $state(false);
  // Multi-select in the Archived list: checked ids → one "Delete selected"
  // confirm instead of N single-row deletes.
  let archSel = $state<Set<string>>(new Set());
  // Select mode on the Agents list: header toggle → per-row checkboxes → one
  // Archive / Delete for the whole selection (delete confirms once).
  let agentSelMode = $state(false);
  let agentSel = $state<Set<string>>(new Set());
  function toggleAgentSel(id: string): void {
    const next = new Set(agentSel);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    agentSel = next;
  }
  function agentSelectAll(): void {
    agentSel = agentSelIds.length === selectable.length ? new Set() : new Set(selectable.map((s) => s.id));
  }
  function setAgentSelMode(on: boolean): void {
    agentSelMode = on;
    if (!on) agentSel = new Set();
  }
  /** The ⌘K palette's close confirm (closeConfirm.ts): titles + mid-turn warning. */
  function askClose(ids: string[], permanent: boolean, from: Session[]): Promise<boolean> {
    const titles = ids.map((id) => from.find((s) => s.id === id)?.title ?? id);
    const working = ids.filter((id) => ws.isAgentMidTurn(id)).length;
    const { message, ...opts } = closeConfirm({ titles, working, permanent });
    return confirmer.ask(message, opts);
  }
  async function archiveSelectedAgents(): Promise<void> {
    const ids = agentSelIds;
    if (ids.length === 0) return;
    // Archiving stops a session: ask first when any of them is mid-turn.
    if (ids.some((id) => ws.isAgentMidTurn(id)) && !(await askClose(ids, false, selectable))) return;
    await ws.archiveSessions(ids);
    setAgentSelMode(false);
  }
  async function deleteSelectedAgents(): Promise<void> {
    const ids = agentSelIds;
    if (ids.length === 0 || !(await askClose(ids, true, selectable))) return;
    await ws.killSessions(ids);
    setAgentSelMode(false);
  }
  const archivedIds = $derived(new Set(ws.archivedSessions.map((s) => s.id)));
  const archSelCount = $derived([...archSel].filter((id) => archivedIds.has(id)).length);
  /** Expand / fold the Archived section; the first open loads page one. */
  function toggleArchived(): void {
    archivedOpen = !archivedOpen;
    if (archivedOpen && !ws.archivedLoaded) void loadArchivedPage(false);
  }
  let archivedError = $state<string | null>(null);
  async function loadArchivedPage(more: boolean): Promise<void> {
    archivedError = null;
    try {
      await ws.loadArchived(more);
    } catch (e) {
      archivedError = loadErrorText(e);
    }
  }
  function toggleArchSel(id: string): void {
    const next = new Set(archSel);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    archSel = next;
  }
  function archSelectAll(): void {
    archSel = archSelCount === ws.archivedSessions.length ? new Set() : new Set(ws.archivedSessions.map((s) => s.id));
  }
  async function deleteSelectedArchived(): Promise<void> {
    const ids = ws.archivedSessions.filter((s) => archSel.has(s.id)).map((s) => s.id);
    if (ids.length === 0 || !(await askClose(ids, true, ws.archivedSessions))) return;
    await ws.killSessions(ids);
    archSel = new Set();
  }
  let renamingId: string | null = $state(null);
  let draft = $state('');
  /** The session whose Share sheet is open (same sheet as the tab / pane menus). */
  let shareSessionId = $state<string | null>(null);

  // When a channel group is expanded, show only the most recent N sessions with
  // a "show more" expander, so a busy day's worth of tickets doesn't flood the
  // sidebar. Bypassed while searching (the user is actively filtering).
  const CHANNEL_CAP = 20;
  let telegramShowAll = $state(false);
  let slackShowAll = $state(false);

  // Session search: filters every group by title (case-insensitive), plus an
  // optional "Needs you" filter that narrows to sessions blocked on the operator.
  let sessionQuery = $state('');
  const q = $derived(sessionQuery.trim().toLowerCase());
  const matches = (s: Session): boolean => {
    if (ws.needsYouFilter && ws.needsYou[s.id] !== true) return false;
    return q === '' || s.title.toLowerCase().includes(q);
  };
  // ── Manual sidebar order (C3b) ───────────────────────────────────────────
  // "Recent" is the daemon's order, rendered unchanged. "Manual" applies the
  // persisted id list, with sessions it has never seen on TOP by recency.
  $effect(() => {
    sessionOrder.load(ws.currentId ?? SCRATCH_WORKSPACE_ID);
  });
  // Per-session tokens (A5): the 30-day workspace rollup feeds the "Tokens"
  // sort and the row tooltip. Read once a minute at most (store-throttled).
  const canUsage = $derived(auth.can('usage', 'view'));
  const usageWs = $derived(ws.currentId ?? SCRATCH_WORKSPACE_ID);
  // Polled only while the Tokens sort is on (the order must stay current);
  // otherwise the rollup only feeds row tooltips, so one read per workspace
  // switch is enough (F4).
  const tokensMode = $derived(sessionOrder.mode === 'tokens');
  $effect(() => {
    if (!canUsage) return;
    const wid = usageWs;
    if (!tokensMode) {
      void sessionUsage.loadWorkspace(wid);
      return;
    }
    const p = pollWhileVisible(() => sessionUsage.loadWorkspace(wid), { ms: 60_000 });
    return () => p.stop();
  });
  const usageRows = $derived(canUsage ? (sessionUsage.byWorkspace[usageWs] ?? {}) : {});
  const tokensSort = $derived(sessionOrder.mode === 'tokens' && canUsage && sessionUsage.available);
  const orderedAgents = $derived(
    sessionOrder.mode === 'manual'
      ? applyOrder(ws.plainAgentSessions, sessionOrder.order)
      : tokensSort
        ? applyTokenOrder(ws.plainAgentSessions, sessionUsage.tokensFor(usageWs))
        : ws.plainAgentSessions,
  );
  const fAgents = $derived(orderedAgents.filter(matches));
  // Rows drag only in the flat Agents list: never while searching, selecting,
  // needs-you filtering, or on phones (HTML5 DnD is inert on iOS Safari).
  const rowsDraggable = $derived(q === '' && !agentSelMode && !ws.needsYouFilter && !viewport.isPhone);
  let rowDragId = $state<string | null>(null);
  let rowDragOverId = $state<string | null>(null);
  function onRowDragStart(e: DragEvent, id: string): void {
    rowDragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onRowDragOver(e: DragEvent, id: string): void {
    if (!rowDragId || id === rowDragId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    rowDragOverId = id;
  }
  function onRowDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    if (rowDragId && rowDragId !== id) sessionOrder.dragTo(fAgents.map((x) => x.id), rowDragId, id);
    rowDragId = null;
    rowDragOverId = null;
  }
  function onRowDragEnd(): void {
    rowDragId = null;
    rowDragOverId = null;
  }
  function openSortMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'Sort: Recent', checked: sessionOrder.mode === 'recent', action: () => sessionOrder.setMode('recent') },
      {
        label: 'Sort: Manual (drag rows)',
        checked: sessionOrder.mode === 'manual',
        action: () => sessionOrder.setMode('manual', fAgents.map((x) => x.id)),
      },
      ...(canUsage && sessionUsage.available
        ? [{ label: 'Sort: Tokens (most first, 30 days)', checked: sessionOrder.mode === 'tokens', action: () => sessionOrder.setMode('tokens') }]
        : []),
      { separator: true },
      { label: 'Reset to recent', icon: 'refresh', disabled: sessionOrder.mode === 'recent', action: () => sessionOrder.reset() },
    ]);
  }
  // Workspace-less sessions (the "No workspace" group below the flat list).
  const fScratch = $derived(ws.scratchSessions.filter(matches));
  // Select mode covers the flat list AND the "No workspace" group.
  const selectable = $derived([...fAgents, ...fScratch]);
  const agentSelIds = $derived(selectable.filter((s) => agentSel.has(s.id)).map((s) => s.id));
  const fTelegram = $derived(ws.telegramSessions.filter(matches));
  const fSlack = $derived(ws.slackSessions.filter(matches));
  // Capped views (full list when searching or "show all" toggled).
  const visTelegram = $derived(q || telegramShowAll ? fTelegram : fTelegram.slice(0, CHANNEL_CAP));
  const visSlack = $derived(q || slackShowAll ? fSlack : fSlack.slice(0, CHANNEL_CAP));

  // Drag-to-resize the navigator from its inline-end edge (widens the session area).
  let resizing = $state(false);
  // Overlay cursor + one width write per frame + one localStorage write on
  // release (lib/dragCursor.ts) — no body.style restyle of the whole app.
  function startResize(e: MouseEvent): void {
    resizing = true;
    const startX = e.clientX;
    const startW = ui.railWidth;
    const rtl = typeof document !== 'undefined' && getComputedStyle(document.documentElement).direction === 'rtl';
    startMouseDrag(e, {
      onMove: (ev) => ui.setRailWidth(startW + (rtl ? startX - ev.clientX : ev.clientX - startX), false),
      onEnd: () => {
        resizing = false;
        ui.setRailWidth(ui.railWidth);
      },
    });
  }

  function openSession(id: string): void {
    ws.navigateToSession(id);
  }

  /** Respawn a stuck in-progress agent's PTY (the terminal reconnects itself).
   *  Asks first when it is working, same as the pane header. */
  function restartAgent(id: string): Promise<void> {
    return ws.requestRestart(id);
  }

  /** Delete = PTY killed, row + full history gone. Asks first unless the
   *  user chose "Always delete" (see ws.requestDeleteSession). */
  function deleteSession(id: string): Promise<void> {
    return ws.requestDeleteSession(id);
  }

  // ── Workspace quick filter ──────────────────────────────────────────────────
  // Name or root-path substring (case-insensitive). The box only appears once
  // the list is long enough to need it (or while a query is still set, so a
  // workspace removed mid-filter can't strand a hidden query).
  const WS_FILTER_MIN = 5;
  let wsQuery = $state('');
  const wq = $derived(wsQuery.trim().toLowerCase());
  const showWsFilter = $derived(ws.workspaces.length >= WS_FILTER_MIN || wsQuery !== '');
  const fWorkspaces = $derived(
    wq === ''
      ? ws.workspaces
      : ws.workspaces.filter((w) => w.name.toLowerCase().includes(wq) || w.root_path.toLowerCase().includes(wq)),
  );
  /** Esc clears the query (a second Esc leaves the box); Enter switches to the
   *  first match and clears, so "type a few letters ⏎" is a workspace switcher. */
  function wsFilterKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (wsQuery) wsQuery = '';
      else (e.currentTarget as HTMLInputElement).blur();
    } else if (e.key === 'Enter' && wq !== '' && fWorkspaces.length > 0) {
      e.preventDefault();
      void ws.select(fWorkspaces[0].id);
      wsQuery = '';
    }
  }

  /** The current-workspace chip under Agents: a switcher menu (filterable once
   *  the list is long, like the Workspaces section's quick filter). */
  function openWsSwitcher(e: MouseEvent | KeyboardEvent): void {
    const items: MenuItem[] = ws.workspaces.map((w) => ({
      label: w.name,
      checked: ws.currentId === w.id,
      title: w.root_path,
      action: () => void ws.select(w.id),
    }));
    if (items.length > 0) items.push({ separator: true });
    items.push({ label: 'Add workspace…', icon: 'plus', pinned: true, action: () => (ui.newWorkspaceOpen = true) });
    ctxMenu.showAt(
      e.currentTarget as HTMLElement,
      items,
      ws.workspaces.length >= WS_FILTER_MIN ? { filter: true, filterPlaceholder: 'Switch workspace…', maxVisible: 12 } : undefined,
    );
  }

  // ── Workspace management (context menu on the Workspaces rows) ──────────────
  async function renameWorkspace(w: WorkspaceWithRole): Promise<void> {
    const name = await confirmer.promptText('New workspace name', {
      title: `Rename “${w.name}”`,
      confirmLabel: 'Rename',
      initial: w.name,
    });
    if (!name || name === w.name) return;
    try {
      await ws.updateWorkspace(w.id, { name });
    } catch (e) {
      toastError('Couldn’t rename', e);
    }
  }

  async function changeWorkspaceDir(w: WorkspaceWithRole): Promise<void> {
    const root = await confirmer.promptText('Working folder (absolute path, ~ ok)', {
      title: `Change folder of “${w.name}”`,
      browseFolder: true,
      confirmLabel: 'Change',
      initial: w.root_path,
      placeholder: '~/projects/my-repo',
    });
    if (!root || root === w.root_path) return;
    try {
      await ws.updateWorkspace(w.id, { root_path: root });
      toasts.success('Folder changed', `${w.name} → ${root}`);
    } catch (e) {
      toastError('Couldn’t change it', e);
    }
  }

  /** "Remove", not "Delete": the workspace is archived (off the sidebar) and
   *  its folder and files are never touched (patterns.md §7 — Remove detaches,
   *  the thing still exists). */
  async function removeWorkspace(w: WorkspaceWithRole): Promise<void> {
    const ok = await confirmer.ask(
      `Remove “${w.name}” from Otto? It leaves the sidebar with its sessions; the folder and its files on disk are not touched.`,
      { title: 'Remove workspace', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      if (!(await ws.archiveWorkspace(w.id))) return;
      toasts.info('Workspace removed', w.name);
    } catch (e) {
      toastError('Couldn’t remove the workspace', e);
    }
  }

  function startRename(id: string, current: string): void {
    const s = ws.getSession(id);
    if (!s || !ws.canEditSession(s)) return;
    renamingId = id;
    draft = current;
  }

  /** Open the New Session sheet in workspace `id` (switching there first —
   *  the sheet always targets the current workspace). */
  async function newSessionIn(id: string): Promise<void> {
    if (ws.currentId !== id && !(await ws.select(id))) return;
    ui.newSessionScratch = false;
    ui.newSessionOpen = true;
  }

  /** Right-click on a workspace folder (the Agents chip, an all-workspaces
   *  group, a Workspaces row): start work there first, then manage it. */
  function workspaceMenuItems(w: WorkspaceWithRole): MenuItem[] {
    const items: MenuItem[] = [
      { label: `New session in “${w.name}”…`, icon: 'plus', action: () => void newSessionIn(w.id) },
      { label: 'New session (no workspace)…', icon: 'home', action: newScratchSession },
      { separator: true },
    ];
    if (ws.currentId !== w.id) {
      items.push({ label: 'Switch to this workspace', icon: 'check', action: () => void ws.select(w.id) });
    }
    items.push(
      {
        label: 'Copy folder path',
        icon: 'copy',
        title: w.root_path,
        action: async () => {
          if (await copyText(w.root_path)) toasts.success('Copied', w.root_path);
          else toasts.error('Couldn’t copy the path', `The clipboard isn’t available here. Select the path and copy it manually: ${w.root_path}`);
        },
      },
      { label: 'Workspace context', icon: 'note', action: async () => { if (await ws.select(w.id)) router.go('settings/context-soul'); } },
    );
    if (w.my_role === 'admin') {
      items.push(
        { separator: true },
        { label: 'Rename…', icon: 'edit', action: () => void renameWorkspace(w) },
        { label: 'Change folder…', icon: 'folder', action: () => void changeWorkspaceDir(w) },
        { label: 'Remove workspace…', icon: 'trash', danger: true, action: () => void removeWorkspace(w) },
      );
    }
    items.push({ separator: true }, { label: 'Add workspace…', icon: 'plus', action: () => (ui.newWorkspaceOpen = true) });
    return items;
  }

  /** Open the New Session sheet pre-set to "No workspace". */
  function newScratchSession(): void {
    ui.newSessionScratch = true;
    ui.newSessionOpen = true;
  }

  async function commitRename(): Promise<void> {
    const id = renamingId;
    renamingId = null;
    if (!id) return;
    const next = draft.trim();
    if (next) await ws.renameSession(id, next);
  }

  // ── Module list (shared registry → RBAC + plugins → user order/visibility) ──
  // `resolved` is the full ordered list (incl. hidden); `visible` drops hidden
  // modules; `navList` is what we actually render — everything while editing (so
  // hidden rows can be toggled back on), only the visible ones otherwise.
  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p): SidebarPluginEntry => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const resolved = $derived(
    resolveOrder(availableModules((f) => auth.can(f, 'view'), pluginEntries), ui.sidebarOrder),
  );
  const visible = $derived(visibleOrder(resolved, ui.sidebarHidden));
  const navList = $derived(ui.sidebarEditMode ? resolved : visible);
  // Sections: Favorites first (only while it holds something), then Work /
  // Automate / Build / … in the user's section order; the saved module order
  // applies within each. A favorited module shows ONLY under Favorites. A
  // section whose modules are all hidden (or all favorited) drops out.
  const sections = $derived(sidebarSections(navList, ui.sidebarFavorites, ui.sidebarGroupOrder));
  /** The favorites as rendered right now (RBAC-filtered; hidden ones only
   *  while editing), in order — what the move/drag bounds are measured on. */
  const favIds = $derived(sections[0]?.group.id === FAVORITES_ID ? sections[0].modules.map((m) => m.id) : []);
  function isFav(id: string): boolean {
    return favIds.includes(id);
  }

  // The sidebar id the current route highlights (plugin slug / default route /
  // database+brokers → Connections are resolved in activeNavId).
  const activeId = $derived(activeNavId(router.parts));
  function isActive(id: string): boolean {
    return activeId === id;
  }
  // The section holding the current page — Favorites when it's favorited.
  const activeGroup = $derived(
    ui.sidebarFavorites.includes(activeId) && resolved.some((m) => m.id === activeId)
      ? FAVORITES_ID
      : (resolved.find((m) => m.id === activeId)?.group ?? null),
  );

  function isHidden(id: string): boolean {
    return ui.sidebarHidden.includes(id);
  }

  // ── Section fold state ──────────────────────────────────────────────────
  // The section holding the current page is never shown folded: navigating
  // into a folded section unfolds it (and persists that, so the state the
  // user sees is the state that's saved). While a session search / Needs-you
  // filter is on, Agents' section is forced open so its results show; edit
  // mode opens everything so every row can be reordered / re-shown.
  $effect(() => {
    const g = activeGroup;
    if (g) untrack(() => ui.setSidebarGroupCollapsed(g, false));
  });
  /** Held open regardless of the saved fold state (so its header can't fold it). */
  function sectionPinned(sec: SidebarSection): boolean {
    if (ui.sidebarEditMode || sec.group.id === activeGroup) return true;
    return (q !== '' || ws.needsYouFilter) && sec.modules.some((m) => m.id === 'agents');
  }
  function sectionOpen(sec: SidebarSection): boolean {
    return sectionPinned(sec) || !ui.sidebarCollapsedGroups.includes(sec.group.id);
  }

  // Keep the current row on screen: on every route / focused-session change,
  // scroll the active row (the focused session's row when there is one, else
  // the module row) into view — at laptop heights the list overflows and the
  // active entry would otherwise sit below the fold.
  let scrollEl = $state<HTMLDivElement>();
  $effect(() => {
    void router.parts.join('/');
    void ws.activeSessionId;
    const el = scrollEl;
    if (!el) return;
    void tick().then(() => {
      const row =
        el.querySelector<HTMLElement>('.nav-item.nested-item.active') ??
        el.querySelector<HTMLElement>('.nav-item.active');
      row?.scrollIntoView({ block: 'nearest' });
    });
  });

  // ── Reordering ─────────────────────────────────────────────────────────
  // Edit mode: every row drags (HTML5 DnD, like TabBar) and has up/down
  // buttons (touch-reliable + keyboard-accessible). Moves stay inside a
  // section; the one crossing is a drop ONTO a favorite, which favorites the
  // dragged module at that slot. Favorites also drag outside edit mode and
  // move with ⌥↑ / ⌥↓ (Finder's sidebar Favorites are always reorderable),
  // except on phones where HTML5 DnD is inert. Section headers get up/down
  // (and drag) while editing, and "Move section up/down" in their menu.
  let dragId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);
  /** Where the dragged row would land relative to the hovered one. */
  let dropSide = $state<'before' | 'after'>('before');

  /** Can the dragged module `from` be dropped on the row `to`? */
  function canDrop(from: string | null, to: string): boolean {
    if (!from || from === to) return false;
    if (isFav(to)) return true; // reorder inside Favorites, or favorite-at-slot
    return !isFav(from) && sameGroup(from, to);
  }
  function onDragStart(e: DragEvent, id: string): void {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onDragOver(e: DragEvent, id: string): void {
    if (!canDrop(dragId, id)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverId = id;
    dropSide = dropSideFor(dragId!, id);
  }
  function onDragLeave(id: string): void {
    if (dragOverId === id) dragOverId = null;
  }
  function onDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    const from = dragId;
    dragId = null;
    dragOverId = null;
    if (!from || !canDrop(from, id)) return;
    if (isFav(id)) {
      if (isFav(from)) {
        const next = reorderAmong(ui.sidebarFavorites, from, id);
        if (next) ui.setSidebarFavorites(next);
      } else ui.addSidebarFavorite(from, id);
    } else ui.reorderSidebar(resolved.map((m) => m.id), from, id);
  }
  function onDragEnd(): void {
    dragId = null;
    dragOverId = null;
  }
  /** A drop lands before the target when dragging up (or in from another
   *  section), after it when dragging down — the indicator says which. */
  function dropSideFor(from: string, to: string): 'before' | 'after' {
    const list = isFav(to) ? favIds : resolved.map((m) => m.id);
    const a = list.indexOf(from);
    return a >= 0 && a < list.indexOf(to) ? 'after' : 'before';
  }
  /** Drag + keyboard reorder props for a Favorites row OUTSIDE edit mode
   *  (spread onto the row's button; nothing for other rows or on phones). */
  function favRowProps(id: string): Record<string, unknown> {
    if (ui.sidebarEditMode || viewport.isPhone || !isFav(id)) return {};
    return {
      draggable: true,
      'aria-keyshortcuts': 'Alt+ArrowUp Alt+ArrowDown',
      ondragstart: (e: DragEvent) => onDragStart(e, id),
      ondragover: (e: DragEvent) => onDragOver(e, id),
      ondragleave: () => onDragLeave(id),
      ondrop: (e: DragEvent) => onDrop(e, id),
      ondragend: onDragEnd,
      onkeydown: (e: KeyboardEvent) => {
        if (!e.altKey || e.metaKey || e.ctrlKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
        e.preventDefault();
        moveFavorite(id, e.key === 'ArrowUp' ? -1 : 1, `[data-nav-id="${CSS.escape(id)}"]`);
      },
    };
  }

  /** After a move re-renders the list, put focus back where it was: a keyed
   *  row that the DOM moved loses focus otherwise. Falls back to the sibling
   *  control when the moved-to end disables the one that was pressed. */
  function refocus(selector: string, fallback?: string): void {
    const root = scrollEl;
    if (!root) return;
    void tick().then(() => {
      const el = root.querySelector<HTMLElement>(selector);
      if (el && !(el as HTMLButtonElement).disabled) el.focus();
      else if (fallback) root.querySelector<HTMLElement>(fallback)?.focus();
    });
  }
  /** Selector for one of an edit row's move buttons (by its aria-label). */
  const moveBtn = (label: string, dir: 'up' | 'down') => `[aria-label="${CSS.escape(`Move ${label} ${dir}`)}"]`;

  // Reordering stays inside a section: up/down swap with the nearest
  // same-section neighbour, and a drag only targets rows of its own section.
  function move(m: SidebarModule, delta: -1 | 1): void {
    if (isFav(m.id)) {
      moveFavorite(m.id, delta, moveBtn(m.label, delta < 0 ? 'up' : 'down'), moveBtn(m.label, delta < 0 ? 'down' : 'up'));
      return;
    }
    const next = moveWithinGroup(resolved, m.id, delta);
    if (next) ui.setSidebarOrder(next);
    refocus(moveBtn(m.label, delta < 0 ? 'up' : 'down'), moveBtn(m.label, delta < 0 ? 'down' : 'up'));
  }
  /** Star toggle (edit mode): the row jumps between Favorites and its own
   *  section, so keep keyboard focus on its star. */
  function toggleFavorite(id: string): void {
    ui.toggleSidebarFavorite(id);
    refocus(`[data-testid="sidebar-fav-${CSS.escape(id)}"]`);
  }
  /** Move a favorite among the RENDERED favorites (saved ids the user can't
   *  see keep their slots and are hopped over). */
  function moveFavorite(id: string, delta: -1 | 1, focusSel?: string, fallback?: string): void {
    const next = moveAmong(ui.sidebarFavorites, id, delta, isFav);
    if (next) ui.setSidebarFavorites(next);
    if (focusSel) refocus(focusSel, fallback);
  }
  function sameGroup(a: string | null, b: string): boolean {
    const ga = resolved.find((m) => m.id === a)?.group;
    return ga != null && ga === resolved.find((m) => m.id === b)?.group;
  }

  // ── Sections: order ─────────────────────────────────────────────────────
  // Favorites is always first and never moves. The others swap with their
  // nearest RENDERED neighbour (an empty section — no plugins, all hidden —
  // isn't a visible slot), persisted as the full resolved section order.
  const movableSections = $derived(sections.filter((s) => s.group.id !== FAVORITES_ID).map((s) => s.group.id as string));
  function canMoveSection(id: string, delta: -1 | 1): boolean {
    const i = movableSections.indexOf(id);
    return i >= 0 && i + delta >= 0 && i + delta < movableSections.length;
  }
  function moveSection(id: string, delta: -1 | 1, focusSel?: string, fallback?: string): void {
    const all = resolveGroupOrder(ui.sidebarGroupOrder).map((g) => g.id as string);
    const next = moveAmong(all, id, delta, (g) => movableSections.includes(g));
    if (next) ui.setSidebarGroupOrder(next);
    if (focusSel) refocus(focusSel, fallback);
  }
  const sectionBtn = (label: string, dir: 'up' | 'down') =>
    `[aria-label="${CSS.escape(`Move ${label} section ${dir}`)}"]`;
  // Section drag (edit mode): the header row is the handle; any other
  // movable section's block is a target.
  let secDragId = $state<string | null>(null);
  let secDragOverId = $state<string | null>(null);
  let secDropSide = $state<'before' | 'after'>('before');
  function onSecDragStart(e: DragEvent, id: string): void {
    secDragId = id;
    e.dataTransfer?.setData('text/plain', `section:${id}`);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onSecDragOver(e: DragEvent, id: string): void {
    if (!secDragId || secDragId === id || !movableSections.includes(id)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    secDragOverId = id;
    secDropSide = movableSections.indexOf(secDragId) < movableSections.indexOf(id) ? 'after' : 'before';
  }
  function onSecDrop(e: DragEvent, id: string): void {
    if (!secDragId) return;
    e.preventDefault();
    const from = secDragId;
    secDragId = null;
    secDragOverId = null;
    if (from === id || !movableSections.includes(id)) return;
    const next = reorderAmong(resolveGroupOrder(ui.sidebarGroupOrder).map((g) => g.id as string), from, id);
    if (next) ui.setSidebarGroupOrder(next);
  }
  function onSecDragEnd(): void {
    secDragId = null;
    secDragOverId = null;
  }

  // ── Context menus ───────────────────────────────────────────────────────
  /** A module row's Favorites entries (+ in-Favorites moves). Shared by the
   *  plain rows and the Agents row, which prepends its own session verbs. */
  function favoriteMenuItems(m: SidebarModule): MenuItem[] {
    if (!isFav(m.id)) {
      return [{ label: 'Add to Favorites', icon: 'star', action: () => ui.addSidebarFavorite(m.id) }];
    }
    const i = favIds.indexOf(m.id);
    return [
      { label: 'Remove from Favorites', icon: 'star', action: () => ui.removeSidebarFavorite(m.id) },
      { label: 'Move up', icon: 'arrowUp', hint: '⌥↑', disabled: i <= 0, action: () => moveFavorite(m.id, -1) },
      { label: 'Move down', icon: 'arrowDown', hint: '⌥↓', disabled: i >= favIds.length - 1, action: () => moveFavorite(m.id, 1) },
    ];
  }
  const customizeItem = (): MenuItem => ({
    label: 'Customize sidebar',
    icon: 'edit',
    action: () => (ui.sidebarEditMode = true),
  });
  /** "Open side by side" & co. (stores/sidePane.svelte.ts), then a separator. */
  function splitItems(m: SidebarModule): MenuItem[] {
    const items = splitMenuItems(m.id, m.label);
    return items.length ? [...items, { separator: true }] : [];
  }
  function moduleMenu(e: MouseEvent, m: SidebarModule): void {
    // Plain clicks resume the module where it was left; this is the way home.
    const home: MenuItem = { label: `Open ${m.label} main page`, icon: 'home', action: () => router.go(m.id) };
    ctxMenu.show(e, [home, { separator: true }, ...splitItems(m), ...favoriteMenuItems(m), { separator: true }, customizeItem()]);
  }
  function sectionMenu(e: MouseEvent, sec: SidebarSection): void {
    const id = sec.group.id;
    const open = sectionOpen(sec);
    const items: MenuItem[] = [
      {
        label: open ? 'Collapse section' : 'Expand section',
        icon: open ? 'chevronRight' : 'chevronDown',
        disabled: sectionPinned(sec),
        action: () => ui.toggleSidebarGroup(id),
      },
    ];
    if (id !== FAVORITES_ID) {
      items.push(
        { separator: true },
        { label: 'Move section up', icon: 'arrowUp', disabled: !canMoveSection(id, -1), action: () => moveSection(id, -1) },
        { label: 'Move section down', icon: 'arrowDown', disabled: !canMoveSection(id, 1), action: () => moveSection(id, 1) },
      );
    }
    items.push({ separator: true }, customizeItem());
    ctxMenu.show(e, items);
  }
;
async () => {

 { svelteHTML.createElement("nav", {     "class":`navigator sidebar-material`,"aria-label":`Navigator`,"style":`width:${ui.railWidth}px`,});resizing;
  
   {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: ui.railWidth, min: RAIL_MIN, max: RAIL_MAX, onChange: (w) => ui.setRailWidth(w), onReset: () => ui.setRailWidth(240), onDragStart: startResize, text: pxWide })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`rail-resize`,"role":`separator`,"aria-label":`Resize the navigator`,"title":RESIZE_TITLE,}); }}
   { svelteHTML.createElement("div", {  "class":`nav-head`,});false;
     { svelteHTML.createElement("img", {        "class":`nav-logo`,"src":`/otto-mark-64.png`,"alt":"","width":`20`,"height":`20`,});}
     { svelteHTML.createElement("span", { "class":`nav-title`,});  }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    
    if(viewport.isDesktop){
     { svelteHTML.createElement("button", {             "class":`icon-btn nav-back`,"onclick":() => router.back(),"disabled":!router.canBack,"title":`Go back`,"aria-label":`Go back`,"aria-keyshortcuts":`Meta+Shift+ArrowLeft`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
     }
     { svelteHTML.createElement("button", {             "class":`icon-btn`,"onclick":() => router.forward(),"disabled":!router.canForward,"title":`Go forward`,"aria-label":`Go forward`,"aria-keyshortcuts":`Meta+Shift+ArrowRight`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":14,}});}
     }
    }
    
     { const $$_lleBnoitacifitoN2C = __sveltets_2_ensureComponent(NotificationBell); new $$_lleBnoitacifitoN2C({ target: __sveltets_2_any(), props: {}});}
    
    
    if(!viewport.isTablet){
     { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => (viewport.isPhone ? (ui.navDrawerOpen = false) : ui.toggleRail()),"title":viewport.isPhone ? 'Close sidebar' : 'Collapse sidebar',"aria-label":viewport.isPhone ? 'Close sidebar' : 'Collapse sidebar',"aria-keyshortcuts":viewport.isPhone ? undefined : 'Meta+1',});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sidebar`,"size":14,}});}
     }
    }
   }

   { const $$_div1 = svelteHTML.createElement("div", {  "class":`nav-scroll`,});scrollEl = $$_div1;
    
    if(!ui.sidebarEditMode){
       { svelteHTML.createElement("div", { "class":`nav-search`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`nav-search-input`,"placeholder":`Filter sessions…`,"aria-label":`Filter sessions`,"bind:value":sessionQuery,"onkeydown":(e) => {
            if (e.key !== 'Escape') return;
            e.preventDefault();
            e.stopPropagation();
            if (sessionQuery) sessionQuery = '';
            else e.currentTarget.blur();
          },});/*Ωignore_startΩ*/() => sessionQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(sessionQuery){
           { svelteHTML.createElement("button", {       "class":`search-clear`,"onclick":() => (sessionQuery = ''),"aria-label":`Clear search`,"title":`Clear search`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
           }
        }
       }

      
      if(ws.needsYouCount > 0 || ws.needsYouFilter){
         { svelteHTML.createElement("button", {          "class":`needs-you-filter`,"aria-pressed":ws.needsYouFilter,"onclick":() => (ws.needsYouFilter = !ws.needsYouFilter),"title":`Show only sessions waiting on you`,});ws.needsYouFilter;
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":12,}});}
           { svelteHTML.createElement("span", { "class":`grow`,});  }
          if(ws.needsYouCount > 0){
             { svelteHTML.createElement("span", { "class":`needs-you-count`,});ws.needsYouCount; }
          }
         }
      }
    }

     { svelteHTML.createElement("div", {   "class":`nav-section modules`,...__sveltets_2_empty({"data-testid":`sidebar-modules`}),});
      if(ui.sidebarEditMode){
         { svelteHTML.createElement("p", { "class":`edit-hint`,});                   }
      }

      
         for(let sec of __sveltets_2_ensureArray(sections)){sec.group.id;
        const open = sectionOpen(sec);
        const pinned = sectionPinned(sec);
        const fav = sec.group.id === FAVORITES_ID;
        const secMovable = ui.sidebarEditMode && !fav;
         { svelteHTML.createElement("div", {                     "class":`nav-group`,"role":`group`,"aria-label":sec.group.label,...__sveltets_2_empty({"data-testid":`sidebar-group-${sec.group.id}`}),...__sveltets_2_empty({"data-open":open}),"ondragover":secMovable ? (e) => onSecDragOver(e, sec.group.id) : undefined,"ondragleave":secMovable ? () => { if (secDragOverId === sec.group.id) secDragOverId = null; } : undefined,"ondrop":secMovable ? (e) => onSecDrop(e, sec.group.id) : undefined,});fav;secDragId === sec.group.id;secDragOverId === sec.group.id && secDropSide === 'before';secDragOverId === sec.group.id && secDropSide === 'after';
          
           { svelteHTML.createElement("div", {           "role":`presentation`,"class":`group-head-row`,"draggable":secMovable,"ondragstart":secMovable ? (e) => onSecDragStart(e, sec.group.id) : undefined,"ondragend":secMovable ? onSecDragEnd : undefined,});
            if(secMovable){
               { svelteHTML.createElement("span", {     "class":`grip sec-grip`,"title":`Drag to reorder sections`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`grip`,"size":12,}});} }
            }
             {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {               "class":`group-head`,"aria-expanded":open,"onclick":() => !pinned && ui.toggleSidebarGroup(sec.group.id),"oncontextmenu":(e) => sectionMenu(e, sec),"title":pinned ? undefined : open ? `Hide ${sec.group.label}` : `Show ${sec.group.label}`,...__sveltets_2_empty({"data-testid":`sidebar-group-head-${sec.group.id}`}),});pinned;
               { svelteHTML.createElement("span", { "class":`group-label`,});sec.group.label; }
              if(fav){ { svelteHTML.createElement("span", {   "class":`group-star`,"aria-hidden":`true`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});} }}
              if(!open && sec.modules.some((m) => m.id === 'agents') && ws.workingCount > 0){
                 { svelteHTML.createElement("span", {   "class":`count-chip working`,"title":`Working sessions`,});ws.workingCount; }
              }
              if(!pinned){
                 { svelteHTML.createElement("span", { "class":`group-chev`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});} }
              }
             }}
            if(secMovable){
               { svelteHTML.createElement("button", {           "class":`row-action mv`,"onclick":() => moveSection(sec.group.id, -1, sectionBtn(sec.group.label, 'up'), sectionBtn(sec.group.label, 'down')),"disabled":!canMoveSection(sec.group.id, -1),"title":`Move section up`,"aria-label":`Move ${sec.group.label} section up`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}
               }
               { svelteHTML.createElement("button", {           "class":`row-action mv`,"onclick":() => moveSection(sec.group.id, 1, sectionBtn(sec.group.label, 'down'), sectionBtn(sec.group.label, 'up')),"disabled":!canMoveSection(sec.group.id, 1),"title":`Move section down`,"aria-label":`Move ${sec.group.label} section down`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}
               }
            }
           }
          if(open){
                for(let m of __sveltets_2_ensureArray(ui.sidebarEditMode ? sec.modules : agentsLast(sec.modules))){let i = 1;m.id;
              if(ui.sidebarEditMode){
                ;__sveltets_2_ensureSnippet(editRow(m, i === 0, i === sec.modules.length - 1));
              } else if (m.id === 'agents'){
                ;__sveltets_2_ensureSnippet(agentsBlock(m));
              }else{
                ;__sveltets_2_ensureSnippet(simpleRow(m));
              }
            }
          }
         }
      }
     }

     { svelteHTML.createElement("div", { "class":`nav-section`,});
       { svelteHTML.createElement("div", { "class":`nav-label-row`,});
         { svelteHTML.createElement("span", { "class":`nav-label`,});  }
         { svelteHTML.createElement("button", {         "class":`icon-btn add-ws`,"onclick":() => (ui.newWorkspaceOpen = true),"title":`Add workspace`,"aria-label":`Add workspace`,});
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}
         }
       }
      if(showWsFilter){
         { svelteHTML.createElement("div", { "class":`nav-search ws-filter`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
           { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`nav-search-input`,"placeholder":`Filter workspaces…`,"aria-label":`Filter workspaces`,...__sveltets_2_empty({"data-testid":`ws-filter`}),"bind:value":wsQuery,"onkeydown":wsFilterKey,});/*Ωignore_startΩ*/() => wsQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(wsQuery){
             { svelteHTML.createElement("button", {       "class":`search-clear`,"onclick":() => (wsQuery = ''),"aria-label":`Clear workspace filter`,"title":`Clear workspace filter`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
          }
         }
      }
      if(wq !== '' && fWorkspaces.length === 0){
         { svelteHTML.createElement("div", { "class":`nested-empty`,});   }
      }
         for(let w of __sveltets_2_ensureArray(fWorkspaces)){w.id;
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {           "class":`nav-item`,"onclick":() => ws.select(w.id),"oncontextmenu":(e) => ctxMenu.show(e, workspaceMenuItems(w)),"title":w.root_path,});ws.currentId === w.id;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`grow ellipsis`,});w.name; }
          if(ws.currentId === w.id){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}
         }}
      }
     }
   }

   { svelteHTML.createElement("div", { "class":`nav-foot`,});
    if(ui.sidebarEditMode){
      
       { svelteHTML.createElement("div", { "class":`edit-foot`,});
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => ui.resetSidebar(),...__sveltets_2_empty({"data-testid":`sidebar-reset`}),"title":`Restore the default order, sections and visibility, and clear Favorites`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}   
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":() => ui.toggleSidebarEdit(),"title":`Finish customizing the sidebar`,...__sveltets_2_empty({"data-testid":`sidebar-edit-toggle`}),});
          
         }
       }
    }else{
       { svelteHTML.createElement("button", {         "class":`nav-item subtle`,"onclick":() => ui.toggleSidebarEdit(),"title":`Show, hide and reorder sidebar items`,...__sveltets_2_empty({"data-testid":`sidebar-edit-toggle`}),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":14,}});}
         { svelteHTML.createElement("span", { "class":`grow`,});  }
       }
    }
     { svelteHTML.createElement("button", {          "class":`nav-item`,"aria-current":router.module === 'walkthroughs' ? 'page' : undefined,"onclick":() => router.go('walkthroughs'),"title":`Help`,});router.module === 'walkthroughs';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`grow`,});  }
     }
     { svelteHTML.createElement("button", {            "class":`nav-item`,"aria-current":router.module === 'settings' ? 'page' : undefined,"onclick":() => router.go('settings/appearance'),"title":`Settings`,"aria-keyshortcuts":`Meta+,`,});router.module === 'settings';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`grow`,});  }
     }
     { svelteHTML.createElement("div", { "class":`nav-user`,});
       { svelteHTML.createElement("span", { "class":`avatar`,});(auth.me?.display_name ?? '?').slice(0, 1).toUpperCase(); }
       { svelteHTML.createElement("div", {   "class":`grow`,"title":auth.me?.display_name,});
         { svelteHTML.createElement("div", { "class":`user-name ellipsis`,});auth.me?.display_name; }
         { svelteHTML.createElement("div", { "class":`user-sub ellipsis`,});auth.isRoot ? 'root' : auth.me?.username; }
       }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => auth.logout(),"title":`Sign out`,"aria-label":`Sign out`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`logout`,"size":14,}});}
       }
     }
   }
 }

if(shareSessionId){
  
  const id = shareSessionId;
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":ShareModalChunk,"what":`the share dialog`,"quiet":true,"onerror":() => (shareSessionId = null),"props":{ sessionId: id, onclose: () => (shareSessionId = null) },}});}
}













};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Navigator__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Navigator__SvelteComponent_ = ReturnType<typeof Navigator__SvelteComponent_>;
/*Ωignore_endΩ*/export default Navigator__SvelteComponent_;