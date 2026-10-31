///<reference types="svelte" />
;// FileTree: lazy browsable file tree + read-only syntax-highlighted viewer
// for the Side panel's Files tab. With `navigation` it also gets the folder
// picker's navigation (lib/components/folderNavigation.ts): Back / Forward /
// Up, clickable breadcrumbs, a typed "go to" path, a Show hidden toggle, and
// an "open this folder" action on every folder row.

import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { onDestroy, tick, untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { openFile as openFileSignal } from '../../lib/stores/openfile.svelte';
import { api } from '../../lib/api/client';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { emptyHistory, folderCrumbs, historyTarget, parentFolder, resolveJump, settleFolder } from '../../lib/components/folderNavigation';
import { lsGet, lsSet } from '../../lib/storage';
import { ListWindow } from '../../lib/listWindow.svelte';
import { tabRowIn, treeKeyAction } from './treeKeys';
import { renderMarkdownGfm } from '../../lib/md';
import type { FsBrowse, FsEntry, FsRead } from '../../lib/api/types';
;

  interface Props {
    /** When provided, the tree is rooted here; otherwise falls back to
     *  ws.activeSession?.cwd or ws.current?.root_path. */
    root?: string;
    /** When provided, a ✕ button is shown in the header to close this section. */
    onClose?: () => void;
    /** When true, this section responds to programmatic open-file requests
     *  (terminal `file:line` clicks). Only one section should be primary. */
    primary?: boolean;
    /** Folder navigation bar (history, breadcrumbs, go-to path, hidden). */
    navigation?: boolean;
    /** Where listings / file reads come from — default `/fs/browse` / `/fs/read`; a workflow run's
     *  Context panel passes its run-scoped, RBAC-checked routes instead. */
    lister?: (path: string, signal?: AbortSignal) => Promise<FsBrowse>;
    reader?: (path: string, signal?: AbortSignal) => Promise<FsRead>;
  };;

  // ── tree state ────────────────────────────────────────────────────────────

  interface TreeNode {
    entry: FsEntry;
    depth: number;
    open: boolean;
    loaded: boolean;
    loading: boolean;
    children: TreeNode[];
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const fsList = (p: string, signal?: AbortSignal) => api.get<FsBrowse>(`/fs/browse?path=${encodeURIComponent(p)}&files=true`, signal);
  const fsRead = (p: string, signal?: AbortSignal) => api.get<FsRead>(`/fs/read?path=${encodeURIComponent(p)}`, signal);
  let { root = undefined, onClose = undefined, primary = false, navigation = false, lister = fsList, reader = fsRead }: Props = $props();


  // The effective root: prop takes precedence, then session cwd, then workspace root.
  const wsRoot = $derived(ws.activeSession?.cwd || ws.current?.root_path || '');
  // overriddenRoot holds folder-picker selections inside this component.
  let overriddenRoot = $state<string | undefined>(undefined);

  const effectiveRoot = $derived(overriddenRoot ?? root ?? wsRoot);

  // Top-level nodes (children of effectiveRoot).
  let rootNodes: TreeNode[] = $state([]);
  let rootLoading = $state(false);
  let rootError = $state('');
  let loadedRoot = $state('');
  let rootGeneration = 0;
  let rootRequest: AbortController | undefined;
  let fileGeneration = 0;
  let fileRequest: AbortController | undefined;

  // ── navigation state ──────────────────────────────────────────────────────
  // The folder actually listed (canonical, from the daemon) + its parent, the
  // Back/Forward history of listed folders, and the history entry a Back /
  // Forward step is walking to (settled when its listing lands).
  let listedPath = $state('');
  let listedParent = $state<string | null>(null);
  let history = $state(emptyHistory());
  let walkIndex: number | undefined;
  const backIndex = $derived(historyTarget(history, -1));
  const forwardIndex = $derived(historyTarget(history, 1));
  // Dotfiles are shown by default (the tree always listed them); the toggle
  // is remembered per device.
  const HIDDEN_KEY = 'otto_files_show_hidden';
  let showHidden = $state(lsGet(HIDDEN_KEY) !== '0');
  function toggleHidden(): void {
    showHidden = !showHidden;
    lsSet(HIDDEN_KEY, showHidden ? '1' : '0');
  }
  let editingPath = $state(false);
  let pathDraft = $state('');
  let pathInput = $state<HTMLInputElement | null>(null);
  let crumbsEl = $state<HTMLElement | null>(null);

  /** Show `path` as the tree's root (`index`: a Back/Forward step). */
  function navigate(path: string, index?: number): void {
    if (path === loadedRoot && !rootError) {
      // Already listed: only the history position moves.
      if (index !== undefined) history = { ...history, index };
      return;
    }
    walkIndex = index;
    overriddenRoot = path;
    if (path === loadedRoot) void loadRoot(path); // a failed listing: retry it
  }
  /** Follow the section's own folder again (the session's / the panel's). */
  function resetRoot(): void {
    walkIndex = undefined;
    overriddenRoot = undefined;
  }
  // The folder this section belongs to changed (another session focused, a
  // folder picked in the Side panel's folder bar, another workspace): follow
  // it — a folder browsed to under the OLD one would leave the tree and the
  // folder bar (and Git) disagreeing — and start a fresh Back/Forward history.
  let followedRoot = untrack(() => root);
  $effect(() => {
    const r = root;
    untrack(() => {
      if (r === followedRoot) return;
      followedRoot = r;
      resetRoot();
      history = emptyHistory();
    });
  });
  function walk(index: number | null): void {
    if (index !== null) navigate(history.paths[index], index);
  }
  async function startPathEdit(): Promise<void> {
    pathDraft = listedPath || effectiveRoot;
    editingPath = true;
    await tick();
    pathInput?.focus();
    pathInput?.select();
  }
  function commitPath(): void {
    const target = resolveJump(pathDraft, listedPath || effectiveRoot);
    editingPath = false;
    if (target) navigate(target);
  }
  function onPathKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitPath();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      editingPath = false;
    }
  }

  // ── viewer state ──────────────────────────────────────────────────────────

  let viewerFile: FsRead | null = $state(null);
  let viewerName = $state('');
  let viewerLoading = $state(false);
  let viewerError = $state('');
  // Line/col to reveal when the viewer opens from a terminal `file:line` click.
  let viewerGotoLine: number | null = $state(null);
  let viewerGotoCol: number | null = $state(null);

  // ── folder picker state ───────────────────────────────────────────────────

  let showPicker = $state(false);

  // ── effects ───────────────────────────────────────────────────────────────

  $effect(() => {
    const r = effectiveRoot;
    if (r && r !== loadedRoot) {
      // Reset tree state when root changes.
      rootNodes = [];
      void loadRoot(r);
    } else if (!r) {
      rootGeneration++; rootRequest?.abort();
      loadedRoot = ''; rootNodes = []; rootLoading = false; rootError = '';
    }
  });

  // Programmatic open: react to terminal `file:line` clicks. Only the primary
  // section handles them so a single, predictable viewer responds. The `n`
  // counter on the request makes each click fire exactly once.
  let lastOpenN = 0;
  $effect(() => {
    if (!primary) return;
    const req = openFileSignal.request;
    if (!req || req.n <= lastOpenN) return;
    lastOpenN = req.n;
    void openPath(req.path, req.line, req.col);
  });

  // ── helpers ───────────────────────────────────────────────────────────────

  async function loadRoot(r: string): Promise<void> {
    const generation = ++rootGeneration;
    rootRequest?.abort();
    const controller = new AbortController();
    rootRequest = controller;
    loadedRoot = r;
    rootLoading = true;
    rootError = '';
    const index = walkIndex;
    walkIndex = undefined;
    try {
      const data = await lister(r, controller.signal);
      if (generation !== rootGeneration) return;
      rootNodes = data.entries.map((e) => makeNode(e, 0));
      listedPath = data.path;
      listedParent = data.parent;
      history = settleFolder(history, data.path, index);
      await tick();
      if (generation === rootGeneration && crumbsEl) crumbsEl.scrollLeft = crumbsEl.scrollWidth;
    } catch (e) {
      if (generation === rootGeneration) rootError = loadErrorText(e);
    } finally {
      if (generation === rootGeneration) rootLoading = false;
    }
  }

  function makeNode(entry: FsEntry, depth: number): TreeNode {
    return { entry, depth, open: false, loaded: false, loading: false, children: [] };
  }

  async function toggleDir(node: TreeNode): Promise<void> {
    if (!node.entry.is_dir) return;
    const generation = rootGeneration;
    if (!node.open) {
      // Open: lazy-load children if not yet done.
      node.open = true;
      if (!node.loaded) {
        node.loading = true;
        try {
          const data = await lister(node.entry.path);
          if (generation !== rootGeneration) return;
          node.children = data.entries.map((e) => makeNode(e, node.depth + 1));
          node.loaded = true;
        } catch (e) {
          if (generation !== rootGeneration) return;
          toastError(`Couldn’t open ${node.entry.name}`, e);
          node.open = false;
        } finally {
          node.loading = false;
        }
      }
    } else {
      node.open = false;
    }
    // Trigger reactivity by reassigning rootNodes.
    if (generation === rootGeneration) rootNodes = [...rootNodes];
  }

  /** APG tree keys (treeKeys.ts): ↑/↓/Home/End, → into / ← out of folders. */
  function onTreeRowKey(e: KeyboardEvent, node: TreeNode): void {
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) return rowMenu(e, node);
    const list = (e.currentTarget as HTMLElement).closest<HTMLElement>('.tree-list');
    const rows = visibleNodes.map((n) => ({ depth: n.depth, isDir: n.entry.is_dir, open: n.open }));
    const i = visibleNodes.findIndex((n) => n.entry.path === node.entry.path);
    const act = treeKeyAction(e.key, rows, i, !!list && getComputedStyle(list).direction === 'rtl');
    if (act.kind === 'none' || !list) return;
    e.preventDefault();
    if (act.kind === 'toggle') void toggleDir(node);
    else void focusRow(list, act.index);
  }
  /** Focus visible row `i`, first scrolling it into the window (it may be unmounted). */
  async function focusRow(list: HTMLElement, i: number): Promise<void> {
    activeRow = visibleNodes[i]?.entry.path ?? activeRow;
    treeWin.reveal(i);
    await new Promise((r) => requestAnimationFrame(r));
    await tick();
    list.querySelectorAll<HTMLButtonElement>('button.tree-row')[i - treeRange.start]?.focus();
  }

  async function openFile(entry: FsEntry): Promise<void> {
    await readFile(entry.path, undefined, undefined, true, false);
  }

  const folderOf = (path: string): string => parentFolder(path) ?? '/';

  async function openPath(path: string, line?: number, col?: number): Promise<void> {
    await readFile(path, line, col, false, true);
  }

  async function readFile(path: string, line: number | undefined, col: number | undefined, preview: boolean, revealParent: boolean): Promise<void> {
    const generation = ++fileGeneration;
    fileRequest?.abort();
    const controller = new AbortController();
    fileRequest = controller;
    // Browsing location is independent from workspace/session identity. Listing
    // the parent is useful but never a prerequisite to showing a readable file.
    if (revealParent) overriddenRoot = folderOf(path);
    viewerGotoLine = line ?? null;
    viewerGotoCol = col ?? null;
    viewerLoading = true;
    viewerName = path.split('/').at(-1) ?? path;
    viewerFile = null;
    viewerError = '';
    previewMode = preview;
    try {
      const data = await reader(path, controller.signal);
      if (generation !== fileGeneration) return;
      viewerFile = data;
      if (revealParent) overriddenRoot = folderOf(data.path);
    } catch (e) {
      if (generation !== fileGeneration) return;
      viewerError = `Couldn’t open ${path}: ${loadErrorText(e)}. If the terminal shortened the filename, use its full path.`;
    } finally {
      if (generation === fileGeneration) viewerLoading = false;
    }
  }

  function closeViewer(): void {
    fileGeneration++; fileRequest?.abort();
    viewerFile = null; viewerError = ''; viewerLoading = false;
    viewerName = ''; viewerGotoLine = null; viewerGotoCol = null;
  }

  onDestroy(() => {
    rootGeneration++; fileGeneration++;
    rootRequest?.abort(); fileRequest?.abort();
  });

  // ── Markdown / HTML preview ────────────────────────────────────────────────
  let previewMode = $state(true);
  const viewerExt = $derived((viewerName.split('.').pop() ?? '').toLowerCase());
  const canPreview = $derived(['md', 'markdown', 'mdx', 'html', 'htm'].includes(viewerExt));
  const isMarkdown = $derived(['md', 'markdown', 'mdx'].includes(viewerExt));
  // Rendered HTML wrapped for a sandboxed iframe (no scripts run).
  const previewSrcdoc = $derived.by(() => {
    if (!viewerFile || !canPreview) return '';
    const inner = isMarkdown ? renderMarkdown(viewerFile.content) : viewerFile.content;
    return `<!doctype html><html><head><meta charset="utf-8"><style>${PREVIEW_CSS}</style></head><body>${inner}</body></html>`;
  });
  // Through lib/md (sanitized, never throws) — not a direct `marked` call.
  const renderMarkdown = (src: string): string => renderMarkdownGfm(src, { breaks: true });
  // The srcdoc is an isolated, script-less document: it can't see the app's
  // tokens, so it gets a palette per resolved scheme (it used to be dark-only
  // — light grey text on a hard-coded near-black box in the light theme).
  const PREVIEW_PALETTE = {
    dark: { text: '#dddddd', dim: '#aaaaaa', link: '#6ea8fe', line: '#ffffff22', fill: '#ffffff12' },
    light: { text: '#1d1d1f', dim: '#6e6e73', link: '#0a60d0', line: '#00000022', fill: '#0000000d' },
  } as const;
  const PREVIEW_CSS = $derived.by(() => {
    const p = PREVIEW_PALETTE[ui.resolvedScheme];
    return `
    :root { color-scheme: ${ui.resolvedScheme}; }
    body { font: 14px/1.6 -apple-system, system-ui, sans-serif; margin: 16px; color: ${p.text}; background: transparent; }
    h1,h2,h3 { line-height: 1.25; } h1,h2 { border-bottom: 1px solid ${p.line}; padding-bottom: .2em; }
    a { color: ${p.link}; } code { background: ${p.fill}; padding: .15em .35em; border-radius: 4px; font-family: ui-monospace, monospace; }
    pre { background: ${p.fill}; padding: 12px; border-radius: 6px; overflow: auto; } pre code { background: none; padding: 0; }
    table { border-collapse: collapse; } th,td { border: 1px solid ${p.line}; padding: 4px 8px; }
    blockquote { border-inline-start: 3px solid ${p.line}; margin: 0; padding-inline-start: 12px; color: ${p.dim}; }
    img { max-width: 100%; }
  `;
  });

  // Collect flattened visible nodes for rendering (DFS walk). Dotfiles (and
  // everything under a dot-folder) drop out while Show hidden is off.
  function flatten(nodes: TreeNode[], hidden: boolean): TreeNode[] {
    const out: TreeNode[] = [];
    for (const n of nodes) {
      if (!hidden && n.entry.name.startsWith('.')) continue;
      out.push(n);
      if (n.open && n.children.length > 0) {
        out.push(...flatten(n.children, hidden));
      }
    }
    return out;
  }

  const visibleNodes = $derived(flatten(rootNodes, showHidden || !navigation));

  /** Each visible row's place among its siblings (`aria-posinset` /
   *  `aria-setsize`, accessibility-responsive-4-08). `nodes` is depth-first,
   *  so a parent's children are contiguous at depth+1 between its own
   *  siblings; a group closes when the walk climbs above its depth. */
  function treePositions(nodes: TreeNode[]): Map<string, { pos: number; size: number }> {
    const out = new Map<string, { pos: number; size: number }>();
    const groups: string[][] = [];
    const close = (depth: number): void => {
      while (groups.length > depth) {
        const g = groups.pop() ?? [];
        g.forEach((path, i) => out.set(path, { pos: i + 1, size: g.length }));
      }
    };
    for (const n of nodes) {
      close(n.depth + 1);
      (groups[n.depth] ??= []).push(n.entry.path);
    }
    close(0);
    return out;
  }
  const positions = $derived(treePositions(visibleNodes));
  let activeRow = $state<string | null>(null);

  /** Row menu: "Open folder" (navigation mode) — a per-row button used to
   *  add a second Tab stop to every folder. Also on Shift+F10 / ContextMenu. */
  function rowMenu(e: MouseEvent | KeyboardEvent, node: TreeNode): void {
    if (!navigation || !node.entry.is_dir) return;
    e.preventDefault();
    ctxMenu.show(e, [{ label: `Open folder ${node.entry.name}`, icon: 'arrow', action: () => navigate(node.entry.path) }]);
  }
  // Windowed past 300 rows (ListWindow): opening node_modules or a build output
  // folder used to mount thousands of rows in a pane that shows ~40.
  const treeWin = new ListWindow('.tree-item');
  const treeRange = $derived(treeWin.range(visibleNodes.length));
  // Roving tabindex: ONE Tab stop, always on a MOUNTED row; ⌘F sees every row.
  const tabRow = $derived(tabRowIn(visibleNodes.map((n) => n.entry.path), activeRow, treeRange.start, treeRange.end));
  $effect(() => treeWin.findRows(() => null, () => visibleNodes, (n) => n.entry.name));
  const hiddenCount = $derived(navigation && !showHidden ? rootNodes.filter((n) => n.entry.name.startsWith('.')).length : 0);

  // Basename of root for display.
  function basename(p: string): string {
    return p.split('/').filter(Boolean).pop() ?? p;
  }

  function onPickFolder(path: string): void {
    closeViewer();
    overriddenRoot = path;
    showPicker = false;
  }
;
async () => {

if(showPicker){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Choose folder`,"start":effectiveRoot,"onpick":onPickFolder,"onclose":() => (showPicker = false),}});}
}

   { svelteHTML.createElement("div", { "class":`ft-wrap`,});
    
     { svelteHTML.createElement("div", { "class":`ft-header`,});
       { svelteHTML.createElement("span", {   "class":`ft-root-path dim`,"title":effectiveRoot,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`ft-root-text`,});basename(effectiveRoot); }
       }
      if(navigation && overriddenRoot !== undefined && (root ?? wsRoot) && overriddenRoot !== (root ?? wsRoot)){
         { svelteHTML.createElement("button", {         "class":`icon-btn ft-change-btn`,"title":`Back to ${basename(root ?? wsRoot)}`,"aria-label":`Back to ${basename(root ?? wsRoot)}`,"onclick":resetRoot,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`home`,"size":12,}});}
         }
      }
      if(effectiveRoot){
        
         { svelteHTML.createElement("button", {           "class":`icon-btn ft-change-btn`,"title":`Refresh folder`,"aria-label":`Refresh folder`,"disabled":rootLoading,"onclick":() => void loadRoot(effectiveRoot),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
         }
      }
       { svelteHTML.createElement("button", {         "class":`icon-btn ft-change-btn`,"title":`Change folder`,"aria-label":`Change folder`,"onclick":() => (showPicker = true),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
       }
      if(onClose){
         { svelteHTML.createElement("button", {         "class":`icon-btn ft-close-btn`,"title":`Close section`,"aria-label":`Close section`,"onclick":onClose,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
      }
     }

    if(navigation && effectiveRoot){
      
       { svelteHTML.createElement("div", { "class":`ft-nav`,});
         { svelteHTML.createElement("button", {         "class":`icon-btn ft-nav-btn`,"title":`Back`,"aria-label":`Back`,"disabled":backIndex === null || rootLoading,"onclick":() => walk(backIndex),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn ft-nav-btn`,"title":`Forward`,"aria-label":`Forward`,"disabled":forwardIndex === null || rootLoading,"onclick":() => walk(forwardIndex),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn ft-nav-btn`,"title":`Up one folder`,"aria-label":`Up one folder`,"disabled":!listedParent || rootLoading,"onclick":() => listedParent && navigate(listedParent),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}
         }
        if(editingPath){
           { const $$_input2 = svelteHTML.createElement("input", {                    "bind:value":pathDraft,"class":`input ft-path-input mono`,"dir":`ltr`,"aria-label":`Go to folder (absolute, ~ or relative path)`,"placeholder":`/path, ~/path or sub/folder`,"spellcheck":`false`,"autocomplete":`off`,"onkeydown":onPathKey,"onblur":() => (editingPath = false),});pathInput = $$_input2;/*Ωignore_startΩ*/() => pathDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        }else{
           { const $$_nav2 = svelteHTML.createElement("nav", {      "class":`ft-crumbs mono`,"dir":`ltr`,"aria-label":`Folder path`,});crumbsEl = $$_nav2;
                for(let part of __sveltets_2_ensureArray(folderCrumbs(listedPath || effectiveRoot))){let i = 1;part.path;
              if(i > 1){ { svelteHTML.createElement("span", {   "class":`ft-crumb-sep`,"aria-hidden":`true`,});  }}
               { svelteHTML.createElement("button", {           "class":`ft-crumb`,"title":part.path,"aria-current":part.path === (listedPath || effectiveRoot) ? 'location' : undefined,"onclick":() => navigate(part.path),"ondblclick":() => void startPathEdit(),});part.label; }
            }
           }
           { svelteHTML.createElement("button", {       "class":`icon-btn ft-nav-btn`,"title":`Type a path to go to`,"aria-label":`Type a path to go to`,"onclick":() => void startPathEdit(),});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
           }
        }
         { svelteHTML.createElement("button", {            "class":`icon-btn ft-nav-btn`,"title":showHidden ? 'Hide hidden files' : 'Show hidden files',"aria-label":`Show hidden files`,"aria-pressed":showHidden,"onclick":toggleHidden,});showHidden;
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":showHidden ? 'eye' : 'eyeOff',"size":12,}});}
         }
       }
    }

    if(!effectiveRoot){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`folder`,"title":`No folder to browse`,"body":`Choose a folder to browse, or open a file from a terminal link.`,"actionLabel":`Choose folder`,"actionIcon":`folder`,"onaction":() => (showPicker = true),}});}
    } else if ((rootLoading || rootError) && rootNodes.length === 0){
      const root = effectiveRoot;
      
       { svelteHTML.createElement("div", { "class":`load-pad`,});
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`the files in ${basename(root)}`,"loading":rootLoading,"error":rootError || null,"empty":true,"onretry":() => void loadRoot(root),}});}
       }
    }else{
      if(rootError){
        const root = effectiveRoot;
         { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`the files in ${basename(root)}`,"error":rootError,"empty":false,"onretry":() => void loadRoot(root),}});}
      }
      
       { svelteHTML.createElement("div", {  "class":`tree-pane`,});!!viewerFile;
         { svelteHTML.createElement("div", {      "class":`tree-list`,"role":`tree`,"aria-label":`Files in ${basename(effectiveRoot)}`,[Symbol("@attach")]:treeWin.attach,});
          if(treeRange.top > 0){ { svelteHTML.createElement("div", {   "style":`height:${treeRange.top}px`,"aria-hidden":`true`,}); }}
             for(let node of __sveltets_2_ensureArray((visibleNodes.slice(treeRange.start, treeRange.end)))){node.entry.path;
            const place = positions.get(node.entry.path);
             { svelteHTML.createElement("div", {   "class":`tree-item`,"role":`none`,});
             { svelteHTML.createElement("button", {                                  "class":`tree-row`,"role":`treeitem`,"aria-level":node.depth + 1,"aria-setsize":place?.size,"aria-posinset":place?.pos,"aria-selected":viewerFile?.path === node.entry.path,"tabindex":node.entry.path === tabRow ? 0 : -1,"onfocus":() => (activeRow = node.entry.path),"oncontextmenu":(e) => rowMenu(e, node),"style":`padding-inline-start: ${8 + node.depth * 14}px`,"aria-expanded":node.entry.is_dir ? node.open : undefined,"onclick":() => node.entry.is_dir ? toggleDir(node) : openFile(node.entry),"ondblclick":() => navigation && node.entry.is_dir && navigate(node.entry.path),"onkeydown":(e) => onTreeRowKey(e, node),"title":node.entry.path,});node.entry.is_dir;node.open;!node.entry.is_dir;
              if(node.entry.is_dir){
                 { svelteHTML.createElement("span", { "class":`chevron`,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":node.open ? 'chevronDown' : 'chevronRight',"size":12,}});}
                 }
                if(node.loading){
                   { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 12px`,"aria-hidden":`true`,}); }
                }else{
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
                }
              }else{
                 { svelteHTML.createElement("span", { "class":`file-spacer`,}); }
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
              }
               { svelteHTML.createElement("span", { "class":`row-name`,});node.entry.name; }
              if(node.entry.is_git_repo){
                 { svelteHTML.createElement("span", { "class":`git-badge dim`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});} }
              }
             }
            if(navigation && node.entry.is_dir){
              
               { svelteHTML.createElement("button", {         "class":`icon-btn enter-btn`,"tabindex":-1,"title":`Open folder ${node.entry.name}`,"aria-label":`Open folder ${node.entry.name}`,"onclick":() => navigate(node.entry.path),});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrow`,"size":12,}});}
               }
            }
             }
          }
          if(treeRange.bottom > 0){ { svelteHTML.createElement("div", {   "style":`height:${treeRange.bottom}px`,"aria-hidden":`true`,}); }}
          if(visibleNodes.length === 0){
             { svelteHTML.createElement("div", { "class":`empty-dir dim`,});
              if(hiddenCount > 0){     { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":toggleHidden,});  }}else{ }
             }
          }
         }
       }

    }
      
      if(viewerFile || viewerLoading || viewerError){
         { svelteHTML.createElement("div", { "class":`viewer-pane`,});
           { svelteHTML.createElement("div", { "class":`viewer-header`,});
             { svelteHTML.createElement("span", {   "class":`viewer-name`,"title":viewerFile?.path ?? viewerName,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
              
               { svelteHTML.createElement("span", { "class":`viewer-name-text`,});viewerName; }
             }
            if(viewerFile?.truncated){
               { svelteHTML.createElement("span", {   "class":`truncated-badge dim`,"title":`File truncated at ~400 KB`,});  }
            }
            if(canPreview){
               { svelteHTML.createElement("div", { "class":`preview-toggle`,});
                 { svelteHTML.createElement("button", {      "class":`pv`,"aria-pressed":!previewMode,"onclick":() => (previewMode = false),});!previewMode;  }
                 { svelteHTML.createElement("button", {      "class":`pv`,"aria-pressed":previewMode,"onclick":() => (previewMode = true),});previewMode;  }
               }
            }
             { svelteHTML.createElement("button", {       "class":`close-btn icon-btn`,"onclick":closeViewer,"title":`Close viewer`,"aria-label":`Close viewer`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
          if(viewerLoading){
             { svelteHTML.createElement("div", { "class":`loading`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":8,"height":14,"label":viewerName,}});} }
          } else if (viewerError){
             { svelteHTML.createElement("div", {   "class":`error-msg`,"role":`alert`,});viewerError; }
          } else if (viewerFile){
            if(canPreview && previewMode){
               { svelteHTML.createElement("iframe", {       "class":`preview-frame`,"title":`Preview`,"sandbox":`allow-same-origin`,"srcdoc":previewSrcdoc,}); }
            }else{
               { svelteHTML.createElement("div", { "class":`code-scroll`,});
                 { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {             "path":viewerFile.path,"content":viewerFile.content,"root":effectiveRoot,"lsp":true,"gotoLine":viewerGotoLine,"gotoCol":viewerGotoCol,"readOnly":true,}});}
               }
            }
          }
         }
      }
   }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FileTree__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FileTree__SvelteComponent_ = ReturnType<typeof FileTree__SvelteComponent_>;
/*Ωignore_endΩ*/export default FileTree__SvelteComponent_;