///<reference types="svelte" />
;
  // Each tab's panel is its own chunk, loaded the first time it is shown (perf
  // G1). The panel used to import all nine statically, so the Agents page —
  // the default landing — evaluated Git, Files, both Browsers, Canvas and the
  // API client (CodeMirror, xterm, marked, the git views) to render one tab.
  // One promise per panel, shared by every RightPanel instance (the desktop
  // aside and the mobile drawer); `panels` is reactive so a tab renders the
  // moment its chunk lands and a loaded tab switches in synchronously.
  import type { Component } from 'svelte';
  import { toastError } from '../lib/toastError';
  import { announce } from '../lib/announce.svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import { ui as uiStore } from '../lib/stores/ui.svelte';
  import { RIGHT_MIN, RIGHT_MAX } from '../lib/stores/ui.svelte';
  import { paneResizer, pxWide, RESIZE_TITLE } from '../lib/paneResizer';
  // Panels take different props (v1 Browser: `active`).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  type PanelComponent = Component<any>;
  export type PanelKey = 'git' | 'files' | 'activity' | 'outputs' | 'canvas' | 'info' | 'browserV1' | 'browserV2' | 'api';
  export const PANEL_LOADERS: Record<PanelKey, () => Promise<{ default: PanelComponent }>> = {
    git: () => import('../modules/git/GitPanel.svelte'),
    files: () => import('../modules/panels/FilesPanel.svelte'),
    activity: () => import('../modules/panels/ActivityPanel.svelte'),
    outputs: () => import('../modules/panels/OutputsPanel.svelte'),
    canvas: () => import('../modules/panels/CanvasPanel.svelte'),
    info: () => import('../modules/panels/InfoPanel.svelte'),
    browserV1: () => import('../modules/panels/BrowserPanel.svelte'),
    browserV2: () => import('../modules/panels/BrowserPanelV2.svelte'),
    api: () => import('../modules/api/ApiPanel.svelte'),
  };
  const panels = new SvelteMap<PanelKey, PanelComponent>();
  const failedPanels = new SvelteSet<PanelKey>();
  const inflight = new Map<PanelKey, Promise<void>>();
  function loadPanel(key: PanelKey): Promise<void> {
    let p = inflight.get(key);
    if (!p) {
      failedPanels.delete(key);
      p = PANEL_LOADERS[key]().then(
        (m) => void panels.set(key, m.default),
        () => {
          inflight.delete(key); // let Retry re-import
          failedPanels.add(key);
        },
      );
      inflight.set(key, p);
    }
    return p;
  }
  /** The chunk a tab renders (null: Notes lives in this file, a page tab is
   *  the page's own component). */
  function panelKeyOf(tab: string | null, browserVersion: string): PanelKey | null {
    if (!tab || tab === 'notes') return null;
    if (tab === 'browser') return browserVersion === 'v1' ? 'browserV1' : 'browserV2';
    return tab in PANEL_LOADERS ? (tab as PanelKey) : null;
  }
  // Warm the persisted tab with this chunk, so an open panel's first tab does
  // not wait for the mount → effect round trip before its import starts.
  {
    const k = uiStore.rightOpenNow ? panelKeyOf(uiStore.rightTabNow, uiStore.browserPanelVersion) : null;
    if (k) void loadPanel(k);
  }
  /** Tabs whose content works on the panel's folder (they get the folder bar). */
  const FOLDER_TABS = new Set(['git', 'files']);
;;// The Side panel (⌘J), on every page. One component in both states: the
// vertical icon rail is always there (VS Code activity bar); clicking an
// icon opens that tab's content beside the rail, clicking the shown tab
// again collapses back to the rail. The content has ONE header (the tab's
// title + expand + collapse) whatever the tab. Which tabs exist and which
// folder Git / Files use comes from stores/rightPanel.svelte.ts: the
// session's cwd in an agent session, a workspace folder anywhere else.


import Icon from '../lib/components/Icon.svelte';
import Skeleton from '../lib/components/Skeleton.svelte';
import FolderPicker from '../lib/components/FolderPicker.svelte';
import { ui } from '../lib/stores/ui.svelte';
import { SIDE_PANEL, railClick } from '../lib/rightTabs';
import { rightPanel, LS_PICKS } from '../lib/stores/rightPanel.svelte';
import { api } from '../lib/api/client';
import type { FsStat } from '../lib/api/types';
import { startMouseDrag } from '../lib/dragCursor';
import { ws } from '../lib/stores/workspace.svelte';
import { git } from '../lib/stores/git.svelte';
import { getToken } from '../lib/api/client';
import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
import { loadErrorText } from '../lib/loadError';
import { onDestroy, untrack } from 'svelte';
;

  // `forceOpen` is set when the panel is hosted inside the mobile right drawer:
  // it then always shows content, fills the drawer width (no fixed width), and
  // drops the desktop drag-resize handle, expand and collapse (the drawer's
  // own ✕ closes it).
  interface Props {
    forceOpen?: boolean;
  }; const lazyPanel/*Ωignore_positionΩ*/ = (key: PanelKey, props: Record<string, unknown> = {})/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const Panel = panels.get(key);
  if(Panel){
     { const $$_lenaP0C = __sveltets_2_ensureComponent(Panel); new $$_lenaP0C({ target: __sveltets_2_any(), props: { ...props,}});}
  } else if (failedPanels.has(key)){
     { svelteHTML.createElement("div", {   "class":`rp-fail`,"role":`alert`,});
       { svelteHTML.createElement("span", {});    }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void loadPanel(key),});  }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`rp-loading`,}); { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {  "rows":4,}});} }
  }
};return __sveltets_2_any(0)};function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { forceOpen = false }: Props = $props();

  // Drag-to-resize: the panel is anchored on the inline-end edge, so dragging
  // its inline-start edge towards the center widens it.
  let resizing = $state(false);
  // Overlay cursor + one width write per frame + one localStorage write on
  // release (lib/dragCursor.ts) — no body.style restyle of the whole app.
  function startResize(e: MouseEvent): void {
    resizing = true;
    const startX = e.clientX;
    const startW = ui.rightWidth;
    const rtl = typeof document !== 'undefined' && getComputedStyle(document.documentElement).direction === 'rtl';
    startMouseDrag(e, {
      onMove: (ev) => ui.setRightWidth(startW + (rtl ? ev.clientX - startX : startX - ev.clientX), false),
      onEnd: () => {
        resizing = false;
        ui.setRightWidth(ui.rightWidth);
      },
    });
  }

  const tabs = $derived(rightPanel.tabs);
  const shown = $derived(rightPanel.shown);
  const shownTab = $derived(tabs.find((t) => t.id === shown) ?? null);
  const pageTab = $derived(rightPanel.pageTabs.find((t) => t.id === shown) ?? null);
  const open = $derived((ui.rightOpenNow || forceOpen) && shown !== null);

  function onRail(id: string): void {
    // In the drawer the content is always showing: a click only switches.
    const next = forceOpen ? { open: true, tab: id } : railClick(ui.rightOpenNow, shown, id);
    ui.setRight(next.open, next.tab as typeof ui.rightTab);
    // An explicit show / hide of a page's own tab is that page's preference.
    rightPanel.pageTabs.find((t) => t.id === id)?.onshow?.(next.open);
  }
  function collapse(): void {
    const page = rightPanel.pageTabs.find((t) => t.id === shown);
    ui.toggleRight();
    page?.onshow?.(false);
  }

  // Closing the panel unmounts (or hides) whatever had focus inside it — the
  // Collapse button, the notes textarea on ⌘J. Hand focus to the rail button
  // of the tab that was showing (DockedDrawer's "refocus the opener" rule), but
  // only when focus was in the panel or fell to <body>: a ⌘J from elsewhere
  // leaves focus where it is.
  let asideEl = $state<HTMLElement | null>(null);
  let wasOpen = false;
  let lastShown: string | null = null;
  $effect.pre(() => {
    const isOpen = open;
    const tab = shown;
    if (wasOpen && !isOpen && !forceOpen) {
      const active = document.activeElement;
      const inPanel = !active || active === document.body || !!asideEl?.contains(active);
      const id = lastShown;
      if (inPanel) {
        queueMicrotask(() => {
          const btn = railEl?.querySelector<HTMLButtonElement>(`.strip-btn[data-tab="${CSS.escape(id ?? '')}"]`)
            ?? railEl?.querySelector<HTMLButtonElement>('.strip-btn');
          btn?.focus();
        });
      }
    }
    wasOpen = isOpen;
    if (isOpen) lastShown = tab;
  });

  // Rail keys: ↑/↓ (Home/End) move focus between the icons; Enter/Space click.
  let railEl = $state<HTMLElement | null>(null);
  function onRailKey(e: KeyboardEvent): void {
    const btns = Array.from(railEl?.querySelectorAll<HTMLButtonElement>('.strip-btn') ?? []);
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    let next = -1;
    if (e.key === 'ArrowDown') next = (i + 1) % btns.length;
    else if (e.key === 'ArrowUp') next = (i - 1 + btns.length) % btns.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = btns.length - 1;
    if (next < 0 || !btns.length) return;
    e.preventDefault();
    btns[next].focus();
  }

  // ── folder bar (Git / Files) ─────────────────────────────────────────────
  const folder = $derived(rightPanel.folder);
  const folderName = $derived(folder.split('/').filter(Boolean).at(-1) ?? folder);
  let pickerOpen = $state(false);

  // A picked folder can vanish (deleted, moved, unmounted): check it once per
  // pick and offer the workspace folder instead of a Git / Files error.
  let pickMissing = $state(false);
  $effect(() => {
    const path = rightPanel.picked ? folder : '';
    pickMissing = false;
    if (!path) return;
    let live = true;
    api
      .get<FsStat>(`/fs/stat?path=${encodeURIComponent(path)}`)
      .then((st) => { if (live) pickMissing = !st.is_dir; })
      .catch((e: unknown) => { if (live) pickMissing = (e as { status?: number })?.status === 404; })
      // Spoken at the edge (the inline "Not found" is inserted with its text).
      .finally(() => { if (live && pickMissing) announce('The picked folder was not found.'); });
    return () => { live = false; };
  });
  // Picks of workspaces that no longer exist are dropped; another window's
  // pick is adopted as it happens.
  $effect(() => {
    const ids = ws.workspaces.map((w) => w.id);
    if (ids.length) untrack(() => rightPanel.prunePicks(ids));
  });
  $effect(() => {
    const onStorage = (e: StorageEvent): void => {
      if (e.storageArea === localStorage && e.key === LS_PICKS) rightPanel.reloadPicks();
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  });
  function folderMenu(e: MouseEvent): void {
    const root = ws.current?.root_path ?? '';
    const items: MenuItem[] = [
      {
        label: root ? `Workspace folder · ${root.split('/').filter(Boolean).at(-1) ?? root}` : 'Workspace folder',
        icon: 'folder',
        checked: !rightPanel.picked,
        disabled: !root,
        title: root || 'This workspace has no folder. Choose one below.',
        pinned: true,
        action: () => rightPanel.pickFolder(null),
      },
      { label: 'Choose folder…', icon: 'folder', pinned: true, action: () => (pickerOpen = true) },
    ];
    const repos = git.repos.filter((r) => r.path && r.path !== root);
    if (repos.length) {
      items.push({ separator: true, pinned: true });
      for (const r of repos) {
        items.push({ label: r.name, icon: 'branch', hint: 'repository', title: r.path, checked: folder === r.path, action: () => rightPanel.pickFolder(r.path) });
      }
    }
    const others = ws.workspaces.filter((w) => w.id !== ws.currentId && w.root_path);
    if (others.length) {
      items.push({ separator: true, pinned: true });
      for (const w of others) {
        items.push({ label: w.name, icon: 'folder', hint: 'workspace', title: w.root_path, checked: folder === w.root_path, action: () => rightPanel.pickFolder(w.root_path) });
      }
    }
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { filter: repos.length + others.length > 8, filterPlaceholder: 'Find a repository or workspace' });
  }

  // ── notes ────────────────────────────────────────────────────────────────
  let notes = $state('');
  let notesLoadedFor: string | null = $state(null);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveState: 'idle' | 'saving' | 'saved' | 'error' = $state('idle');
  let notesError = $state('');
  let notesRevision = 0;
  let notesQueue: Promise<void> = Promise.resolve();

  /** Text typed but not yet saved, bound to the workspace it was typed in:
   *  the debounce used to save whatever workspace was current WHEN IT FIRED,
   *  so switching within 600 ms wrote B's text back to B and lost A's edit. */
  let pendingNotes: { wsId: string; text: string; token: string | null } | null = null;

  $effect(() => {
    // (re)load notes when workspace changes
    const w = ws.current;
    if (w && notesLoadedFor !== w.id) {
      untrack(() => void flushNotes());
      notesLoadedFor = w.id;
      notes = typeof w.settings?.notes === 'string' ? (w.settings.notes as string) : '';
      notesRevision++;
      notesError = '';
      saveState = 'idle';
    }
  });

  async function flushNotes(): Promise<void> {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    const p = pendingNotes;
    pendingNotes = null;
    if (!p || p.token !== getToken()) return;
    const revision = notesRevision;
    saveState = 'saving';
    notesError = '';
    // Serialize writes so an older, slow save cannot overwrite newer text.
    const saving = notesQueue.then(() => {
      if (p.token === getToken()) return ws.saveNotes(p.text, p.wsId);
    });
    notesQueue = saving.catch(() => {});
    try {
      await saving;
      if (p.token !== getToken() || p.wsId !== notesLoadedFor || revision !== notesRevision) return;
      saveState = 'saved';
    } catch (e) {
      if (p.token !== getToken()) return;
      if (p.wsId !== notesLoadedFor) {
        toastError('Couldn’t save the notes', e);
        return;
      }
      if (revision !== notesRevision) return;
      pendingNotes = p;
      notesError = loadErrorText(e);
      saveState = 'error';
    }
  }

  function onNotesInput(): void {
    if (!notesLoadedFor) return;
    if (saveTimer) clearTimeout(saveTimer);
    pendingNotes = { wsId: notesLoadedFor, text: notes, token: getToken() };
    notesRevision++;
    notesError = '';
    saveState = 'saving';
    saveTimer = setTimeout(() => void flushNotes(), 600);
  }

  onDestroy(() => void flushNotes());

  // Keep-alive for the v1 Browser: it holds live pages (native webviews),
  // several tabs and unsent take-over annotations in component state, so
  // unmounting it on every tab switch or ⌘J collapse threw all of that away.
  // Once opened it stays mounted (hidden) until the panel itself goes away;
  // every other tab still mounts on demand (their state lives in stores).
  let browserKept = $state(false);
  $effect(() => {
    if (open && shown === 'browser' && ui.browserPanelVersion === 'v1') browserKept = true;
  });
  // Leaving the session drops the v1 tab from the rail: release its webviews.
  $effect(() => {
    if (browserKept && !tabs.some((t) => t.id === 'browser')) browserKept = false;
  });
  const browserShown = $derived(open && shown === 'browser');

  // Load the visible tab's chunk, and the kept-alive v1 Browser's.
  $effect(() => {
    const k = open ? panelKeyOf(shown, ui.browserPanelVersion) : null;
    if (k && !panels.has(k)) void loadPanel(k);
  });
;
async () => {






if(pickerOpen){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Folder for the ${SIDE_PANEL}`,"start":folder,"onpick":(p) => {
      pickerOpen = false;
      rightPanel.pickFolder(p);
    },"onclose":() => (pickerOpen = false),}});}
}

 { svelteHTML.createElement("div", {  "class":`rside`,});forceOpen;
  if(open || browserKept){
     { const $$_aside1 = svelteHTML.createElement("aside", {              "class":`rpanel`,"id":`side-panel-content`,"hidden":!open,"aria-label":shownTab ? `${shownTab.label} · ${SIDE_PANEL}` : SIDE_PANEL,"style":forceOpen ? undefined : `width:${ui.rightWidth}px`,});asideEl = $$_aside1;resizing;forceOpen;
      if(!forceOpen){
        
         {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: ui.rightWidth, min: RIGHT_MIN, max: Math.max(RIGHT_MIN, Math.min(RIGHT_MAX, (typeof window === 'undefined' ? RIGHT_MAX : window.innerWidth) - 360)), invert: true, onChange: (w) => ui.setRightWidth(w), onReset: () => ui.setRightWidth(300), onDragStart: startResize, text: pxWide })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`resize-handle`,"role":`separator`,"aria-label":`Resize the ${SIDE_PANEL}`,"title":RESIZE_TITLE,}); }}
      }
       { svelteHTML.createElement("header", { "class":`rpanel-head`,});
        if(shownTab){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":shownTab.icon,"size":13,}});}}
         { svelteHTML.createElement("h2", { "class":`rpanel-title`,});shownTab?.label ?? SIDE_PANEL; }
        if(!forceOpen){
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => ui.toggleRightWide(),"title":ui.rightWide ? 'Restore panel width' : 'Expand panel',"aria-label":ui.rightWide ? 'Restore panel width' : 'Expand panel',});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":ui.rightWide ? 'minimize' : 'maximize',"size":13,}});}
           }
           { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":collapse,"title":`Collapse panel`,"aria-keyshortcuts":`Meta+J`,"aria-label":`Collapse panel`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`panel`,"size":13,}});}
           }
        }
       }

      if(shown && FOLDER_TABS.has(shown)){
        
         { svelteHTML.createElement("div", { "class":`rpanel-folder`,});
          if(rightPanel.session){
             { svelteHTML.createElement("span", {   "class":`folder-label`,"title":folder,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
               { svelteHTML.createElement("span", { "class":`folder-name`,});folderName || 'No folder'; }
               { svelteHTML.createElement("span", { "class":`dim folder-kind`,});  }
             }
          }else{
             { svelteHTML.createElement("button", {         "class":`folder-btn`,"title":folder ? `${folder} · change the folder` : 'Choose a folder',"aria-haspopup":`menu`,"aria-label":`Folder: ${folderName || 'none'}. Change folder`,"onclick":folderMenu,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
               { svelteHTML.createElement("span", { "class":`folder-name`,});folderName || 'Choose a folder'; }
              if(folder){ { svelteHTML.createElement("span", { "class":`dim folder-kind`,});rightPanel.picked ? 'picked' : 'workspace'; }}
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
             }
            if(pickMissing){
               { svelteHTML.createElement("span", { "class":`folder-missing`,});
                 { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}  
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => rightPanel.pickFolder(null),});   }
               }
            }
          }
         }
      }

       { svelteHTML.createElement("div", { "class":`rpanel-body`,});
        if(pageTab){
          const PagePanel = pageTab.component;
           { const $$_lenaPegaP3C = __sveltets_2_ensureComponent(PagePanel); new $$_lenaPegaP3C({ target: __sveltets_2_any(), props: { ...pageTab.props ?? {},}});}
        } else if (shown === 'git' || shown === 'files' || shown === 'activity' || shown === 'outputs' || shown === 'canvas' || shown === 'info'){
          ;__sveltets_2_ensureSnippet(lazyPanel(shown as PanelKey));
        }
        if(browserShown || (browserKept && ui.browserPanelVersion === 'v1')){
          
           { svelteHTML.createElement("div", {   "class":`browser-host`,"hidden":!browserShown,});
            if(ui.browserPanelVersion === 'v2'){
              ;__sveltets_2_ensureSnippet(lazyPanel('browserV2'));
            }else{
              ;__sveltets_2_ensureSnippet(lazyPanel('browserV1', { active: browserShown }));
            }
           }
        }
        if(shown === 'api'){
          ;__sveltets_2_ensureSnippet(lazyPanel('api'));
        } else if (shown === 'notes'){
           { svelteHTML.createElement("div", { "class":`notes-wrap`,});
             { svelteHTML.createElement("textarea", {              "dir":`auto`,"class":`notes`,"aria-label":`Workspace notes`,"bind:value":notes,"oninput":onNotesInput,"placeholder":`Workspace notes (markdown)…`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => notes = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
            
             { svelteHTML.createElement("div", { "class":`notes-foot`,});
               { svelteHTML.createElement("div", {   "class":`notes-alert`,"role":`alert`,});
                if(saveState === 'error'){
                   { svelteHTML.createElement("div", { "class":`notes-error`,}); { svelteHTML.createElement("span", {});   notesError; }
                     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void flushNotes(),});  }
                   }
                }
               }
              if(saveState === 'saving'){ { svelteHTML.createElement("span", { "class":`dim`,});  }
              } else if (saveState === 'saved'){ { svelteHTML.createElement("span", { "class":`dim`,});  }
              } else if (saveState !== 'error'){ { svelteHTML.createElement("span", { "class":`dim`,});       }}
             }
           }
        }
       }
     }
  }

  
   { const $$_div1 = svelteHTML.createElement("div", {            "class":`rstrip`,"role":`toolbar`,"aria-orientation":`vertical`,"tabindex":-1,"aria-label":SIDE_PANEL,"onkeydown":onRailKey,});railEl = $$_div1;
        for(let t of __sveltets_2_ensureArray(tabs)){let i = 1;t.id;
      if(i > 0 && tabs[i - 1].page && !t.page){ { svelteHTML.createElement("span", {   "class":`strip-sep`,"aria-hidden":`true`,}); }}
       { svelteHTML.createElement("button", {                "class":`icon-btn strip-btn`,...__sveltets_2_empty({"data-tab":t.id}),"onclick":() => onRail(t.id),"title":open && shown === t.id && !forceOpen ? `${t.label} · click to collapse` : t.label,"aria-label":t.label,"aria-pressed":open && shown === t.id,"aria-controls":open ? 'side-panel-content' : undefined,});open && shown === t.id;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":t.icon,"size":14,}});}
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RightPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RightPanel__SvelteComponent_ = ReturnType<typeof RightPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default RightPanel__SvelteComponent_;