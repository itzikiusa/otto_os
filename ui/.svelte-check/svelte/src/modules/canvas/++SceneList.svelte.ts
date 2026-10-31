///<reference types="svelte" />
;
import { rowMenu as rowMenuKeys } from '../../lib/rowMenu';
import { toastError } from '../../lib/toastError';
// Scene list — the workspace's canvas scenes grouped into collapsible SECTIONS
// (a folder path like "Platform/Staging" → sections + sub-sections), with
// search, New, click-to-open, inline RENAME, MOVE-to-section, and delete.
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { canvas } from '../../lib/stores/canvas.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { api } from '../../lib/api/client';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { rel } from '../../lib/stores/now.svelte';
import type { CanvasSceneSummary, CanvasScene } from '../../lib/canvas/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  


  let filter = $state('');
  let collapsed = $state<Record<string, boolean>>({});


  const rows = $derived.by((): CanvasSceneSummary[] => {
    const q = filter.trim().toLowerCase();
    const list = q
      ? canvas.scenes.filter(
          (s) => s.title.toLowerCase().includes(q) || (s.section ?? '').toLowerCase().includes(q),
        )
      : canvas.scenes;
    return [...list].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  });

  // Group by section path. Root (no section) first, then sections alphabetically.
  const groups = $derived.by((): [string, CanvasSceneSummary[]][] => {
    const map = new Map<string, CanvasSceneSummary[]>();
    for (const s of rows) {
      const key = (s.section ?? '').trim();
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(s);
    }
    return [...map.entries()].sort((a, b) => {
      if (a[0] === '') return -1;
      if (b[0] === '') return 1;
      return a[0].localeCompare(b[0]);
    });
  });

  /** One ⋯ / right-click menu per row instead of four always-on icons
   *  (the shared clamped ctxMenu). */
  function rowMenu(e: MouseEvent | KeyboardEvent, s: CanvasSceneSummary): void {
    e.preventDefault();
    e.stopPropagation();
    ctxMenu.show(e, [
      { label: 'Rename…', icon: 'edit', action: () => void rename(null, s) },
      { label: 'Move to section…', icon: 'folder', action: () => void move(null, s) },
      { label: 'Duplicate', icon: 'copy', action: () => void duplicate(null, s) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(null, s) },
    ]);
  }

  async function rename(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    const t = await confirmer.promptText('', {
      title: 'Rename scene',
      initial: s.title,
      confirmLabel: 'Rename',
    });
    if (t == null || t.trim() === '' || t === s.title) return;
    try {
      await canvas.updateMeta(s.id, { title: t.trim() });
    } catch (err) {
      toastError('Couldn’t rename', err);
    }
  }

  async function move(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    const sec = await confirmer.promptText('Sections group scenes in the list. Use / to nest them.', {
      title: 'Move to section',
      initial: s.section ?? '',
      placeholder: 'e.g. Platform/Staging — empty = no section',
      confirmLabel: 'Move',
    });
    if (sec == null) return;
    try {
      await canvas.updateMeta(s.id, { section: sec.trim() || null });
    } catch (err) {
      toastError('Couldn’t move', err);
    }
  }

  /** Clone a scene's full doc (fetched fresh — the summary row has no `doc_json`)
   *  into a new scene titled "<title> (copy)", in the SAME section and workspace
   *  as the original (Canvas lists scenes across all workspaces, so this can
   *  differ from the currently-active one). */
  async function duplicate(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    try {
      const row = await api.get<CanvasScene>(`/canvas/scenes/${s.id}`);
      let doc: unknown;
      try {
        doc = JSON.parse(row.doc_json);
      } catch {
        doc = undefined;
      }
      await api.post(`/workspaces/${row.workspace_id}/canvas/scenes`, {
        title: `${row.title} (copy)`,
        doc,
        section: row.section,
      });
      await canvas.loadScenes().catch(() => {});
      toasts.success('Scene duplicated', `${row.title} (copy)`);
    } catch (err) {
      toastError('Couldn’t duplicate', err);
    }
  }

  async function remove(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    const ok = await confirmer.ask(`“${s.title}” and its drawing are deleted for good. This can’t be undone.`, {
      title: 'Delete scene',
      danger: true,
      confirmLabel: 'Delete scene',
    });
    if (!ok) return;
    try {
      await canvas.del(s.id);
      toasts.success('Scene deleted', s.title);
    } catch (err) {
      toastError('Couldn’t delete', err);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`scene-list`,});
   { svelteHTML.createElement("div", { "class":`search`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
     { svelteHTML.createElement("input", {            "dir":`auto`,"type":`search`,"placeholder":`Filter scenes…`,"aria-label":`Filter scenes`,"bind:value":filter,"spellcheck":`false`,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
    if(filter){
       { svelteHTML.createElement("button", {       "class":`clear`,"onclick":() => (filter = ''),"aria-label":`Clear the filter`,"title":`Clear the filter`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`rows`,});
    if(canvas.listError){
      
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             "what":`scenes`,"variant":`compact`,"loading":canvas.listLoading,"error":canvas.listError,"empty":!canvas.scenes.length,"onretry":() => void canvas.loadScenes().catch(() => {}),}});}
    }
    if(canvas.listError && !canvas.scenes.length){
      
    } else if (canvas.listLoading && !canvas.scenes.length){
        { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {    "what":`scenes`,"variant":`compact`,"loading":true,"empty":true,}});}
    } else if (!rows.length){
       { svelteHTML.createElement("div", { "class":`hint`,});
        if(filter){   filter.trim();
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (filter = ''),});  }
        }else{  }
       }
    }else{
         for(let [section, items] of __sveltets_2_ensureArray(groups)){section;
        if(section !== ''){
           { svelteHTML.createElement("button", {         "class":`section-head`,"aria-expanded":!collapsed[section],"title":section,"onclick":() => (collapsed = { ...collapsed, [section]: !collapsed[section] }),});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":collapsed[section] ? 'chevronRight' : 'chevronDown',"size":12,}});}
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}
             { svelteHTML.createElement("span", { "class":`section-label`,});section.replace(/\//g, ' / '); }
             { svelteHTML.createElement("span", { "class":`section-count`,});items.length; }
           }
        }
        if(section === '' || !collapsed[section]){
             for(let s of __sveltets_2_ensureArray(items)){s.id;
            
             {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {            "class":`row`,"role":`group`,"aria-label":s.title,"oncontextmenu":(e) => rowMenu(e, s),});canvas.currentId === s.id;section !== '';
               { svelteHTML.createElement("button", {             "class":`row-open`,"aria-current":canvas.currentId === s.id ? 'true' : undefined,"onclick":() => void canvas.open(s.id).catch(() => {}),"ondblclick":(e) => rename(e, s),"onkeydown":(e) => {
                  if (e.key === 'F2') { e.preventDefault(); void rename(null, s); }
                  else if (e.key === 'F10' && e.shiftKey) rowMenu(e, s);
                },"title":s.title,});
                 { svelteHTML.createElement("span", { "class":`title`,});s.title; }
                 { svelteHTML.createElement("span", { "class":`when`,}); rel(s.updated_at); }
               }
               { svelteHTML.createElement("button", {            "class":`icon-btn row-more`,"onclick":(e) => rowMenu(e, s),"aria-label":`Actions for ${s.title}`,"aria-haspopup":`menu`,"title":`Rename, move, duplicate or delete`,});canvas.currentId === s.id;
                 { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
               }
             }}
          }
        }
      }
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SceneList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SceneList__SvelteComponent_ = ReturnType<typeof SceneList__SvelteComponent_>;
/*Ωignore_endΩ*/export default SceneList__SvelteComponent_;