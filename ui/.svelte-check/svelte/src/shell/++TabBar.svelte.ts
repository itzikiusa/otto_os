///<reference types="svelte" />
;
import LazyMount from '../lib/components/LazyMount.svelte';
import { ShareModalChunk } from './overlays.svelte';
import { rowMenu } from '../lib/rowMenu';
// Session tabs: click activates, middle-click closes, ⌘W closes active,
// ⌃Tab cycles (handled in keys.ts → workspace store).
import Icon from '../lib/components/Icon.svelte';
import { focusOnMount } from '../lib/focusOnMount';
import { toastError } from '../lib/toastError';
import StatusDot from '../lib/components/StatusDot.svelte';
import { events } from '../lib/events.svelte';
import { sessionState, type SessionStateInfo } from '../lib/status';
import { ws, DB_PANE_ID } from '../lib/stores/workspace.svelte';
import { ui, isTauri } from '../lib/stores/ui.svelte';
import { startWindowDrag } from '../lib/windowDrag';
import { router } from '../lib/router.svelte';
import { sidePane } from '../lib/stores/sidePane.svelte';
import { embedChrome } from '../lib/stores/embedChrome.svelte';
import { isEmbedded } from '../lib/desktop';
import PaneControls from '../lib/components/PaneControls.svelte';
import { ctxMenu } from '../lib/contextmenu.svelte';
import { popoutItems } from '../lib/popoutMenu';
import { confirmer } from '../lib/confirm.svelte';
import type { MenuItem } from '../lib/contextmenu.svelte';
import { onTabKey } from '../lib/tabKeys';
import { tick } from 'svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Share modal: tracks the session id we're sharing; null = closed.
  let shareSessionId = $state<string | null>(null);

  let renamingId: string | null = $state(null);
  let draft = $state('');

  function activate(id: string): void {
    // Navigate via the route so Back/Forward walk session history.
    ws.navigateToSession(id);
  }

  function title(id: string): string {
    if (id === DB_PANE_ID) return 'Database';
    return ws.getSession(id)?.title ?? '…';
  }

  /** Rich hover tooltip: title · provider · cwd (+ suspended note). */
  function tabTooltip(id: string): string {
    if (id === DB_PANE_ID) return 'Database Explorer';
    const s = ws.getSession(id);
    if (!s) return 'Double-click to rename';
    // One short line per fact: a single very long line made a native tooltip
    // wider than the window, pinned to its edge and clipped.
    const title = s.title.length > 80 ? `${s.title.slice(0, 79).trimEnd()}…` : s.title;
    const parts = [title, [s.provider, s.cwd].filter((p) => p !== '' && p != null).join(' · ')].filter(Boolean);
    const st = tabState(id);
    return [...parts, st.resumable ? (st.hint ?? st.label) : 'Double-click to rename'].join('\n');
  }

  // ── Keep the active tab visible + surface overflow ────────────────────────
  // The strip is overflow-x:auto with the scrollbar hidden, so without these a
  // ⌃Tab / ⌘] activation can land on a tab scrolled fully out of view with no
  // visual feedback at all.
  let tabsEl: HTMLElement | null = $state(null);
  let overflowing = $state(false);

  function measureOverflow(): void {
    overflowing = !!tabsEl && tabsEl.scrollWidth > tabsEl.clientWidth + 1;
  }

  $effect(() => {
    void ws.openTabs.length; // re-measure when tabs come and go
    measureOverflow();
  });

  $effect(() => {
    const id = ws.activeSessionId;
    if (!id || !tabsEl) return;
    tabsEl
      .querySelector(`[data-tab-id="${CSS.escape(id)}"]`)
      ?.scrollIntoView({ inline: 'nearest', block: 'nearest' });
  });

  // Keyboard on a focused tab: ←/→ (visual direction — RTL mirrors), Home and
  // End move AND select through the shared tablist helper; ⌥⇧←/→ moves the tab
  // itself (the keyboard path for drag-to-reorder); Delete closes it.
  function onTabKeydown(e: KeyboardEvent, id: string): void {
    if (e.altKey && e.shiftKey && !e.metaKey && !e.ctrlKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      e.preventDefault();
      const rtl = getComputedStyle(e.currentTarget as Element).direction === 'rtl';
      moveTab(id, (e.key === 'ArrowRight') !== rtl ? 1 : -1);
      return;
    }
    if ((e.key === 'Delete' || e.key === 'Backspace') && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      void ws.requestCloseTab(id);
      return;
    }
    onTabKey(e);
  }

  /** Move a tab one place toward the end (+1) or the start (−1) of the strip
   *  and keep focus on it. */
  function moveTab(id: string, dir: 1 | -1): void {
    const from = ws.openTabs.indexOf(id);
    const to = from + dir;
    if (from < 0 || to < 0 || to >= ws.openTabs.length) return;
    ws.reorderTab(id, to);
    void tick().then(() =>
      tabsEl?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(id)}"] .tab-main`)?.focus(),
    );
  }

  /** The tab's context menu. "Move left / right" are the pointer-free way to
   *  reorder (also ⌥⇧←/→ on a focused tab); they name the VISUAL direction,
   *  so in RTL "Move left" moves toward the end of the strip. */
  function tabMenu(id: string, tabIdx: number): MenuItem[] {
    const rtl = !!tabsEl && getComputedStyle(tabsEl).direction === 'rtl';
    const last = ws.openTabs.length - 1;
    const canBack = tabIdx > 0;
    const canFwd = tabIdx < last;
    const leftOk = rtl ? canFwd : canBack;
    const rightOk = rtl ? canBack : canFwd;
    return tidy([
      ...(id !== DB_PANE_ID && ws.myRole !== 'viewer'
        ? [{ label: 'Rename', icon: 'edit', action: () => startRename(id) } as MenuItem]
        : []),
      ...(id !== DB_PANE_ID
        ? [{ label: 'Share…', icon: 'share', action: () => (shareSessionId = id) } as MenuItem]
        : []),
      ...(id === DB_PANE_ID
        ? popoutItems('database', 'Database')
        : popoutItems(`agents/${id}`, ws.getSession(id)?.title)),
      ...(leftOk || rightOk
        ? [
            { separator: true } as MenuItem,
            ...(leftOk ? [{ label: 'Move left', hint: '⌥⇧←', action: () => moveTab(id, rtl ? 1 : -1) } as MenuItem] : []),
            ...(rightOk ? [{ label: 'Move right', hint: '⌥⇧→', action: () => moveTab(id, rtl ? -1 : 1) } as MenuItem] : []),
          ]
        : []),
      { separator: true },
      { label: 'Close tab', icon: 'x', action: () => void ws.requestCloseTab(id) },
      ...(ws.openTabs.length > 1
        ? [{
            label: 'Close others',
            icon: 'x',
            action: () => void ws.requestCloseTabs(ws.openTabs.filter((t) => t !== id)),
          }]
        : []),
      ...(tabIdx < ws.openTabs.length - 1
        ? [{
            label: 'Close to the right',
            icon: 'x',
            action: () => void ws.requestCloseTabs(ws.openTabs.slice(tabIdx + 1)),
          }]
        : []),
      ...(ws.openTabs.length > 1
        ? [{
            label: 'Close all tabs',
            icon: 'x',
            action: () => void ws.requestCloseTabs([...ws.openTabs]),
          }]
        : []),
      ...(ws.recentlyClosed.length > 0
        ? [{ label: 'Reopen closed tab', icon: 'refresh', action: () => ws.reopenClosedTab() }]
        : []),
      ...(id !== DB_PANE_ID ? sessionRows(id) : []),
      { separator: true },
      { label: 'New session…', icon: 'plus', action: () => (ui.newSessionOpen = true) },
      {
        label: ws.viewMode === 'tiled' ? 'Switch to tabbed view' : 'Switch to tiled view',
        icon: ws.viewMode === 'tiled' ? 'square' : 'grid',
        action: () => ws.setViewMode(ws.viewMode === 'tiled' ? 'tabs' : 'tiled'),
      },
      {
        label: ws.viewMode === 'mission' ? 'Exit Work Queue' : 'Work Queue',
        icon: 'gauge',
        action: () => ws.setViewMode(ws.viewMode === 'mission' ? 'tabs' : 'mission'),
      },
    ]);
  }

  // One session vocabulary (lib/status.ts): the tab's dot/↻ and tooltip line
  // come from `sessionState` — the same state the sidebar row shows. A stale
  // events socket stops the working pulse ("Reconnecting…").
  function tabState(id: string): SessionStateInfo {
    const s = ws.getSession(id);
    return sessionState(s, ws.statusMap[id] ?? 'idle', needsYou(id), { stale: events.state !== 'connected' });
  }
  function isResumable(id: string): boolean {
    return id !== DB_PANE_ID && tabState(id).resumable;
  }

  // A tab "needs you" when its session is blocked on operator input — distinct
  // from idle. Cleared by the store when the session is opened / fed input.
  function needsYou(id: string): boolean {
    return id !== DB_PANE_ID && ws.needsYou[id] === true;
  }

  function startRename(id: string): void {
    if (ws.myRole === 'viewer' || id === DB_PANE_ID) return;
    renamingId = id;
    draft = title(id);
  }

  async function commitRename(): Promise<void> {
    const id = renamingId;
    renamingId = null;
    if (!id) return;
    const next = draft.trim();
    if (!next || next === title(id)) return;
    try {
      await ws.renameSession(id, next);
    } catch (e) {
      toastError('Couldn’t rename the session', e);
    }
  }

  /** Drop leading, trailing and doubled separators — rows above are
   *  conditional (the DB pane has no Rename/Share; the web build no pop-out). */
  function tidy(items: MenuItem[]): MenuItem[] {
    const out: MenuItem[] = [];
    for (const it of items) {
      if (it.separator && (out.length === 0 || out[out.length - 1].separator)) continue;
      out.push(it);
    }
    while (out.length > 0 && out[out.length - 1].separator) out.pop();
    return out;
  }

  // The SESSION rows of the tab menu — the same verbs, words and order as the
  // pane ⋯ menu and the sidebar row (Rename · Share… · Restart · Archive ·
  // Delete), so a session means the same thing wherever it is right-clicked.
  function sessionRows(id: string): MenuItem[] {
    const s = ws.getSession(id);
    if (!s || !ws.canEditSession(s)) return [];
    const st = ws.statusMap[id] ?? s.status;
    return [
      { separator: true },
      ...(s.kind === 'agent' && (st === 'running' || st === 'working')
        ? [{ label: 'Restart session', icon: 'refresh', action: () => void ws.requestRestart(id) } as MenuItem]
        : []),
      ...(!s.archived
        ? [{ label: 'Archive', icon: 'archive', action: () => void archiveTab(id) } as MenuItem]
        : []),
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteTab(id) },
    ];
  }

  async function archiveTab(id: string): Promise<void> {
    try {
      await ws.requestArchive(id); // confirms first when the agent is mid-turn
    } catch (e) {
      toastError('Couldn’t archive the session', e);
    }
  }

  // Asks first unless the user chose "Always delete" (ws.requestDeleteSession).
  function deleteTab(id: string): Promise<void> {
    return ws.requestDeleteSession(id);
  }

  // ── Drag-to-reorder tabs ─────────────────────────────────────────────────
  // HTML5 drag: tracks which tab is being dragged (dragId) and which position
  // the tab is being dragged over (dragOverId). On drop we reorder and persist.
  let dragId: string | null = $state(null);
  let dragOverId: string | null = $state(null);

  function onDragStart(e: DragEvent, id: string): void {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDragOver(e: DragEvent, id: string): void {
    // Only accept drags from our own tabs.
    if (!dragId || id === dragId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverId = id;
  }

  function onDragLeave(id: string): void {
    if (dragOverId === id) dragOverId = null;
  }

  function onDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    if (!dragId || id === dragId) return;
    const targetIdx = ws.openTabs.indexOf(id);
    ws.reorderTab(dragId, targetIdx);
    dragId = null;
    dragOverId = null;
  }

  function onDragEnd(): void {
    dragId = null;
    dragOverId = null;
  }

  // In the side-by-side pane the tab row carries the pane's controls, like a
  // PageHeader does on every other page (lib/stores/embedChrome).
  let barEl: HTMLDivElement | undefined = $state();
  $effect(() => (barEl ? embedChrome.claim(barEl) : undefined));
  const hostsPrimaryPane = $derived(!isEmbedded && isTauri && sidePane.active && !!barEl && embedChrome.owner === barEl);
  const hostsPane = $derived(isEmbedded && !!barEl && embedChrome.owner === barEl);
;
async () => {


  { const $$_div0 = svelteHTML.createElement("div", {         "role":`presentation`,"class":`tabbar chrome-material`,...__sveltets_2_empty({"data-tauri-drag-region":true}),"onmousedown":startWindowDrag,});barEl = $$_div0;(isTauri && !isEmbedded && !ui.railExpanded && !(sidePane.showing && sidePane.placement === 'leading')) ||
    (hostsPane && embedChrome.padTraffic);
   { const $$_div1 = svelteHTML.createElement("div", {         "class":`tabs`,"onscroll":measureOverflow,"role":`tablist`,"aria-label":`Session tabs`,});overflowing;tabsEl = $$_div1;
        for(let id of __sveltets_2_ensureArray(ws.openTabs)){let tabIdx = 1;id;
      
       { svelteHTML.createElement("div", {            "class":`tab`,"role":`presentation`,...__sveltets_2_empty({"data-tab-id":id}),});ws.activeSessionId === id;isResumable(id);needsYou(id);ws.unread[id] === true;dragOverId === id;
        if(renamingId === id){
          if(id !== DB_PANE_ID){ { const $$_toDsutatS3C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS3C({ target: __sveltets_2_any(), props: {    "state":tabState(id),"size":6,}});}}
           {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {             "dir":`auto`,"class":`tab-rename`,"bind:value":draft,"aria-label":`Session name`,"onblur":commitRename,"onkeydown":(e) => {
              if (e.key === 'Enter') commitRename();
              else if (e.key === 'Escape') renamingId = null;
            },});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
        }else{
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                                  "class":`tab-main`,"role":`tab`,"aria-selected":ws.activeSessionId === id,"tabindex":ws.activeSessionId === id ? 0 : -1,"draggable":id !== DB_PANE_ID,"aria-keyshortcuts":`Alt+Shift+ArrowLeft Alt+Shift+ArrowRight Backspace Delete`,"onclick":() => activate(id),"onkeydown":(e) => onTabKeydown(e, id),"ondblclick":() => startRename(id),"ondragstart":(e) => onDragStart(e, id),"ondragover":(e) => onDragOver(e, id),"ondragleave":() => onDragLeave(id),"ondrop":(e) => onDrop(e, id),"ondragend":onDragEnd,"onauxclick":(e) => {
              if (e.button === 1) {
                e.preventDefault();
                void ws.requestCloseTab(id);
              }
            },"oncontextmenu":(e) => ctxMenu.show(e, tabMenu(id, tabIdx)),});
            if(id === DB_PANE_ID){
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":12,}});}
            }else{
              const st = tabState(id);
              if(st.resumable){
                 { svelteHTML.createElement("span", {       "class":`susp-dot`,"role":`img`,"aria-label":st.label,"title":st.hint,});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
                 }
              }else{
                 { const $$_toDsutatS4C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS4C({ target: __sveltets_2_any(), props: {    "state":st,"size":6,}});}
              }
            }
             { svelteHTML.createElement("span", {   "class":`tab-title`,"title":tabTooltip(id),});title(id); }
            if(needsYou(id)){
               { svelteHTML.createElement("span", {       "class":`tab-needs-you`,"title":`Waiting on you`,"role":`img`,"aria-label":`Needs you`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":12,}});}
               }
            } else if (ws.unread[id] === true){
               { svelteHTML.createElement("span", {       "class":`tab-unread`,"title":`New activity since you last looked`,"role":`img`,"aria-label":`Unread activity`,}); }
            }
           }}
        }
         { svelteHTML.createElement("button", {           "class":`tab-close reveal-on-hover`,"tabindex":ws.activeSessionId === id ? 0 : -1,"onclick":() => void ws.requestCloseTab(id),"aria-label":`Close ${title(id)}`,"title":ws.closeTabTitle(id),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
       }
    }
   }
  
   { svelteHTML.createElement("button", {           "class":`icon-btn history-btn`,"onclick":() => router.go('history'),"title":`Session history`,"aria-label":`Session history`,...__sveltets_2_empty({"data-testid":`agents-history-btn`}),});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":14,}});}
   }
   { svelteHTML.createElement("div", {     "class":`view-toggle`,"role":`group`,"aria-label":`View mode`,});
     { svelteHTML.createElement("button", {          "aria-pressed":ws.viewMode === 'tabs',"onclick":() => ws.setViewMode('tabs'),"title":`Tabbed view`,"aria-label":`Tabbed view`,});ws.viewMode === 'tabs';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}
     }
     { svelteHTML.createElement("button", {          "aria-pressed":ws.viewMode === 'tiled',"onclick":() => ws.setViewMode('tiled'),"title":`Tiled view`,"aria-label":`Tiled view`,});ws.viewMode === 'tiled';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});}
     }
     { svelteHTML.createElement("button", {          "aria-pressed":ws.viewMode === 'mission',"onclick":() => ws.setViewMode('mission'),"title":`Work Queue`,"aria-label":`Work Queue`,});ws.viewMode === 'mission';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`gauge`,"size":12,}});}
     }
   }
   { svelteHTML.createElement("button", {           "class":`icon-btn new-tab`,"onclick":() => (ui.newSessionOpen = true),"title":`New session`,"aria-keyshortcuts":`Meta+T`,"aria-label":`New session`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}
   }
  if(hostsPane){ { const $$_slortnoCenaP1C = __sveltets_2_ensureComponent(PaneControls); new $$_slortnoCenaP1C({ target: __sveltets_2_any(), props: {}});}} else if (hostsPrimaryPane){ { const $$_slortnoCenaP1C = __sveltets_2_ensureComponent(PaneControls); new $$_slortnoCenaP1C({ target: __sveltets_2_any(), props: {  "pane":`primary`,}});}}
 }



if(shareSessionId){
  
  const id = shareSessionId;
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":ShareModalChunk,"what":`the share dialog`,"quiet":true,"onerror":() => (shareSessionId = null),"props":{ sessionId: id, onclose: () => (shareSessionId = null) },}});}
}
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TabBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TabBar__SvelteComponent_ = ReturnType<typeof TabBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default TabBar__SvelteComponent_;