///<reference types="svelte" />
;// Canvases linked to this story (canvas scenes whose story_id == this story).
// Lists them with a click-to-open deep-link into the Canvas module, and a
// "New canvas" action that creates one already linked to the story.

import { rel } from '../../lib/stores/now.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { api } from '../../lib/api/client';
import { canvas } from '../../lib/stores/canvas.svelte';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import type { CanvasFormat, CanvasSceneSummary } from '../../lib/canvas/types';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
;

  interface Props {
    storyId: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { storyId }: Props = $props();

  let scenes = $state<CanvasSceneSummary[]>([]);
  let loading = $state(false);
  let creating = $state(false);
  let picking = $state(false);
  let linkingId = $state('');
  /** A failed load — inline with Retry (never a silent "No canvases linked"). */
  let loadError = $state<string | null>(null);

  async function load(): Promise<void> {
    loading = true;
    try {
      scenes = await api.get<CanvasSceneSummary[]>(`/product/stories/${storyId}/linked-canvases`);
      loadError = null;
    } catch (e) {
      // Keep the last good list (LoadState shows a stale bar over it).
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (storyId) void load();
  });

  // Existing canvases that aren't already linked to this story (the picker).
  const candidates = $derived(canvas.scenes.filter((c) => !scenes.some((l) => l.id === c.id)));

  // The picker's own load: a failure shows inline with Retry, and the
  // "No other canvases" line waits for a finished, successful load.
  let pickerLoading = $state(false);
  let pickerError = $state<string | null>(null);

  async function loadPicker(): Promise<void> {
    pickerLoading = true;
    pickerError = null;
    try {
      await canvas.loadScenes();
    } catch (e) {
      pickerError = loadErrorText(e);
    } finally {
      pickerLoading = false;
    }
  }

  async function openPicker(): Promise<void> {
    picking = !picking;
    if (picking && !canvas.scenes.length) await loadPicker();
  }

  async function link(sceneId: string): Promise<void> {
    if (linkingId) return;
    linkingId = sceneId;
    try {
      await canvas.updateMeta(sceneId, { story_id: storyId });
      picking = false;
      await load();
      toasts.success('Canvas linked', 'Attached to this story');
    } catch (e) {
      toastError('Couldn’t link the canvas', e);
    } finally {
      linkingId = '';
    }
  }

  function open(id: string): void {
    canvas.pendingOpenId = id;
    router.go('canvas');
  }

  /** A blank doc for a freshly-created scene of the given format — mirrors
   *  CanvasPage's own `blankDoc()` (Canvas module's "New" action). */
  function blankDoc(format: CanvasFormat): unknown {
    if (format === 'excalidraw') {
      return {
        type: 'otto-canvas',
        version: 1,
        format: 'excalidraw',
        source: JSON.stringify({ type: 'excalidraw', version: 2, source: 'otto', elements: [] }),
      };
    }
    if (format === 'd2') {
      return { type: 'otto-canvas', version: 1, format: 'd2', source: '' };
    }
    return { type: 'otto-canvas', version: 1, format: 'mermaid', source: '' };
  }

  /** "New ▾": pick the format from the shared menu, anchored under the button
   *  (same rows as the Canvas module's own "New scene" menu). */
  function newMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'Excalidraw board', icon: 'shapes', action: () => void createLinked('excalidraw') },
      { label: 'Mermaid diagram', icon: 'branch', action: () => void createLinked('mermaid') },
      { label: 'D2 diagram', icon: 'layers', action: () => void createLinked('d2') },
    ], { align: 'end' });
  }

  async function createLinked(format: CanvasFormat): Promise<void> {
    if (creating) return;
    creating = true;
    try {
      const created = await canvas.create('Untitled canvas', blankDoc(format), storyId);
      canvas.pendingOpenId = created.id;
      router.go('canvas');
      toasts.success('Canvas created', 'Linked to this story');
    } catch (e) {
      toastError('Couldn’t create canvas', e);
    } finally {
      creating = false;
    }
  }

  function label(s: CanvasSceneSummary): string {
    return s.section ? `${s.section.replace(/\//g, ' / ')} · ${s.title}` : s.title;
  }

  /** Icon + short label for a scene's source format. */
  function formatMeta(f: CanvasFormat): { icon: IconName; label: string } {
    switch (f) {
      case 'excalidraw': return { icon: 'shapes', label: 'Excalidraw' };
      case 'd2': return { icon: 'layers', label: 'D2' };
      default: return { icon: 'branch', label: 'Mermaid' };
    }
  }

  // The shared, reactive relative time (content.md §7).
  const ago = (iso: string): string => rel(iso);
;
async () => {

 { svelteHTML.createElement("div", { "class":`linked-canvases`,});
   { svelteHTML.createElement("div", { "class":`lc-head`,});
     { svelteHTML.createElement("span", { "class":`lc-title`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":14,}});}  }
     { svelteHTML.createElement("div", { "class":`lc-actions`,});
       { svelteHTML.createElement("button", {    "class":`lc-btn`,"onclick":openPicker,});picking;
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plug`,"size":12,}});}  
       }
       { svelteHTML.createElement("button", {       "class":`lc-btn primary`,"onclick":newMenu,"disabled":creating,"aria-haspopup":`menu`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} creating ? 'Creating…' : 'New';
       }
     }
   }

   { svelteHTML.createElement("p", { "class":`lc-sub`,});
                { svelteHTML.createElement("strong", {});  } 
        { svelteHTML.createElement("strong", {});  }       
   }

  if(picking){
     { svelteHTML.createElement("div", { "class":`lc-picker`,});
      if(pickerLoading || pickerError){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {              "what":`your canvases`,"variant":`compact`,"rows":2,"loading":pickerLoading,"error":pickerError,"empty":!canvas.scenes.length,"onretry":() => void loadPicker(),}});}
      }
      if(pickerLoading || (pickerError && !canvas.scenes.length)){
        
      } else if (!candidates.length){
         { svelteHTML.createElement("p", { "class":`lc-empty`,});     }
      }else{
           for(let c of __sveltets_2_ensureArray(candidates)){c.id;
           { svelteHTML.createElement("button", {     "class":`lc-cand`,"onclick":() => link(c.id),"disabled":!!linkingId,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":12,}});}
             { svelteHTML.createElement("span", { "class":`lc-name`,});label(c); }
            if(linkingId === c.id){ { svelteHTML.createElement("span", { "class":`lc-busy`,});  }}else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}}
           }
        }
      }
     }
  }
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"what":`linked canvases`,"variant":`compact`,"rows":2,loading,"error":loadError,"empty":!scenes.length,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("p", { "class":`lc-empty`,});           }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("ul", { "class":`lc-list`,});
         for(let s of __sveltets_2_ensureArray(scenes)){s.id;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {     "class":`lc-row`,"onclick":() => open(s.id),"title":`Open in Canvas`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":13,}});}
             { svelteHTML.createElement("span", { "class":`lc-name`,});label(s); }
            if(s.format){
               { svelteHTML.createElement("span", {   "class":`lc-format-chip`,"title":formatMeta(s.format).label,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":formatMeta(s.format).icon,"size":12,}});}
                formatMeta(s.format).label;
               }
            }
             { svelteHTML.createElement("span", { "class":`lc-updated`,});ago(s.updated_at); }
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":13,}});}
           }
         }
      }
     }
   LoadState}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LinkedCanvases__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LinkedCanvases__SvelteComponent_ = ReturnType<typeof LinkedCanvases__SvelteComponent_>;
/*Ωignore_endΩ*/export default LinkedCanvases__SvelteComponent_;