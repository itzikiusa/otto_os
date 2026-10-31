///<reference types="svelte" />
;
import { vault } from './vault.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { assetPath, restoreVaultRevision, restoreVaultTrash, vaultHistory, vaultRevision, vaultTrash } from '../../lib/api/vault';
import { ApiError, authedBlobUrl } from '../../lib/api/client';
import type { VaultRevision, VaultRevisionDetail, VaultTrashEntry } from '../../lib/api/types';
import DiffView from '../../lib/components/DiffView.svelte';
import { toasts } from '../../lib/toast.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { loadErrorText } from '../../lib/loadError';
import Icon from '../../lib/components/Icon.svelte';

;type $$ComponentProps =  {mode: 'trash' | 'history'};function $$render() {

  
  
  
  
  
  
  
  
  
  

  let {mode}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let trash = $state<VaultTrashEntry[]>([]);
  // Raw: pages of 200 are replaced/appended wholesale, never mutated in place.
  let revisions = $state.raw<VaultRevision[]>([]);
  let selected = $state<VaultRevisionDetail | null>(null);
  let currentHash = $state<string | null>(null);
  let busy = $state(false);
  let more = $state(false);
  let loading = $state(false);
  let error = $state('');
  let destination = $state<Record<string, string>>({});
  let seq = 0;
  let detailSeq = 0;
  const visibleRevisions = $derived(revisions.filter(r => !vault.historySince || Date.parse(r.created_at) >= Date.parse(vault.historySince)));

  async function load(id: number, path: string) {
    const request = ++seq, workspace = vault.wsId;
    loading = true; error = ''; selected = null; detailSeq++;
    trash = []; revisions = []; more = false;
    try {
      if (mode === 'trash') {
        const data = await vaultTrash(workspace, id);
        if (request === seq) trash = data;
      } else {
        const data = await vaultHistory(workspace, id, path);
        if (request === seq) {revisions = data; more = data.length === 200;}
      }
    } catch (e) { if (request === seq) error = loadErrorText(e); }
    finally { if (request === seq) loading = false; }
  }
  // Debounced: the path filter is `bind:value`, so an undebounced effect
  // re-read every revision's history on each keystroke.
  $effect(() => {
    const id = vault.current?.id, path = vault.historyPath;
    void vault.wsId; void mode;
    if (!id) { seq++; detailSeq++; trash = []; revisions = []; selected = null; loading = false; return; }
    const timer = setTimeout(() => void load(id, path), 250);
    return () => clearTimeout(timer);
  });

  async function loadOlder() {
    if (!vault.current || !revisions.length || loading) return;
    const id = vault.current.id, request = seq;
    loading = true;
    try {
      const data = await vaultHistory(vault.wsId, id, vault.historyPath, revisions[revisions.length - 1].id);
      if (request === seq) { revisions = [...revisions, ...data]; more = data.length === 200; }
    } catch (e) { if (request === seq) error = loadErrorText(e); }
    finally { if (request === seq) loading = false; }
  }

  async function selectRevision(entry: VaultRevision) {
    if (!vault.current) return;
    const request = ++detailSeq, id = vault.current.id, workspace = vault.wsId;
    error = ''; selected = null; currentHash = null;
    try {
      const detail = await vaultRevision(workspace, id, entry.id);
      let hash = '';
      try {
        const url = await authedBlobUrl(assetPath(workspace, id, entry.path));
        try {
          const bytes = await (await fetch(url)).arrayBuffer();
          const digest = await crypto.subtle.digest('SHA-256', bytes);
          hash = [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('');
        } finally { URL.revokeObjectURL(url); }
      } catch (e) { if (!(e instanceof ApiError && e.status === 404)) throw e; }
      if (request !== detailSeq || vault.current?.id !== id || vault.wsId !== workspace) return;
      selected = detail; currentHash = hash;
    } catch (e) { if (request === detailSeq) error = loadErrorText(e); }
  }

  async function restoreTrash(entry: VaultTrashEntry) {
    if (!vault.current || busy) return;
    const id = vault.current.id, workspace = vault.wsId, request = seq;
    busy = true; error = '';
    try {
      const result = await restoreVaultTrash(workspace, id, entry.id, destination[entry.id]?.trim() || undefined);
      toasts.success('Restored', result.path);
      if (request !== seq || vault.current?.id !== id || vault.wsId !== workspace) return;
      await load(id, vault.historyPath);
      await vault.refreshTree();
      await vault.refreshStatus();
    } catch (e) { if (request === seq) error = loadErrorText(e); }
    finally { busy = false; }
  }

  async function restoreRevision(version: 'before' | 'after') {
    if (!vault.current || !selected || currentHash === null || busy) return;
    const entry = selected, id = vault.current.id, workspace = vault.wsId, request = seq;
    busy = true; error = '';
    try {
      await restoreVaultRevision(workspace, id, entry.id, version, currentHash);
      toasts.success(`Restored the ${version} version`, entry.path);
      if (request !== seq || vault.current?.id !== id || vault.wsId !== workspace) return;
      await load(id, vault.historyPath);
      await vault.refreshTree();
      await vault.refreshOpenNote();
    } catch (e) { if (request === seq) error = loadErrorText(e); }
    finally { busy = false; }
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`recovery`,"aria-label":mode === 'trash' ? 'Vault trash' : 'Vault edit history',});
   { svelteHTML.createElement("header", {}); { svelteHTML.createElement("h2", {});mode === 'trash' ? 'Trash' : 'Edit history'; } { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":loading,"onclick":() => vault.current && void load(vault.current.id, vault.historyPath),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}  } }
  if(error){
     { svelteHTML.createElement("div", {   "role":`alert`,"class":`error`,}); { svelteHTML.createElement("span", {});  mode === 'trash' ? 'the trash' : 'the edit history'; error; }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":loading,"onclick":() => vault.current && void load(vault.current.id, vault.historyPath),});  } }
  }
  if(mode === 'trash'){
     { svelteHTML.createElement("p", {});               }
    if(loading){ { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":3,"height":36,"label":`the trash`,}});}} else if (trash.length === 0 && !error){ { svelteHTML.createElement("p", {});                }}
       for(let entry of __sveltets_2_ensureArray(trash)){entry.id;
       { svelteHTML.createElement("article", { "class":`trash-entry`,});
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("strong", {});entry.original_path; } { svelteHTML.createElement("small", { "title":new Date(entry.deleted_at).toLocaleString(),}); rel(entry.deleted_at);  entry.kind === 'dir' ? 'Folder' : 'File'; } }
         { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {        "dir":`ltr`,"aria-label":`Restore path for ${entry.original_path}`,"placeholder":entry.original_path,"bind:value":destination[entry.id],});/*Ωignore_startΩ*/() => destination[entry.id] = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":() => void restoreTrash(entry),});  }
       }
    }
  }else{
     { svelteHTML.createElement("p", {});              }
     { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "dir":`ltr`,"aria-label":`History file path`,"bind:value":vault.historyPath,"placeholder":`All files`,});/*Ωignore_startΩ*/() => vault.historyPath = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
    if(vault.historySince){ { svelteHTML.createElement("p", {});  new Date(vault.historySince).toLocaleString();  { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => {vault.historySince = ''; vault.persistView();},});   } }}
     { svelteHTML.createElement("div", { "class":`history-layout`,});
       { svelteHTML.createElement("nav", { "aria-label":`Saved revisions`,});
        if(loading){ { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":4,"height":36,"label":`the edit history`,}});}} else if (visibleRevisions.length === 0 && !error){ { svelteHTML.createElement("p", {});             }}
           for(let entry of __sveltets_2_ensureArray(visibleRevisions)){entry.id;
           { svelteHTML.createElement("button", {      "class":`rev`,"aria-current":selected?.id === entry.id ? 'true' : undefined,"onclick":() => void selectRevision(entry),});selected?.id === entry.id;
             { svelteHTML.createElement("strong", {});entry.path; } { svelteHTML.createElement("small", { "title":new Date(entry.created_at).toLocaleString(),});rel(entry.created_at);  entry.reason;entry.committed ? '' : ' · write not confirmed'; }
           }
        }
        if(more){ { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":loading,"onclick":() => void loadOlder(),});   }}
       }
       { svelteHTML.createElement("div", { "class":`revision-detail`,});
        if(selected){
           { svelteHTML.createElement("h3", {});selected.path; }
           { svelteHTML.createElement("div", { "class":`actions`,}); { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy || selected.before === null || currentHash === null,"onclick":() => void restoreRevision('before'),});   } { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy || currentHash === null,"onclick":() => void restoreRevision('after'),});   } }
          if((selected.before?.length ?? 0) + selected.after.length <= 500_000){
             { const $$_weiVffiD3C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD3C({ target: __sveltets_2_any(), props: {        "before":selected.before ?? '',"after":selected.after,"mode":`line`,"contextLines":4,}});}
          }else{
             { svelteHTML.createElement("p", {});               }
             { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});  } { svelteHTML.createElement("pre", {});selected.before ?? '(File did not exist)'; } }
             { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});  } { svelteHTML.createElement("pre", {});selected.after; } }
          }
        }else{ { svelteHTML.createElement("p", {});       }}
       }
     }
  }
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RecoveryView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RecoveryView__SvelteComponent_ = ReturnType<typeof RecoveryView__SvelteComponent_>;
/*Ωignore_endΩ*/export default RecoveryView__SvelteComponent_;