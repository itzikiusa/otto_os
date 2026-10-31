///<reference types="svelte" />
;
import { loadErrorText } from '../../lib/loadError';
import { announce } from '../../lib/announce.svelte';
// Vault v3 — the docs home. Obsidian-style three-pane layout: left sidebar (Files / Search / Tags
// over the active vault), center (note edit⇄read or the graph), and the note's details (backlinks /
// outgoing / outline / properties / OKF) as a "Note details" tab of the shell's Side panel — one
// right panel, not two. Files on disk are the truth; the daemon keeps a derived index.
import { onMount, untrack } from 'svelte';
import { focusOnMount } from '../../lib/focusOnMount';
import Icon from '../../lib/components/Icon.svelte';
import { toasts } from '../../lib/toast.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { lsGet, lsSet } from '../../lib/storage';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { rightPanel } from '../../lib/stores/rightPanel.svelte';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import DocsAgentsView from './DocsAgentsView.svelte';
import FileTree from './FileTree.svelte';
import FileViewer from './FileViewer.svelte';
import GraphView from './GraphView.svelte';
import RecoveryView from './RecoveryView.svelte';
import NewNoteDialog from './NewNoteDialog.svelte';
import NoteView from './NoteView.svelte';
import RightPanel from './RightPanel.svelte';
import SearchPanel from './SearchPanel.svelte';
import Switcher from './Switcher.svelte';
import TagsPanel from './TagsPanel.svelte';
import { vault } from './vault.svelte';
import { registerVaultPort } from '../../lib/uiCommands/vaultPort';
import PathField from '../../lib/components/PathField.svelte';
import Modal from '../../lib/components/Modal.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // -- pane widths (drag-resizable, persisted) ---------------------------------
  const LEFT_W_KEY = 'otto_vault_left_w';
  // Storage is a convenience cache: lsGet/lsSet swallow a blocked accessor or
  // a full quota, so a private window never blanks the page.
  let leftW = $state(loadPaneWidth(LEFT_W_KEY, LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  // Note details live in the Side panel, as its "Note details" tab. The
  // Vault's own "show details" preference (default on) is kept SEPARATE from
  // the panel's saved state: it is applied as the panel's transient view
  // (ui.rightTransient — never persisted, gone when the Vault unmounts), and
  // written only by an explicit show / hide of Note details (the ⋯ menu, its
  // rail icon, collapsing while it is shown). Visiting the Vault therefore
  // never leaves the panel open on other pages.
  const DETAILS_TAB = 'page:vault-details' as const;
  const DETAILS_KEY = 'otto_vault_right_open';
  const rightOpen = $derived(ui.rightOpenNow && rightPanel.shown === DETAILS_TAB);
  // a11y-6-01: a backlinks refresh failure is announced at its edge (the status-bar glyph is no live region).
  const backlinksError = $derived(vault.note ? (vault.backlinksError ?? '') : '');
  $effect(() => { if (backlinksError) announce(`Couldn’t refresh the backlinks. ${backlinksError}`); });
  $effect(() => registerVaultPort(vault)); // otto.ui_vault_* drive the store while the page is up (lib/uiCommands/vaultPort.ts)
  function rememberDetails(shown: boolean): void {
    lsSet(DETAILS_KEY, shown ? '1' : '0');
  }
  let leftOpen = $state(lsGet('otto_vault_left_open') !== '0');

  function toggleLeft(): void {
    leftOpen = !leftOpen;
    lsSet('otto_vault_left_open', leftOpen ? '1' : '0');
  }
  function toggleRight(): void {
    const show = !rightOpen;
    ui.rightTransient = { open: show, tab: DETAILS_TAB };
    rememberDetails(show);
  }
  $effect(() => {
    if (vault.centerMode !== 'note') return;
    // untrack: register reads + writes the page-tab list (no self-dependency).
    const off = untrack(() => rightPanel.register({ id: DETAILS_TAB, label: 'Note details', icon: 'link', component: RightPanel, onshow: rememberDetails }));
    untrack(() => {
      if (lsGet(DETAILS_KEY) !== '0' && !rightOpen) ui.rightTransient = { open: true, tab: DETAILS_TAB };
    });
    return () =>
      untrack(() => {
        off();
        if (ui.rightTransient?.tab === DETAILS_TAB) ui.rightTransient = null;
      });
  });

  // -- create-vault dialog -------------------------------------------------------
  let createOpen = $state(false);
  let cName = $state('');
  let cPath = $state('');
  let cOkf = $state(true);
  let creating = $state(false);
  let createError = $state('');

  async function submitCreate(): Promise<void> {
    if (!cName.trim() || creating) return;
    creating = true;
    createError = '';
    try {
      await vault.create(cName.trim(), cPath.trim() || undefined, cOkf);
      createOpen = false;
      cName = '';
      cPath = '';
    } catch (e) {
      // Surface the daemon's reason inline — a silently-failing dialog is the
      // worst kind of "did nothing".
      createError = loadErrorText(e);
    } finally {
      creating = false;
    }
  }

  // -- new-note dialog -------------------------------------------------------------
  let newNoteOpen = $state(false);
  let newNoteDir = $state('');

  function openNewNote(dir: string): void {
    newNoteDir = dir;
    newNoteOpen = true;
  }

  function vaultMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      ...vault.vaults.map((v) => ({
        label: v.name,
        checked: v.id === vault.current?.id,
        action: () => void vault.select(v.id),
      })),
      { separator: true },
      { label: 'Add vault…', icon: 'plus', action: () => (createOpen = true) },
      ...(vault.current
        ? [
            {
              label: 'OKF mode (validation + templates)',
              checked: vault.current.okf,
              action: () => void vault.toggleOkf(),
            },
            { label: 'Rescan', icon: 'refresh', action: () => void vault.rescan() },
            {
              // Reversible (the files, their edit history and trash all live
              // in the folder; re-adding it rebuilds the index), so no confirm
              // — the toast says what happened and how to undo it.
              label: 'Unregister vault',
              icon: 'x',
              action: () => {
                if (vault.current) void vault.unregister(vault.current.id);
              },
            },
          ]
        : []),
    ]);
  }

  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'o') {
      e.preventDefault();
      vault.switcherOpen = true;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'n' && vault.current) {
      e.preventDefault();
      openNewNote('');
    }
  }

  // ⌘K: the page's verbs (the same ones the toolbar and ⌘O / ⌘N offer).
  $effect(() => {
    const hasVault = !!vault.current;
    return registry.register('vault', hasVault
      ? [
          { id: 'vault.new-note', title: 'New note…', group: 'Vault', keywords: 'create markdown doc page', shortcut: '⌘N', run: () => openNewNote('') },
          { id: 'vault.open-note', title: 'Open note…', group: 'Vault', keywords: 'quick switcher jump find file', shortcut: '⌘O', run: () => (vault.switcherOpen = true) },
        ]
      : []);
  });

  /** Header ⋯: the less-used views (history, trash) and the pane toggles —
   *  the toolbar keeps ≤5 controls (graph, docs agent, switcher, new note, ⋯). */
  function moreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Edit history', icon: 'clock', action: () => void vault.openHistory() },
      { label: 'Trash and restore', icon: 'trash', action: () => void vault.openTrash() },
      { separator: true },
      { label: 'File tree', checked: leftOpen, action: toggleLeft },
      ...(vault.centerMode === 'note' ? [{ label: 'Note details', checked: rightOpen, hint: '⌘J', action: toggleRight }] : []),
    ];
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }

  function toggleGraph(): void {
    vault.centerMode = vault.centerMode === 'graph' ? (vault.note ? 'note' : 'empty') : 'graph';
    vault.persistView();
  }
  function toggleDocsAgents(): void {
    if (vault.centerMode === 'docs-agents') {
      vault.centerMode = vault.note ? 'note' : 'empty';
      vault.persistView();
    } else {
      vault.openDocsAgents('');
    }
  }

  // -- landing note ----------------------------------------------------------------
  // A vault never opens onto a bare "open a note" pane: on a first visit (no
  // saved tabs/view for this vault) its root index/README note opens by itself.
  // Once per vault per mount, so closing it lands on the empty state instead.
  const INDEX_NAMES = ['index.md', 'readme.md', '_index.md'];
  const indexNote = $derived.by(() => {
    for (const name of INDEX_NAMES) {
      const hit = vault.roots.find((r) => r.entry.kind === 'note' && r.entry.name.toLowerCase() === name);
      if (hit) return hit.entry.path;
    }
    return null;
  });
  let phoneShowSidebar = $state(false);
  const phoneInSidebar = $derived(
    viewport.isPhone && (phoneShowSidebar || vault.centerMode === 'empty'),
  );
  $effect(() => {
    void vault.note;
    void vault.filePath;
    void vault.activeTab;
    const mode = vault.centerMode;
    untrack(() => {
      if (mode !== 'empty') phoneShowSidebar = false;
    });
  });

  let autoIndexFor: number | null = null;
  $effect(() => {
    if (viewport.isPhone) return;
    const id = vault.current?.id;
    const path = indexNote;
    if (id == null || !path || autoIndexFor === id) return;
    if (vault.centerMode !== 'empty' || vault.tabs.length > 0) return;
    // A saved view (tabs, or graph/docs-agents/trash) is restored by the
    // store — don't race it.
    let saved: { tabs?: unknown; mode?: unknown } = {};
    try {
      saved = JSON.parse(lsGet(`otto_vault_tabs:${id}`) ?? '{}') ?? {};
    } catch {
      /* corrupt blob — treat as a first visit */
    }
    const hasTabs = Array.isArray(saved.tabs) && saved.tabs.length > 0;
    const mode = typeof saved.mode === 'string' ? saved.mode : 'empty';
    if (hasTabs || (mode !== 'empty' && mode !== 'note')) return;
    autoIndexFor = id;
    void vault.open(path);
  });

  const scanning = $derived(vault.status?.scan_state === 'scanning');
  const scanError = $derived(vault.status?.scan_state.startsWith('error') ?? false);
  const trackedRecovery = $derived(vault.status?.tracked_recovery ?? []);
  function explainTrackedRecovery(): void {
    const cmd = `git rm --cached -r ${trackedRecovery.join(' ')}`;
    toasts.info(
      'Note history is tracked by git',
      `Otto now ignores ${trackedRecovery.join(' and ')}, but files committed earlier are still in the repository. ` +
        `In the vault folder run:\n${cmd}\nthen commit. Earlier commits keep their copies until the history is rewritten.`,
    );
  }

  // The first load comes from the workspace effect below (it runs on mount
  // too) — loading here as well fetched every vault request twice.
  onMount(() => {
    vault.startPolling();
    return () => vault.stopPolling();
  });

  // Leaving the Vault lands a pending autosave first (never asks: the vault
  // auto-saves and keeps a local draft). Moves inside the Vault are the
  // store's own business (it saves before switching notes).
  $effect(() => router.guard((to) => (to === 'vault' || to.startsWith('vault/') ? true : vault.flushBeforeLeave())));

  // Reload when the workspace changes.
  let lastWs = $state('');
  $effect(() => {
    const id = ws.current?.id ?? '';
    if (id && id !== lastWs) {
      lastWs = id;
      void vault.load();
    }
  });
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKeydown,});}

 { svelteHTML.createElement("div", { "class":`vault-page`,});
  
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":vault.current?.name ?? 'Vault',"class":`vault-header`,leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if(viewport.isPhone && !phoneInSidebar && vault.current){
         { svelteHTML.createElement("button", {         "type":`button`,"class":`btn ghost small phone-back-btn`,"onclick":() => (phoneShowSidebar = true),"aria-label":`Back to vault sidebar`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
           { svelteHTML.createElement("span", {});vault.leftMode === 'search' ? 'Search' : vault.leftMode === 'tags' ? 'Tags' : 'Files'; }
         }
      }
    };return __sveltets_2_any(0)},titleContent:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`vault-pick`,"onclick":(e) => vaultMenu(e),"title":vault.current ? `${vault.current.name} — switch vault` : 'Switch vault',"aria-haspopup":`menu`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`book`,"size":14,}});}
         { svelteHTML.createElement("span", { "class":`vp-name`,});vault.current?.name ?? 'No vault'; }
         { svelteHTML.createElement("span", { "class":`tri`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
       }
    };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
      if(vault.current?.okf){
         { svelteHTML.createElement("span", {   "class":`okf-chip`,"title":`OKF (Open Knowledge Format) vault`,});  }
      }
      if(scanning){
         { svelteHTML.createElement("span", { "class":`scan-chip`,});  }
      } else if (scanError){
         { svelteHTML.createElement("button", {     "class":`scan-chip err`,"title":`${vault.status?.scan_state ?? ''} — click to rescan`,"onclick":() => void vault.rescan(),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}    
         }
      }
      if((vault.status?.tracked_recovery?.length ?? 0) > 0){
        
         { svelteHTML.createElement("button", {       "class":`scan-chip err`,"title":`Git already tracks ${trackedRecovery.join(' and ')} (private note history). Click for the command that stops tracking it.`,"onclick":explainTrackedRecovery,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}     
         }
      }
      if(vault.activeDocsRuns.length > 0){
        
         { svelteHTML.createElement("button", {       "class":`run-chip`,"title":vault.activeDocsRuns
            .map((r) => (r.kind === 'refine' ? `refine: ${r.note_path}` : r.prompt))
            .join('\n'),"onclick":() => vault.openDocsAgents(''),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
          vault.activeDocsRuns.length === 1
            ? '1 agent run active'
            : `${vault.activeDocsRuns.length} agent runs active`;
         }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(vault.current){
         { svelteHTML.createElement("button", {              "class":`icon-btn vh-tool`,"aria-pressed":vault.centerMode === 'graph',"title":`Graph view`,"aria-label":`Graph view`,...__sveltets_2_empty({"data-icon":`share`}),"onclick":toggleGraph,});vault.centerMode === 'graph';
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`share`,"size":14,}});}
         }
         { svelteHTML.createElement("button", {                "class":`icon-btn vh-tool`,"aria-pressed":vault.centerMode === 'docs-agents',"title":`Docs agent — have agents write documentation into this vault`,"aria-label":`Docs agent`,...__sveltets_2_empty({"data-label":`Docs agent`}),...__sveltets_2_empty({"data-icon":`zap`}),"onclick":toggleDocsAgents,});vault.centerMode === 'docs-agents';
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":14,}});}
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn vh-tool`,"title":`Quick switcher (⌘O)`,"aria-label":`Quick switcher`,...__sveltets_2_empty({"data-icon":`search`}),"onclick":() => (vault.switcherOpen = true),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":14,}});}
         }
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"title":`New note (⌘N)`,...__sveltets_2_empty({"data-icon":`plus`}),"onclick":() => openNewNote(''),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}  
         }
        
         { svelteHTML.createElement("button", {                 "class":`icon-btn vh-tool`,"title":`More vault actions`,"aria-label":`More vault actions`,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-overflow":`-10`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More vault actions…`}),"onclick":moreMenu,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, titleContent, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
    
    
   PageHeader}

  if(!vault.current && vault.loadError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`vaults`,"variant":`page`,"loading":vault.loading,"error":vault.loadError,"empty":true,"onretry":() => void vault.load(),}});}
  } else if (!vault.current && vault.loading){
      { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {    "what":`vaults`,"variant":`page`,"loading":true,"empty":true,}});}
  } else if (!vault.current && !vault.loading){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`note`,"title":`The docs home`,"body":`A vault is a folder of markdown files on disk — point Otto at an existing Obsidian vault or create a fresh one. Files stay yours; Otto indexes links, tags and full text, and agents read/write it over MCP in OKF.`,"actionLabel":`Add a vault`,"actionIcon":`plus`,"onaction":() => (createOpen = true),}});}
  } else if (vault.current){
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
     { svelteHTML.createElement("div", { "class":`panes`,});
      if(phoneInSidebar || (!viewport.isPhone && leftOpen)){
       { svelteHTML.createElement("aside", {   "class":`left`,"style":`width:${leftW}px`,});
         { svelteHTML.createElement("div", {     "class":`left-modes`,"role":`group`,"aria-label":`Sidebar view`,});
           { svelteHTML.createElement("button", {           "class":`icon-btn vh-tool`,"aria-pressed":vault.leftMode === 'files',"title":`Files`,"aria-label":`Files`,"onclick":() => (vault.leftMode = 'files'),});vault.leftMode === 'files'; { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":14,}});} }
           { svelteHTML.createElement("button", {           "class":`icon-btn vh-tool`,"aria-pressed":vault.leftMode === 'search',"title":`Search`,"aria-label":`Search`,"onclick":() => (vault.leftMode = 'search'),});vault.leftMode === 'search'; { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":14,}});} }
           { svelteHTML.createElement("button", {           "class":`icon-btn vh-tool`,"aria-pressed":vault.leftMode === 'tags',"title":`Tags`,"aria-label":`Tags`,"onclick":() => {
              vault.leftMode = 'tags';
              void vault.loadTags();
            },});vault.leftMode === 'tags'; { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":14,}});} }
           { svelteHTML.createElement("div", { "class":`spacer`,}); }
          if(vault.leftMode === 'files'){
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Collapse all`,"aria-label":`Collapse all`,"onclick":() => vault.collapseAll(),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`minimize`,"size":13,}});}
             }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Rescan vault`,"aria-label":`Rescan vault`,"onclick":() => void vault.rescan(),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
             }
          }
         }
        if(vault.leftMode === 'files'){
           { const $$_eerTeliF4C = __sveltets_2_ensureComponent(FileTree); new $$_eerTeliF4C({ target: __sveltets_2_any(), props: {  "onNewNote":openNewNote,}});}
        } else if (vault.leftMode === 'search'){
           { const $$_lenaPhcraeS4C = __sveltets_2_ensureComponent(SearchPanel); new $$_lenaPhcraeS4C({ target: __sveltets_2_any(), props: {}});}
        }else{
           { const $$_lenaPsgaT4C = __sveltets_2_ensureComponent(TagsPanel); new $$_lenaPsgaT4C({ target: __sveltets_2_any(), props: {}});}
        }
       }
      if(!viewport.isPhone){
         { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:leftW,"storageKey":LEFT_W_KEY,"label":`Resize sidebar`,}});/*Ωignore_startΩ*/() => leftW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
      }
      }

      if(!phoneInSidebar){
       { svelteHTML.createElement("main", { "class":`center`,});
        if(vault.noteOpenError){
           { svelteHTML.createElement("div", {     "class":`note-open-error`,"role":`alert`,...__sveltets_2_empty({"data-testid":`note-open-error`}),});
             { svelteHTML.createElement("span", {});   { svelteHTML.createElement("bdi", {});vault.noteOpenError.path; } vault.noteOpenError.message; }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { const failed = vault.noteOpenError; if (failed) void vault.open(failed.path, failed.opts); },});  }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (vault.noteOpenError = null),});  }
           }
        }
        if(vault.tabs.length > 0){
          
           { svelteHTML.createElement("div", {     "class":`tabstrip scroll-thin`,"role":`tablist`,"aria-label":`Open notes`,});
                for(let t of __sveltets_2_ensureArray(vault.tabs)){let i = 1;t.kind + ':' + t.path;
              const tabName = t.kind === 'note'
                ? (t.path.split('/').pop() ?? t.path).replace(/\.md$/i, '')
                : (t.path.split('/').pop() ?? t.path);
               { svelteHTML.createElement("div", {      "class":`vtab`,"role":`presentation`,});i === vault.activeTab &&
                  (vault.centerMode === 'note' || vault.centerMode === 'file');
                 { svelteHTML.createElement("button", {                 "class":`vtab-main`,"role":`tab`,"aria-selected":i === vault.activeTab,"tabindex":i === vault.activeTab ? 0 : -1,"title":t.path,"onkeydown":onTabKey,"onclick":() => void vault.activateTab(i),"onauxclick":(e) => {
                    if (e.button === 1) void vault.closeTab(i);
                  },});
                   { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":t.kind === 'note' ? 'note' : 'file',"size":12,}});}
                   { svelteHTML.createElement("span", { "class":`vtab-name`,});tabName; }
                 }
                 { svelteHTML.createElement("button", {        "class":`vtab-close`,"title":`Close tab`,"aria-label":`Close ${tabName}`,"onclick":() => void vault.closeTab(i),}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
               }
            }
           }
        }
        if(vault.centerMode === 'graph'){
           { svelteHTML.createElement("div", { "class":`graph-scope`,});
             { svelteHTML.createElement("label", {});   { svelteHTML.createElement("select", {     "aria-label":`Graph scope`,"bind:value":vault.graphLocal,"onchange":() => vault.persistView(),});/*Ωignore_startΩ*/() => vault.graphLocal = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":false,});  } { svelteHTML.createElement("option", {   "value":true,"disabled":!vault.notePath,});   }
             } }
            if(vault.graphLocal){ { svelteHTML.createElement("span", {   "class":`gs-path`,"title":vault.notePath ?? '',});vault.notePath; }}
           }
           { const $$_weiVhparG4C = __sveltets_2_ensureComponent(GraphView); new $$_weiVhparG4C({ target: __sveltets_2_any(), props: {    "local":vault.graphLocal,"onNewNote":() => openNewNote(''),}});}
        } else if (vault.centerMode === 'trash' || vault.centerMode === 'history'){
          vault.centerMode; { { const $$_weiVyrevoceR4C = __sveltets_2_ensureComponent(RecoveryView); new $$_weiVyrevoceR4C({ target: __sveltets_2_any(), props: {  "mode":vault.centerMode,}});}}
        } else if (vault.centerMode === 'docs-agents'){
           { const $$_weiVstnegAscoD4C = __sveltets_2_ensureComponent(DocsAgentsView); new $$_weiVstnegAscoD4C({ target: __sveltets_2_any(), props: {}});}
        } else if (vault.centerMode === 'file' && vault.filePath){
           { const $$_reweiVeliF4C = __sveltets_2_ensureComponent(FileViewer); new $$_reweiVeliF4C({ target: __sveltets_2_any(), props: {}});}
        } else if (vault.centerMode === 'note' && vault.note){
           { const $$_weiVetoN4C = __sveltets_2_ensureComponent(NoteView); new $$_weiVetoN4C({ target: __sveltets_2_any(), props: {}});}
        }else{
          const emptyVault = (vault.status?.notes ?? vault.current.notes) === 0;
           { svelteHTML.createElement("div", { "class":`center-empty`,});
             { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"icon":`note`,"title":emptyVault ? 'This vault is empty' : 'No note open',"body":emptyVault
                ? 'Start a new markdown note or run the Docs agent to populate this vault.'
                : 'Open a note from the file tree or search, press ⌘O to jump to one, or start a new note.',"actionLabel":`New note`,"actionIcon":`plus`,"onaction":() => openNewNote(''),}});
              if(indexNote){
                 { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => void vault.open(indexNote!),}); indexNote; }
              } else if (!emptyVault){
                 { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (vault.switcherOpen = true),});   }
              }
             EmptyState}
           }
        }
       }
      }

     }

     { svelteHTML.createElement("footer", { "class":`vault-statusbar`,});
       { svelteHTML.createElement("span", { "class":`vs-counts`,});
        vault.status?.notes ?? vault.current.notes;   vault.status?.links ?? vault.current.links; if((vault.status?.unresolved ?? 0) > 0){' · ';vault.status?.unresolved; }
       }
      if(vault.note){
         { svelteHTML.createElement("span", {});
          vault.backlinks.length; if(vault.backlinksLoading){ { svelteHTML.createElement("span", {     "class":`spinner vs-inline`,"role":`img`,"aria-label":`Loading backlinks`,}); }} else if (vault.backlinksError){ { svelteHTML.createElement("span", {   "class":`vs-warn`,"title":vault.backlinksError,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}   }}
         }
         { svelteHTML.createElement("span", {});vault.note.meta.word_count;  }
         { svelteHTML.createElement("span", {});(vault.editing ? vault.draft : vault.note.raw).length;  }
        if(vault.current.okf && vault.okfReport){
           { svelteHTML.createElement("span", {    "title":vault.okfReport.conformant ? 'This note passes OKF validation' : 'See the OKF section in Note details (Side panel)',});vault.okfReport.conformant;!vault.okfReport.conformant;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":vault.okfReport.conformant ? 'check' : 'warning',"size":12,}});}
            vault.okfReport.conformant ? 'OKF valid' : `OKF: ${vault.okfReport.errors.length} ${vault.okfReport.errors.length === 1 ? 'issue' : 'issues'}`;
           }
        }
      }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("span", {   "class":`dim vs-path`,"title":vault.current.root_path,});vault.current.root_path; }
     }
     PageBody}
  }
 }

if(createOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":`Add a vault`,"onclose":() => (createOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (createOpen = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!cName.trim() || creating,"onclick":() => void submitCreate(),});
        creating ? 'Adding…' : 'Add vault';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("form", {   "class":`av-body`,"onsubmit":(e) => { e.preventDefault(); void submitCreate(); },});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`av-name`,});  }
          {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {          "dir":`auto`,"id":`av-name`,"class":`input`,"bind:value":cName,"placeholder":`Team Docs`,});/*Ωignore_startΩ*/() => cName = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`av-path`,});  }
         { const $$_dleiFhtaP3C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP3 = new $$_dleiFhtaP3C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },value:cPath,"start":cPath || '~',}});/*Ωignore_startΩ*/() => cPath = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP3.$$bindings = 'value';
           { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`av-path`,"class":`input`,"bind:value":cPath,"placeholder":`~/Documents/Obsidian/MyVault`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => cPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         PathField}
         { svelteHTML.createElement("span", { "class":`hint`,});                       }
       }
       { svelteHTML.createElement("label", { "class":`checkbox-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":cOkf,});/*Ωignore_startΩ*/() => cOkf = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               
       }
      if(createError){
         { svelteHTML.createElement("div", {   "class":`av-err`,"role":`alert`,});    createError; }
      }
     }
    
   Modal}
}

 { const $$_golaiDetoNweN0C = __sveltets_2_ensureComponent(NewNoteDialog); const $$_golaiDetoNweN0 = new $$_golaiDetoNweN0C({ target: __sveltets_2_any(), props: {     open:newNoteOpen,dir:newNoteDir,}});/*Ωignore_startΩ*/() => newNoteOpen = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_golaiDetoNweN0.$$bindings = 'open';/*Ωignore_startΩ*/() => newNoteDir = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_golaiDetoNweN0.$$bindings = 'dir';}
 { const $$_rehctiwS0C = __sveltets_2_ensureComponent(Switcher); new $$_rehctiwS0C({ target: __sveltets_2_any(), props: {}});}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const VaultPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type VaultPage__SvelteComponent_ = ReturnType<typeof VaultPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default VaultPage__SvelteComponent_;