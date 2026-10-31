///<reference types="svelte" />
;// Daemon-side folder/file picker. Browses the directory tree on the machine that
// runs ottod (the path the user picks is used as a session cwd / repo path,
// so it must exist on the daemon host, not the client). Backed by GET
// /fs/browse. `gitOnly` highlights git repos for the repo picker.
// When `files` is true, files are shown and can be picked directly (for
// identity-file selection etc.); directories still navigate on click.

import { untrack, onDestroy, onMount, tick } from 'svelte';
import Skeleton from './Skeleton.svelte';
import Badge from './Badge.svelte';
import { emptyHistory, settleFolder, historyTarget, folderCrumbs, emptyShortcuts, parseShortcuts, rememberFolder, toggleFavorite } from './folderNavigation';
import { auth } from '../stores/auth.svelte';
import { api, baseUrl } from '../api/client';
import type { FsBrowse, FsEntry } from '../api/types';
import Modal from './Modal.svelte';
import Icon from './Icon.svelte';
import VirtualList from './VirtualList.svelte';
import { moveGrid, gridAction } from './folderGrid';
;

  interface Props {
    title?: string;
    start?: string;
    /** Only allow picking directories that are git repos (repo registration). */
    gitOnly?: boolean;
    /** When true, files are shown as selectable rows and the picker can return
     *  a file path. Directory navigation still works normally. Default false. */
    files?: boolean;
    onpick: (path: string) => void;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { title = 'Choose Folder', start = '', gitOnly = false, files = false, onpick, onclose }: Props = $props();

  let view: FsBrowse | null = $state(null);
  let loading = $state(true);
  let error = $state('');
  // Type-to-filter the current listing; "Show hidden" reveals dotfiles
  // (default off — dotfiles are hidden so the listing isn't cluttered).
  let filter = $state('');
  let filterElement: HTMLInputElement | undefined = $state();
  let retryElement: HTMLButtonElement | undefined = $state();
  let gridElement: HTMLDivElement | undefined = $state();
  let position = $state({ index: 0, action: 0 });
  let scrollVersion = $state(0);
  const gridId = `folder-grid-${crypto.randomUUID()}`;
  let showHidden = $state(false);

  // The entries actually rendered: dotfiles hidden unless `showHidden`, and
  // name-filtered (case-insensitive) by the filter box. The `..` up row is
  // rendered separately and always shown.
  const shown = $derived.by((): FsEntry[] => {
    const entries: FsEntry[] = view?.entries ?? [];
    const q = filter.trim().toLowerCase();
    return entries.filter((e) => {
      if (!showHidden && e.name.startsWith('.')) return false;
      return q === '' || e.name.toLowerCase().includes(q);
    });
  });

  let history = $state(emptyHistory());
  let shortcuts = $state(emptyShortcuts());
  let storageKey = '';
  let crumbsElement: HTMLElement | undefined = $state();
  let requestSeq = 0;
  let requestController: AbortController | null = null;
  let lastAttempt = $state<{ path: string; index?: number }>({ path: '' });
  const backIndex = $derived(historyTarget(history, -1));
  const forwardIndex = $derived(historyTarget(history, 1));

  async function load(path: string, index?: number): Promise<void> {
    const restoreFocus = !!gridElement && (document.activeElement === gridElement || gridElement.contains(document.activeElement));
    if (restoreFocus) filterElement?.focus();
    requestController?.abort();
    const controller = new AbortController();
    requestController = controller;
    let timedOut = false;
    const timeout = setTimeout(() => { timedOut = true; controller.abort(); }, 12000);
    const seq = ++requestSeq;
    lastAttempt = { path, index };
    loading = true;
    error = '';
    try {
      let q = path ? `?path=${encodeURIComponent(path)}` : '';
      if (files) q = q ? `${q}&files=true` : '?files=true';
      const next = await api.get<FsBrowse>(`/fs/browse${q}`, controller.signal);
      if (seq !== requestSeq) return;
      view = next;
      updateShortcuts(current => rememberFolder(current, next.path));
      filter = ''; // A successful navigation starts with the full listing.
      history = settleFolder(history, next.path, index);
      await tick();
      if (seq === requestSeq && crumbsElement) crumbsElement.scrollLeft = crumbsElement.scrollWidth;
    } catch (e) {
      if (seq === requestSeq) error = timedOut
        ? 'Folder listing timed out. Retry or choose another folder.'
        : (e instanceof Error ? e.message : String(e)) || 'Couldn’t open folder.';
    } finally {
      clearTimeout(timeout);
      if (seq === requestSeq) {
        loading = false;
        if (requestController === controller) requestController = null;
        if (restoreFocus) { await tick(); if (seq === requestSeq) (error ? retryElement : gridElement)?.focus(); }
      }
    }
  }

  function disposeRequest() { requestSeq++; requestController?.abort(); requestController = null; }
  function closePicker() { disposeRequest(); onclose(); }
  function pick(path: string) { disposeRequest(); onpick(path); }
  function cellId(index: number, action: number) { return `${gridId}-${encodeURIComponent(shown[index]?.path ?? '')}-${action}`; }
  function activate(index: number, action: number) {
    const entry = shown[index]; if (!entry) return;
    const intent = gridAction(entry, action, gitOnly);
    if (intent === 'open') void load(entry.path);
    else if (intent === 'pick') pick(entry.path);
  }
  function gridKey(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey || shown.length === 0) return;
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); event.stopPropagation(); activate(position.index, position.action); return; }
    const next = moveGrid(position, event.key, shown.length, Math.max(1, Math.floor((gridElement?.clientHeight ?? 288) / 32)));
    if (!next) return;
    event.preventDefault(); event.stopPropagation();
    if (!gridAction(shown[next.index], next.action, gitOnly)) next.action = 0;
    position = next; scrollVersion++;
  }
  $effect(() => {
    void filter; void showHidden; void view?.path;
    untrack(() => { position = { index: 0, action: 0 }; scrollVersion++; });
  });

  function updateShortcuts(update: (current: ReturnType<typeof emptyShortcuts>) => ReturnType<typeof emptyShortcuts>) {
    // Merge against storage at mutation time: another window may have pinned a
    // folder since this picker opened. Navigating must never overwrite that pin.
    let latest = shortcuts;
    try { latest = parseShortcuts(localStorage.getItem(storageKey)); } catch { /* Memory-only fallback. */ }
    shortcuts = update(latest);
    try {
      localStorage.setItem(storageKey, JSON.stringify(shortcuts));
      window.dispatchEvent(new CustomEvent('otto:folder-shortcuts', { detail: storageKey }));
    } catch { /* Private browsing may disable storage. */ }
  }

  function favoriteCurrent() {
    if (!view || loading || error) return;
    const path = view.path;
    updateShortcuts(current => toggleFavorite(current, path));
  }

  onMount(() => {
    const refresh = () => {
      try { shortcuts = parseShortcuts(localStorage.getItem(storageKey)); } catch { /* Keep in-memory choices. */ }
    };
    const stored = (event: StorageEvent) => { if (event.key === storageKey || event.key === null) refresh(); };
    const local = (event: Event) => { if ((event as CustomEvent).detail === storageKey) refresh(); };
    window.addEventListener('storage', stored);
    window.addEventListener('otto:folder-shortcuts', local);
    return () => {
      window.removeEventListener('storage', stored);
      window.removeEventListener('otto:folder-shortcuts', local);
    };
  });

  function folderName(path: string) { return path.split('/').filter(Boolean).at(-1) ?? '/'; }

  function traverse(index: number | null) {
    if (index !== null) void load(history.paths[index], index);
  }

  $effect(() => {
    const initial = start;
    const key = `otto_folder_shortcuts:${baseUrl()}:${auth.me?.id ?? 'anonymous'}`;
    void files;
    untrack(() => {
      storageKey = key;
      try { shortcuts = parseShortcuts(localStorage.getItem(key)); } catch { shortcuts = emptyShortcuts(); }
      history = emptyHistory();
      view = null;
      void load(initial);
    });
  });
  onDestroy(disposeRequest);
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },title,"onclose":closePicker,"width":620,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":closePicker,});  }
    
    if(!files && (!gitOnly || view?.is_git_repo)){
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!view || loading || !!error,"onclick":() => view && pick(view.path),});
        gitOnly ? 'Use this repository' : 'Use this folder';
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`navigation`,"aria-label":`Folder navigation`,});
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":loading || backIndex === null,"onclick":() => traverse(backIndex),});  }
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":loading || forwardIndex === null,"onclick":() => traverse(forwardIndex),});  }
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":loading || !view?.parent,"onclick":() => view?.parent && load(view.parent),});  }
     { svelteHTML.createElement("button", {     "class":`btn favorite-action`,"disabled":!view || loading || !!error,"onclick":favoriteCurrent,});
      view && shortcuts.favorites.includes(view.path) ? 'Remove favorite' : 'Add favorite';
     }
   }
  if(view){
    if(error || loading){ { svelteHTML.createElement("div", { "class":`dim last-opened`,});   }}
     { const $$_nav1 = svelteHTML.createElement("nav", {         "class":`crumb mono`,"dir":`ltr`,"aria-label":`Folder path`,...__sveltets_2_empty({"data-path":view.path}),});crumbsElement = $$_nav1;
          for(let part of __sveltets_2_ensureArray(folderCrumbs(view.path))){let i = 1;part.path;
        if(i > 1){ { svelteHTML.createElement("span", { "aria-hidden":`true`,});  }}
         { svelteHTML.createElement("button", {       "title":part.path,"aria-current":part.path === view.path ? 'location' : undefined,"disabled":loading,"onclick":() => load(part.path),});part.label; }
      }
     }
  }

   { svelteHTML.createElement("div", { "class":`pick-tools`,});
      { const $$_input2 = svelteHTML.createElement("input", {           "dir":`ltr`,"class":`input filter-input`,"placeholder":`Filter…`,"aria-label":`Filter folders`,"bind:value":filter,...__sveltets_2_empty({"data-autofocus":true}),});filterElement = $$_input2;/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("label", {   "class":`hidden-toggle`,"title":`Show dotfiles (names starting with .)`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showHidden,});/*Ωignore_startΩ*/() => showHidden = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       
     }
   }

   { svelteHTML.createElement("div", { "class":`pick-content`,});
     { svelteHTML.createElement("aside", {   "class":`shortcuts`,"aria-label":`Quick navigation`,});
       { svelteHTML.createElement("section", { "aria-label":`Favorites`,});
         { svelteHTML.createElement("h4", {});  }
         { svelteHTML.createElement("div", { "class":`shortcut-list`,});
             for(let path of __sveltets_2_ensureArray(shortcuts.favorites)){path;
             { svelteHTML.createElement("div", { "class":`favorite-row`,});
               { svelteHTML.createElement("button", {      "title":path,"disabled":loading,"onclick":() => load(path),});path === view?.path;folderName(path); }
               { svelteHTML.createElement("button", {       "class":`remove-favorite`,"aria-label":`Remove ${folderName(path)} from favorites`,"title":`Remove favorite`,"onclick":() => updateShortcuts(current => toggleFavorite(current, path)),}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
          } { svelteHTML.createElement("p", { "class":`dim`,});      }
         }
       }
       { svelteHTML.createElement("section", { "aria-label":`Recents`,});
         { svelteHTML.createElement("h4", {});  }
         { svelteHTML.createElement("div", { "class":`shortcut-list`,});
             for(let path of __sveltets_2_ensureArray(shortcuts.recents)){path;
             { svelteHTML.createElement("button", {      "title":path,"disabled":loading,"onclick":() => load(path),});path === view?.path;folderName(path); }
          } { svelteHTML.createElement("p", { "class":`dim`,});     }
         }
       }
     }
   { svelteHTML.createElement("div", { "class":`browser`,});
    if(loading){
       { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":4,"height":28,"label":`folders`,}});} }
    } else if (error){
       { svelteHTML.createElement("div", {   "class":`err pad`,"role":`alert`,});
         { svelteHTML.createElement("div", {});   { svelteHTML.createElement("code", { ...__sveltets_2_empty({"data-testid":`attempted-folder`}),});lastAttempt.path || '~'; } }
         { svelteHTML.createElement("div", {});error; }
       }
      if(view){ { svelteHTML.createElement("button", {   "class":`btn retry`,"onclick":() => view && load(view.path),});     }}
       { const $$_button3 = svelteHTML.createElement("button", {    "class":`btn retry`,"onclick":() => load(lastAttempt.path, lastAttempt.index),});retryElement = $$_button3;  }
    } else if (view){
      if(view.parent !== null){
         { svelteHTML.createElement("button", {   "class":`row`,"onclick":() => load(view!.parent!),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":14,}});}
           { svelteHTML.createElement("span", { "class":`grow`,});  }
           { svelteHTML.createElement("span", { "class":`dim`,});  }
         }
      }
      if(shown.length > 0){
         { const $$_div3 = svelteHTML.createElement("div", {                "class":`folder-grid`,"role":`grid`,"tabindex":0,"aria-label":`Folder entries. Arrow keys choose a row and Open or Use; Enter activates.`,"aria-rowcount":shown.length,"aria-colcount":2,"aria-activedescendant":cellId(position.index, position.action),"onkeydown":gridKey,});gridElement = $$_div3;
           { const $$_tsiLlautriV4C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV4 = new $$_tsiLlautriV4C({ target: __sveltets_2_any(), props: {              "items":shown,"estimateHeight":32,"pinnedIndex":position.index,"scrollIndex":position.index,scrollVersion,"tabindex":-1,"key":entry => entry.path,"class":`picker-rows`,row:(e: FsEntry, index: number) => { async ()/*Ωignore_positionΩ*/ => {
               { svelteHTML.createElement("div", {     "class":`row-wrap`,"role":`row`,"aria-rowindex":index + 1,});
                 { svelteHTML.createElement("div", {         "class":`entry-cell`,"role":`gridcell`,"id":cellId(index, 0),"aria-colindex":1,"aria-selected":position.index === index && position.action === 0,});
                   { svelteHTML.createElement("button", {      "class":`row`,"tabindex":-1,"onclick":() => activate(index, 0),});!e.is_dir;
                     { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":e.is_dir ? (e.is_git_repo ? 'branch' : 'folder') : 'file',"size":14,}});}
                     { svelteHTML.createElement("span", { "class":`grow ellipsis`,});e.name; }
                    if(e.is_git_repo){ { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Git`,}});}}
                    if(!e.is_dir){ { svelteHTML.createElement("span", { "class":`dim use-file`,});  }}
                   }
                 }
                if(gridAction(e, 1, gitOnly)){
                   { svelteHTML.createElement("div", {       "role":`gridcell`,"id":cellId(index, 1),"aria-colindex":2,"aria-selected":position.index === index && position.action === 1,});
                     { svelteHTML.createElement("button", {       "class":`use`,"tabindex":-1,"title":`Use this folder`,"onclick":() => activate(index, 1),});  }
                   }
                }
               }
            };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV4.$$prop_def;/*Ωignore_endΩ*/
            
           VirtualList}
         }
      }
      if(shown.length === 0){
         { svelteHTML.createElement("div", { "class":`dim pad`,});
          if(view.entries.length === 0){
            files ? 'No items here.' : 'No subfolders here.';
          }else{ }
         }
      }
    }
   }

   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FolderPicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FolderPicker__SvelteComponent_ = ReturnType<typeof FolderPicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default FolderPicker__SvelteComponent_;