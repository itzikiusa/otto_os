///<reference types="svelte" />
;
import { rowMenu as rowMenuKeys } from '../../lib/rowMenu';
// Workbench file list: search, pinned-first live docs grouped by scope
// (this workspace / all workspaces), and the trash view (Restore / Delete
// forever). Every row has a context menu (ctxMenu).
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { WorkbenchDoc } from '../../lib/api/types';
import { workbench, sortDocs } from './workbench.svelte';
;

  interface Props {
    onopen: (id: string) => void;
    onnew: () => void;
    onrename: (id: string) => void;
  };function $$render() {
 const row/*Ωignore_positionΩ*/ = (d: WorkbenchDoc)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("li", {});
     {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {               "class":`wb-row`,...__sveltets_2_empty({"data-testid":`wb-file-row`}),...__sveltets_2_empty({"data-doc-id":d.id}),"title":d.folder ? `${d.folder}/${d.name}` : d.name,"onclick":() => onopen(d.id),"oncontextmenu":(e) => rowMenu(e, d),});workbench.active === d.id;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":iconFor(d.language),"size":13,}});}
       { svelteHTML.createElement("span", { "class":`wb-name`,});d.name; }
      if(d.scope === 'global'){ { svelteHTML.createElement("span", {       "class":`wb-scope`,"role":`img`,"title":`Shows in every workspace`,"aria-label":`All workspaces`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});} }}
      if(workbench.isDirty(d.id)){ { svelteHTML.createElement("span", {       "class":`wb-dot`,"role":`img`,"title":`Unsaved changes`,"aria-label":`Unsaved changes`,}); }}
       { svelteHTML.createElement("span", { "class":`wb-when`,}); { const $$_emiTleR3C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR3C({ target: __sveltets_2_any(), props: {  "iso":d.updated_at,}});} }
     }}
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onopen, onnew, onrename }: Props = $props();

  let query = $state('');

  function matches(d: WorkbenchDoc, q: string): boolean {
    if (!q) return true;
    const hay = `${d.name} ${d.folder} ${d.tags.join(' ')} ${d.language}`.toLowerCase();
    return q
      .toLowerCase()
      .split(/\s+/)
      .filter(Boolean)
      .every((t) => hay.includes(t));
  }

  const live = $derived(sortDocs(workbench.docs).filter((d) => matches(d, query.trim())));
  const pinned = $derived(live.filter((d) => d.pinned));
  const rest = $derived(live.filter((d) => !d.pinned));
  const restHere = $derived(rest.filter((d) => d.scope !== 'global'));
  const restGlobal = $derived(rest.filter((d) => d.scope === 'global'));
  /** Label the scope groups only when both exist: one group needs no title. */
  const splitScopes = $derived(restHere.length > 0 && restGlobal.length > 0);
  const trashed = $derived(workbench.trash.filter((d) => matches(d, query.trim())));

  function iconFor(lang: string): IconName {
    if (lang === 'image') return 'image';
    if (lang === 'md') return 'note';
    if (lang === 'sql') return 'db';
    return 'file';
  }

  function sizeLabel(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  async function act(label: string, fn: () => Promise<unknown>): Promise<void> {
    try {
      await fn();
    } catch (e) {
      toasts.error(`Couldn’t ${label}`, loadErrorText(e));
    }
  }

  function rowMenu(e: MouseEvent, d: WorkbenchDoc): void {
    e.preventDefault();
    const items: MenuItem[] = [
      { label: 'Open', icon: 'file', action: () => onopen(d.id) },
      { label: 'Rename…', icon: 'edit', action: () => onrename(d.id) },
      {
        label: d.pinned ? 'Unpin' : 'Pin to top',
        icon: 'pin',
        action: () => void act(d.pinned ? 'unpin' : 'pin', () => workbench.patchMeta(d.id, { pinned: !d.pinned })),
      },
      { label: 'Duplicate', icon: 'copy', action: () => void act('duplicate', () => workbench.duplicate(d.id)) },
      d.scope === 'global'
        ? {
            label: 'Keep in this workspace only',
            icon: 'folder',
            action: () => void act('change where it shows', () => workbench.setScope(d.id, 'workspace')),
          }
        : {
            label: 'Show in all workspaces',
            icon: 'globe',
            action: () => void act('change where it shows', () => workbench.setScope(d.id, 'global')),
          },
      { separator: true },
      {
        label: 'Move to trash', // ui-guards: allow — reversible (restore from the trash), so no confirm
        icon: 'trash',
        danger: true,
        action: () =>
          void act('move to trash', async () => {
            await workbench.moveToTrash(d.id);
            toasts.success('Moved to trash', `“${d.name}” and its history can be restored from the trash.`);
          }),
      },
    ];
    ctxMenu.show(e, items);
  }

  function trashMenu(e: MouseEvent, d: WorkbenchDoc): void {
    e.preventDefault();
    ctxMenu.show(e, [
      { label: 'Restore', icon: 'undo', action: () => void restore(d) },
      { separator: true },
      { label: 'Delete forever…', icon: 'trash', danger: true, action: () => void purge(d) },
    ]);
  }

  async function restore(d: WorkbenchDoc): Promise<void> {
    await act('restore', async () => {
      await workbench.restoreFromTrash(d.id);
      toasts.success('Restored', `“${d.name}” is back with its full history.`);
    });
  }

  async function purge(d: WorkbenchDoc): Promise<void> {
    const ok = await confirmer.ask(
      `“${d.name}” and its entire edit history will be erased. This cannot be undone.`,
      { title: 'Delete forever?', confirmLabel: 'Delete forever', danger: true },
    );
    if (!ok) return;
    await act('delete', () => workbench.purge(d.id));
  }

  function toggleTrash(): void {
    workbench.showTrash = !workbench.showTrash;
    if (workbench.showTrash) void workbench.loadTrash();
  }
;
async () => {



 { svelteHTML.createElement("div", { "class":`wb-files`,});
   { svelteHTML.createElement("div", { "class":`wb-files-head`,});
     { svelteHTML.createElement("label", { "class":`wb-search`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { svelteHTML.createElement("input", {            "dir":`auto`,"type":`search`,"placeholder":workbench.showTrash ? 'Search trash' : 'Search files',"aria-label":workbench.showTrash ? 'Search trash' : 'Search files',...__sveltets_2_empty({"data-testid":`wb-search`}),"bind:value":query,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onnew,"aria-label":`New file`,"title":`New file (also in the toolbar)`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}
     }
   }

   { svelteHTML.createElement("div", { "class":`wb-files-body`,});
    if(workbench.showTrash){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`trash`,"variant":`compact`,"loading":workbench.trashLoading,"error":workbench.trashError,"empty":workbench.trash.length === 0,"onretry":() => void workbench.loadTrash(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { svelteHTML.createElement("p", { "class":`wb-hint`,});                  }
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
        
         { svelteHTML.createElement("ul", {   "class":`wb-list`,"aria-label":`Trash`,});
             for(let d of __sveltets_2_ensureArray(trashed)){d.id;
             {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('li')));{ svelteHTML.createElement("li", __sveltets_2_union($$action_0), {      "class":`wb-trash-row`,...__sveltets_2_empty({"data-testid":`wb-trash-row`}),"oncontextmenu":(e) => trashMenu(e, d),});
               { svelteHTML.createElement("span", {   "class":`wb-name`,"title":d.name,});d.name; }
              if(d.scope === 'global'){ { svelteHTML.createElement("span", {       "class":`wb-scope`,"role":`img`,"title":`Shows in every workspace`,"aria-label":`All workspaces`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});} }}
               { svelteHTML.createElement("span", { "class":`wb-when`,});sizeLabel(d.size); }
               { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void restore(d),"title":`Restore with its full history`,});  }
               { svelteHTML.createElement("button", {       "class":`icon-btn wb-purge`,"onclick":() => void purge(d),"aria-label":`Delete ${d.name} forever`,"title":`Delete forever`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
               }
             }}
          }
          if(trashed.length === 0 && workbench.trash.length > 0){ { svelteHTML.createElement("li", { "class":`wb-hint`,});  }}
         }
       LoadState}
    }else{
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`files`,"variant":`compact`,"loading":workbench.loading && !workbench.loaded,"error":workbench.error,"empty":workbench.docs.length === 0,"onretry":() => void workbench.loadList(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               "icon":`workbench`,"title":`No scratch files yet`,"body":`Scripts, JSON, notes, diagrams — every save is kept in history.`,"actionLabel":`New file`,"actionIcon":`plus`,"actionKind":`secondary`,"onaction":onnew,}});}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
        
        if(pinned.length > 0){
           { svelteHTML.createElement("h4", { "class":`wb-sec`,});  }
           { svelteHTML.createElement("ul", {   "class":`wb-list`,"aria-label":`Pinned files`,});
               for(let d of __sveltets_2_ensureArray(pinned)){d.id;;__sveltets_2_ensureSnippet(row(d));}
           }
        }
        if(splitScopes){
           { svelteHTML.createElement("h4", {   "class":`wb-sec`,"title":`Only in this workspace`,});  }
           { svelteHTML.createElement("ul", {     "class":`wb-list`,"aria-label":`Files in this workspace`,...__sveltets_2_empty({"data-testid":`wb-files-here`}),});
               for(let d of __sveltets_2_ensureArray(restHere)){d.id;;__sveltets_2_ensureSnippet(row(d));}
           }
           { svelteHTML.createElement("h4", {   "class":`wb-sec`,"title":`Show in every workspace`,});  }
           { svelteHTML.createElement("ul", {     "class":`wb-list`,"aria-label":`Files in all workspaces`,...__sveltets_2_empty({"data-testid":`wb-files-global`}),});
               for(let d of __sveltets_2_ensureArray(restGlobal)){d.id;;__sveltets_2_ensureSnippet(row(d));}
           }
        } else if (rest.length > 0){
          if(pinned.length > 0){ { svelteHTML.createElement("h4", { "class":`wb-sec`,});  }}
           { svelteHTML.createElement("ul", {   "class":`wb-list`,"aria-label":`Files`,});
               for(let d of __sveltets_2_ensureArray(rest)){d.id;;__sveltets_2_ensureSnippet(row(d));}
           }
        }
        if(live.length === 0){ { svelteHTML.createElement("p", { "class":`wb-hint`,});   query;  }}
       LoadState}
    }
   }

   { svelteHTML.createElement("button", {      "class":`wb-trash-toggle`,"onclick":toggleTrash,...__sveltets_2_empty({"data-testid":`wb-trash-toggle`}),});workbench.showTrash;
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":workbench.showTrash ? 'chevronLeft' : 'trash',"size":12,}});}
    workbench.showTrash ? 'Back to files' : 'Trash';
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FileList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FileList__SvelteComponent_ = ReturnType<typeof FileList__SvelteComponent_>;
/*Ωignore_endΩ*/export default FileList__SvelteComponent_;