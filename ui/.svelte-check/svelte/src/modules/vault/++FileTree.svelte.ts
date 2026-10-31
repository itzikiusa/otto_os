///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
// Vault file explorer — lazy directory tree over the store's TreeNode roots,
// virtualized (big vaults stay cheap), with ctx-menu file ops and
// drag-to-folder moves.
import VirtualList from '../../lib/components/VirtualList.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { vault, type TreeNode } from './vault.svelte';
import { tick } from 'svelte';
import { focusOnMount } from '../../lib/focusOnMount';

;type $$ComponentProps =  {
    /** Open the new-note dialog seeded with a folder. */
    onNewNote: (dir: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    onNewNote,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Re-derive the flat list from tree state (open/loaded live on the nodes).
  const flat = $derived.by(() => {
    // Touch roots deeply so toggles re-run this.
    void vault.roots;
    return vault.flatTree();
  });

  let renaming = $state<string | null>(null);
  let renameValue = $state('');
  let dragOver = $state<string | null>(null);

  // Multi-select (notes/files, not dirs): hover checkboxes; any selection
  // keeps every checkbox visible and shows the group action bar.
  let selected = $state<Set<string>>(new Set());

  function toggleSelect(path: string): void {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }

  function clearSelection(): void {
    selected = new Set();
  }

  // Free-form group instruction + the agent that runs it, all INLINE in the
  // selection bar — window.prompt() does not exist in the desktop webview
  // (silent no-op).
  let groupPrompt = $state('');
  let groupProvider = $state(defaultAgentProvider());
  let groupModel = $state('');
  let groupInputEl = $state<HTMLInputElement | null>(null);

  function groupAgent(): { provider: string; model?: string } {
    return { provider: groupProvider, model: groupModel.trim() || undefined };
  }

  function groupReviewFix(): void {
    vault.sendGroupToAgent([...selected], null, groupAgent());
    clearSelection();
  }

  function groupSend(): void {
    const inst = groupPrompt.trim();
    if (!inst) {
      groupInputEl?.focus();
      return;
    }
    vault.sendGroupToAgent([...selected], inst, groupAgent());
    groupPrompt = '';
    clearSelection();
  }

  /** Mode-aware selection — only the pane actually shown highlights its row. */
  function isActive(n: TreeNode): boolean {
    return vault.centerMode === 'note'
      ? vault.notePath === n.entry.path
      : vault.centerMode === 'file' && vault.filePath === n.entry.path;
  }

  function rowClick(n: TreeNode, e?: MouseEvent | KeyboardEvent): void {
    if (n.entry.kind === 'dir') {
      void vault.toggleDir(n);
      return;
    }
    // ⌘/Ctrl-click → toggle multi-select (OS file-manager convention). Open
    // in a new tab moved to middle-click (onauxclick) / the context menu.
    if (e && (e.metaKey || e.ctrlKey)) {
      toggleSelect(n.entry.path);
      return;
    }
    if (n.entry.kind === 'note') void vault.open(n.entry.path);
    else void vault.openFile(n.entry.path);
  }

  function rowAuxClick(n: TreeNode, e: MouseEvent): void {
    if (e.button !== 1 || n.entry.kind === 'dir') return;
    e.preventDefault();
    if (n.entry.kind === 'note') void vault.open(n.entry.path, { newTab: true });
    else void vault.openFile(n.entry.path, { newTab: true });
  }

  function startRename(n: TreeNode): void {
    renaming = n.entry.path;
    renameValue = n.entry.name;
  }

  function commitRename(n: TreeNode): void {
    // Enter/Escape unmount the input, and WebKit (the desktop webview) fires
    // blur on a removed focused element — without this guard Enter renamed
    // twice (the second call failing with a "not found" toast) and Escape
    // COMMITTED the rename it was meant to cancel.
    if (renaming !== n.entry.path) return;
    const name = renameValue.trim();
    renaming = null;
    if (!name || name === n.entry.name) return;
    const dir = n.entry.path.includes('/')
      ? n.entry.path.slice(0, n.entry.path.lastIndexOf('/') + 1)
      : '';
    void vault.rename(n.entry.path, dir + name);
  }

  // ── Tree keyboard (WAI-ARIA tree view, roving tabindex) ─────────────────────
  // One row is in the Tab order: the last focused row, else the open note/file,
  // else the first row. ↑/↓/Home/End move it, → opens a folder or steps into
  // it, ← closes it or steps out to the parent folder (mirrored under RTL),
  // Enter/Space opens, ⌘/Ctrl+Space toggles the note's selection for group
  // agent actions (the per-row checkbox stays out of the Tab order), F2
  // renames, ContextMenu / ⇧F10 opens the row menu. The list is
  // virtualized, so the focused row is pinned (kept mounted) and scrolled to.
  let treeEl = $state<HTMLElement | null>(null);
  let focusPath = $state<string | null>(null);
  let scrollVer = $state(0);
  const tabStop = $derived.by(() => {
    if (focusPath && flat.some((n) => n.entry.path === focusPath)) return focusPath;
    return flat.find((n) => isActive(n))?.entry.path ?? flat[0]?.entry.path ?? null;
  });
  const tabIndex = $derived(tabStop ? flat.findIndex((n) => n.entry.path === tabStop) : -1);

  function focusRow(i: number): void {
    const n = flat[Math.max(0, Math.min(flat.length - 1, i))];
    if (!n) return;
    focusPath = n.entry.path;
    scrollVer++;
    void tick().then(() => treeEl?.querySelector<HTMLElement>(`[data-path="${CSS.escape(n.entry.path)}"]`)?.focus());
  }

  function rowKey(e: KeyboardEvent, n: TreeNode): void {
    if (e.target !== e.currentTarget) return; // the checkbox / rename input own their keys
    const i = flat.indexOf(n);
    const isDir = n.entry.kind === 'dir';
    const rtl = treeEl ? getComputedStyle(treeEl).direction === 'rtl' : false;
    const key = rtl && e.key === 'ArrowRight' ? 'ArrowLeft' : rtl && e.key === 'ArrowLeft' ? 'ArrowRight' : e.key;
    if (key === 'ArrowDown' || key === 'ArrowUp' || key === 'Home' || key === 'End') {
      e.preventDefault();
      focusRow(key === 'Home' ? 0 : key === 'End' ? flat.length - 1 : i + (key === 'ArrowDown' ? 1 : -1));
    } else if (key === 'ArrowRight') {
      e.preventDefault();
      if (isDir && !n.open) void vault.toggleDir(n);
      else if (isDir && n.children.length) focusRow(i + 1);
    } else if (key === 'ArrowLeft') {
      e.preventDefault();
      if (isDir && n.open) {
        void vault.toggleDir(n);
        return;
      }
      const cut = n.entry.path.lastIndexOf('/');
      const parent = cut > 0 ? flat.findIndex((x) => x.entry.path === n.entry.path.slice(0, cut)) : -1;
      if (parent >= 0) focusRow(parent);
    } else if (e.key === ' ' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (!isDir) toggleSelect(n.entry.path);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      rowClick(n, e);
    } else if (e.key === 'F2') {
      e.preventDefault();
      startRename(n);
    } else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault();
      menu(e, n);
    }
  }

  function menu(e: MouseEvent | KeyboardEvent, n: TreeNode): void {
    const isDir = n.entry.kind === 'dir';
    ctxMenu.show(e, [
      ...(isDir
        ? []
        : [
            {
              label: 'Open in new tab',
              icon: n.entry.kind === 'note' ? 'note' : 'file',
              action: () =>
                n.entry.kind === 'note'
                  ? void vault.open(n.entry.path, { newTab: true })
                  : void vault.openFile(n.entry.path, { newTab: true }),
            },
            // With a multi-selection containing this note, Review + fix acts
            // on the WHOLE selection (same as the selection bar) — a single
            // right-clicked note out of N selected being fixed alone is never
            // what the user meant.
            ...(n.entry.kind === 'note'
              ? [
                  selected.size > 1 && selected.has(n.entry.path)
                    ? {
                        label: `Review + fix ${selected.size} selected (agent)`,
                        icon: 'zap',
                        action: () => groupReviewFix(),
                      }
                    : {
                        label: 'Review + fix (agent)',
                        icon: 'zap',
                        action: () => vault.reviewFixNote(n.entry.path),
                      },
                ]
              : []),
            { separator: true },
          ]),
      ...(isDir
        ? [
            { label: 'New note here', icon: 'note', action: () => onNewNote(n.entry.path) },
            {
              label: 'Docs agent here',
              icon: 'zap',
              action: () => vault.openDocsAgents(n.entry.path),
            },
            {
              label: 'Review + fix docs (agent)',
              icon: 'zap',
              action: () => vault.reviewFixBundle(n.entry.path),
            },
            {
              // Opens the docs-agent form scoped to the folder — the form's
              // prompt box is where the instruction is written (window.prompt
              // does not exist in the desktop webview).
              label: 'Send to agent…',
              icon: 'zap',
              action: () => vault.sendDirToAgent(n.entry.path, ''),
            },
            {
              label: 'New folder here',
              icon: 'folder',
              action: async () => {
                const name = await confirmer.promptText('Folder name', { title: 'New folder', confirmLabel: 'Create' });
                if (name?.trim()) void vault.createFolder(`${n.entry.path}/${name.trim()}`);
              },
            },
            { separator: true },
          ]
        : []),
      { label: 'Rename', icon: 'edit', action: () => startRename(n) },
      {
        label: 'Move to…',
        icon: 'branch',
        action: async () => {
          const to = await confirmer.promptText('Move to path', { title: 'Move', confirmLabel: 'Move', initial: n.entry.path });
          if (to?.trim() && to.trim() !== n.entry.path) void vault.rename(n.entry.path, to.trim());
        },
      },
      { separator: true },
      {
        label: 'Move to trash', // ui-guards: allow — reversible (restore from the trash), so no confirm
        icon: 'trash',
        danger: true,
        action: () => void vault.trash(n.entry.path),
      },
    ]);
  }

  // Drag a file onto a folder row → move.
  function onDragStart(e: DragEvent, n: TreeNode): void {
    e.dataTransfer?.setData('text/vault-path', n.entry.path);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDrop(e: DragEvent, n: TreeNode): void {
    e.preventDefault();
    dragOver = null;
    const from = e.dataTransfer?.getData('text/vault-path');
    if (!from || n.entry.kind !== 'dir') return;
    const base = from.split('/').pop()!;
    const to = `${n.entry.path}/${base}`;
    if (to !== from && !to.startsWith(`${from}/`)) void vault.rename(from, to);
  }
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {        "class":`tree`,"role":`tree`,"aria-label":`Vault files`,"aria-multiselectable":`true`,});treeEl = $$_div0;
  if((vault.treeLoading || vault.treeError) && flat.length === 0){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "what":`files`,"variant":`compact`,"loading":vault.treeLoading,"error":vault.treeError,"empty":true,"onretry":() => void vault.loadRoot(),}});}
  } else if (flat.length === 0){
     { svelteHTML.createElement("div", { "class":`empty`,});
        
      if(vault.centerMode !== 'empty'){
         { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => onNewNote(''),});  }
      }else{
         { svelteHTML.createElement("button", {   "class":`btn ghost only-phone`,"onclick":() => onNewNote(''),});  }
      }
     }
  }else{
    if(vault.treeError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`files`,"variant":`compact`,"error":vault.treeError,"onretry":() => void vault.loadRoot(),}});}
    }
    
     { const $$_tsiLlautriV1C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV1 = new $$_tsiLlautriV1C({ target: __sveltets_2_any(), props: {               "items":flat,"estimateHeight":26,"class":`tree-list`,"pinnedIndex":tabIndex,"scrollIndex":tabIndex,"scrollVersion":scrollVer,"findText":(n: TreeNode) => (n.entry.kind === 'note' ? n.entry.name.replace(/\.md$/i, '') : n.entry.name),row:(n: TreeNode) => { async ()/*Ωignore_positionΩ*/ => {
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                                            "class":`row ${n.entry.kind}`,"style":`padding-inline-start: ${8 + n.depth * 14}px`,"role":`treeitem`,...__sveltets_2_empty({"data-path":n.entry.path}),"aria-selected":n.entry.kind === 'dir' ? undefined : selected.has(n.entry.path),"aria-current":isActive(n) ? 'page' : undefined,"aria-level":n.depth + 1,"aria-expanded":n.entry.kind === 'dir' ? n.open : undefined,"tabindex":n.entry.path === tabStop ? 0 : -1,"draggable":n.entry.kind !== 'dir',"onclick":(e) => rowClick(n, e),"onfocus":() => (focusPath = n.entry.path),"onauxclick":(e) => rowAuxClick(n, e),"onkeydown":(e) => rowKey(e, n),"oncontextmenu":(e) => menu(e, n),"ondragstart":(e) => onDragStart(e, n),"ondragover":(e) => {
            if (n.entry.kind === 'dir') {
              e.preventDefault();
              dragOver = n.entry.path;
            }
          },"ondragleave":() => (dragOver = null),"ondrop":(e) => onDrop(e, n),});isActive(n);n.entry.reserved;dragOver === n.entry.path;selected.has(n.entry.path);
          if(n.entry.kind === 'dir'){
             { svelteHTML.createElement("span", {  "class":`chev`,});n.open; { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
             { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":14,}});}
          }else{
             { svelteHTML.createElement("input", {                "type":`checkbox`,"class":`sel reveal-on-hover`,"checked":selected.has(n.entry.path),"tabindex":-1,"aria-label":`Select ${n.entry.kind === 'note' ? n.entry.name.replace(/\.md$/i, '') : n.entry.name}`,"title":`Select for group agent actions (⌘/Ctrl+Space)`,"onclick":(e) => {
                e.stopPropagation();
                toggleSelect(n.entry.path);
              },});selected.size > 0;}
             { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":n.entry.kind === 'note' ? 'note' : 'file',"size":14,}});}
          }
           { svelteHTML.createElement("span", {   "class":`name`,"title":n.entry.reserved ? `${n.entry.path} — reserved OKF file (folder index)` : n.entry.path,});
            n.entry.kind === 'note' ? n.entry.name.replace(/\.md$/i, '') : n.entry.name;
           }
          if(n.entry.kind === 'dir'){
            const prov = vault.draftAgentLabel(n.entry.path);
            if(prov){
               { svelteHTML.createElement("span", {   "class":`prov`,"title":`Writer agent`,});prov; }
            }
          }
          if(renaming === n.entry.path){
             {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input'),({ select: true })));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {                "dir":`auto`,"class":`rename`,"aria-label":`New name`,"bind:value":renameValue,"onclick":(e) => e.stopPropagation(),"onkeydown":(e) => {
                if (e.key === 'Enter') commitRename(n);
                if (e.key === 'Escape') renaming = null;
                e.stopPropagation();
              },"onblur":() => commitRename(n),});/*Ωignore_startΩ*/() => renameValue = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
          }
          if(n.entry.kind === 'dir' && n.entry.children > 0){
             { svelteHTML.createElement("span", { "class":`count`,});n.entry.children; }
          }
         }}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV1.$$prop_def;/*Ωignore_endΩ*/
      
     VirtualList}
  }

  if(selected.size > 0){
     { svelteHTML.createElement("div", { "class":`sel-bar`,});
       { svelteHTML.createElement("div", { "class":`sel-row`,});
         { svelteHTML.createElement("span", { "class":`sel-count`,});selected.size;  }
         { svelteHTML.createElement("button", {       "class":`btn small`,"title":`Review + fix ALL ${selected.size} selected notes as one coherent set (starts the agent immediately)`,"onclick":groupReviewFix,});
            
         }
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":clearSelection,});  }
       }
       { svelteHTML.createElement("div", { "class":`sel-row`,});
         { svelteHTML.createElement("select", {         "class":`sel-select`,"bind:value":groupProvider,"title":`Agent provider that runs the group action`,"aria-label":`Agent provider`,});/*Ωignore_startΩ*/() => groupProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(agentProviders())){p;
             { svelteHTML.createElement("option", { "value":p,});p; }
          }
         }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`sel-input`,"bind:value":groupModel,"placeholder":`Model (optional)`,"aria-label":`Model override`,"title":`Model override for the agent (leave empty for the provider default)`,});/*Ωignore_startΩ*/() => groupModel = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`sel-row`,});
         { const $$_input3 = svelteHTML.createElement("input", {             "dir":`auto`,"class":`sel-input`,"bind:value":groupPrompt,"placeholder":`Instruction for the ${selected.size} notes… (Enter to send)`,"aria-label":`Instruction for the selected notes`,"onkeydown":(e) => {
            if (e.key === 'Enter') groupSend();
          },});groupInputEl = $$_input3;/*Ωignore_startΩ*/() => groupPrompt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {         "class":`btn small`,"title":`Send ALL ${selected.size} selected notes to an agent with this instruction`,"disabled":!groupPrompt.trim(),"onclick":groupSend,});
          
         }
       }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FileTree__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FileTree__SvelteComponent_ = ReturnType<typeof FileTree__SvelteComponent_>;
/*Ωignore_endΩ*/export default FileTree__SvelteComponent_;