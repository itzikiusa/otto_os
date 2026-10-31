///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
// Canvas Studio entry. Left: scene list. Right: an infinite Mermaid board that
// renders the scene's agent-edited `.mermaid` source (full rich diagrams), or a
// hero to start a new canvas. You never write Mermaid — you describe what you
// want in the Assistant and the agent edits the file; the board re-renders live.
import { untrack } from 'svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { canvas } from '../../lib/stores/canvas.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { toasts } from '../../lib/toast.svelte';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import SceneList from './SceneList.svelte';
import ExcalidrawCanvas from './ExcalidrawCanvas.svelte';
import MermaidCanvas from './MermaidCanvas.svelte';
import D2Canvas from './D2Canvas.svelte';
import ConversationPanel from './ConversationPanel.svelte';
import type { CanvasFormat } from '../../lib/canvas/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Three modes the user picks at creation: Excalidraw (a fully editable board,
  // the agent writes canvas.json), Mermaid (rich auto-rendered diagrams of any
  // kind — flowchart / sequence / class — the agent writes canvas.mermaid), or
  // D2 (modern declarative diagrams — architecture / sequence / SQL tables — the
  // agent writes canvas.d2).
  const isExcalidraw = $derived(canvas.format === 'excalidraw');
  const isD2 = $derived(canvas.format === 'd2');

  // Phone collapses the scene list once a board is open (more room).
  const isPhone = $derived(viewport.isPhone);
  const openTitle = $derived(
    canvas.scenes.find((scene) => scene.id === canvas.currentId)?.title,
  );

  // The board exposes generate()/isGenerating() for agent drawing.
  let editor = $state<
    { generate: (p: string) => Promise<boolean>; isGenerating: () => boolean } | undefined
  >(undefined);
  // The Assistant panel (the agent shell + Ask-AI input) — opens on demand.
  let showConvo = $state(false);
  // The scene list's width — drag / ←→ on the divider, remembered across visits.
  let listW = $state(loadPaneWidth('canvas.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let showTabletScenes = $state(true);
  let workspaceEmpty: HTMLDivElement | undefined = $state();

  // Canvas is global — list the user's scenes across all workspaces. A failure
  // lands in `canvas.listError` and renders inline with Retry.
  $effect(() => {
    void canvas.loadScenes().catch(() => {});
  });

  // List/detail: with scenes, open on one (the remembered scene, else the
  // first) instead of the "start a new canvas" hero. Not on phone — opening a
  // board collapses the scene list there, which is the first screen. Also
  // re-picks after the open scene is deleted. A failed open stops the loop
  // (it shows inline with Retry).
  //
  // `picking` (plain, untracked) is true while ANY open this page started is in
  // flight — the auto-pick's own, or a deep link's — so the auto-pick never
  // races a deep link to the remembered scene. It is checked LAST: returning
  // before the reactive reads would drop this effect's dependencies, and it
  // would then never re-run (e.g. never re-pick after the open scene is deleted).
  let picking = false;
  $effect(() => {
    if (viewport.isPhone) return;
    if (canvas.currentId || canvas.pendingOpenId || canvas.loadError) return;
    if (canvas.listLoading || canvas.scenes.length === 0) return;
    if (picking) return;
    const id = initialSelection('canvas', canvas.scenes, (s) => s.id);
    if (!id) return;
    picking = true;
    untrack(() => {
      void canvas
        .open(id)
        .catch(() => {})
        .finally(() => (picking = false));
    });
  });
  $effect(() => {
    if (canvas.currentId) rememberSelection('canvas', canvas.currentId);
  });

  /** The last failed scene open — Retry re-opens it. */
  function retryOpen(): void {
    const id = canvas.loadErrorId;
    if (id) void canvas.open(id).catch(() => {});
  }

  // Honor a deep-link request (e.g. Discovery-Chat "Open in Canvas").
  $effect(() => {
    const id = canvas.pendingOpenId;
    if (id) {
      canvas.pendingOpenId = null;
      picking = true;
      void canvas
        .open(id)
        .catch(() => {})
        .finally(() => (picking = false));
    }
  });

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

  /** Header "New scene ▾": pick the format from the shared (viewport-clamped) menu. */
  function newSceneMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Excalidraw board', icon: 'shapes', action: () => void createBlank('excalidraw') },
      { label: 'Mermaid diagram', icon: 'branch', action: () => void createBlank('mermaid') },
      { label: 'D2 diagram', icon: 'layers', action: () => void createBlank('d2') },
    ]);
  }

  /** No scenes anywhere → no empty list pane; the hero's mode cards are the CTA. */
  const noScenes = $derived(!canvas.listLoading && !canvas.listError && canvas.scenes.length === 0);
  /** The list failed with nothing to show — the main pane owns the error + Retry. */
  const listFailed = $derived(!!canvas.listError && canvas.scenes.length === 0);

  /** Phone push-nav back: save a pending edit first (closeScene drops the
   *  autosave timer), then return to the scene list. */
  async function backToList(): Promise<void> {
    if (canvas.dirty) await canvas.saveNow().catch(() => {});
    showConvo = false;
    canvas.closeScene();
  }

  // ⌘K: the page's verbs (new scene per format; the assistant while a scene
  // is open). Group = the module name.
  $effect(() => {
    if (!ws.currentId) return registry.register('canvas', []);
    const open = !!canvas.currentId;
    return registry.register('canvas', [
      { id: 'canvas.new-excalidraw', title: 'New Excalidraw board', group: 'Canvas', keywords: 'create scene whiteboard draw', run: () => void createBlank('excalidraw') },
      { id: 'canvas.new-mermaid', title: 'New Mermaid diagram', group: 'Canvas', keywords: 'create scene flowchart sequence', run: () => void createBlank('mermaid') },
      { id: 'canvas.new-d2', title: 'New D2 diagram', group: 'Canvas', keywords: 'create scene architecture', run: () => void createBlank('d2') },
      ...(open
        ? [{ id: 'canvas.assistant', title: 'Open the canvas assistant', group: 'Canvas', keywords: 'agent ai redraw chat', run: () => (showConvo = true) }]
        : []),
    ]);
  });

  async function createBlank(format: CanvasFormat = 'excalidraw'): Promise<void> {
    try {
      const created = await canvas.create('Untitled canvas', blankDoc(format));
      await canvas.open(created.id);
    } catch (e) {
      toastError('Couldn’t create canvas', e);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`canvas-shell`,});

  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {      "title":openTitle || 'Canvas',"subtitle":`Diagrams an agent draws and refines as you chat`,"crumbs":isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }],leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(isPhone && canvas.currentId){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void backToList(),"aria-label":`Back to scenes`,"title":`Back to scenes`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
       }
    } else if (viewport.isTablet && canvas.currentId && !noScenes && !listFailed){
       { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => (showTabletScenes = !showTabletScenes),"aria-label":showTabletScenes ? 'Hide scenes' : 'Show scenes',"title":showTabletScenes ? 'Hide scenes' : 'Show scenes',"aria-expanded":showTabletScenes,"aria-controls":`canvas-scene-list`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`sidebar`,"size":16,}});}
       }
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    
    if(ws.currentId && !noScenes){
       { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":newSceneMenu,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`canvas-new-scene`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}    { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
 PageHeader}
   for(let [id, error] of __sveltets_2_ensureArray(Object.entries(canvas.docSaveErrors))){id;
   { svelteHTML.createElement("div", {   "class":`save-error`,"role":`alert`,});
     { svelteHTML.createElement("span", {});  canvas.scenes.find(scene => scene.id === id)?.title ?? 'scene';        error; }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void canvas.open(id).catch(() => {}),});  }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void canvas.retryDoc(id).catch(() => {}),});  }
   }
}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
if(!ws.currentId){
   { const $$_div2 = svelteHTML.createElement("div", {  "class":`canvas-page empty-ws`,});workspaceEmpty = $$_div2;
     { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`shapes`,"title":`Select a workspace`,"body":`Canvas scenes live in a workspace. Pick or create one to start drawing.`,"actionLabel":`Choose workspace`,"onaction":() => {
        const trigger = workspaceEmpty?.querySelector('button');
        trigger?.focus();
        if (!ws.workspaces.length) { ui.newWorkspaceOpen = true; return; }
        ctxMenu.showAt(trigger!, [
          ...ws.workspaces.map(w => ({ label: w.name, icon: 'folder', action: () => void ws.select(w.id) })),
          { separator: true },
          { label: 'Add workspace…', icon: 'plus', action: () => (ui.newWorkspaceOpen = true) },
        ], { filter: true, filterPlaceholder: 'Find a workspace…' });
      },}});}
   }
}else{
  
   { svelteHTML.createElement("div", {   "class":`canvas-page`,});isPhone;viewport.isTablet;
     { svelteHTML.createElement("aside", {         "id":`canvas-scene-list`,"class":`scenes`,"style":`--list-pane-w:${listW}px`,});(isPhone && canvas.currentId) || (viewport.isTablet && canvas.currentId && !showTabletScenes) || noScenes || listFailed;isPhone && !canvas.currentId;
       { const $$_tsiLenecS4C = __sveltets_2_ensureComponent(SceneList); new $$_tsiLenecS4C({ target: __sveltets_2_any(), props: {}});}
     }
    if(!isPhone && !viewport.isTablet && !noScenes && !listFailed){
       { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`canvas.listW`,"label":`Resize the scenes list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
    }

     { svelteHTML.createElement("section", {  "class":`main`,});isPhone && !canvas.currentId && !noScenes && !listFailed && !canvas.loadError;
      if(canvas.loadError && canvas.scene && canvas.currentId){
        
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "what":`that scene`,"variant":`compact`,"error":canvas.loadError,"empty":true,"onretry":retryOpen,}});}
      }
      if(listFailed){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {            "what":`scenes`,"variant":`page`,"loading":canvas.listLoading,"error":canvas.listError,"empty":true,"onretry":() => void canvas.loadScenes().catch(() => {}),}});}
      } else if (canvas.loadError && !(canvas.scene && canvas.currentId)){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "what":`this scene`,"variant":`page`,"error":canvas.loadError,"empty":true,"onretry":retryOpen,}});}
      } else if (canvas.scene && canvas.currentId){
        
        canvas.currentId; {
           { svelteHTML.createElement("div", {  "class":`editor-split`,});showConvo;
             { svelteHTML.createElement("div", { "class":`editor-host`,});
              if(isExcalidraw){
                 { const $$_savnaCwardilacxE6C = __sveltets_2_ensureComponent(ExcalidrawCanvas); const $$_savnaCwardilacxE6 = new $$_savnaCwardilacxE6C({ target: __sveltets_2_any(), props: {  }});editor = $$_savnaCwardilacxE6;}
              } else if (isD2){
                 { const $$_savnaC2D6C = __sveltets_2_ensureComponent(D2Canvas); const $$_savnaC2D6 = new $$_savnaC2D6C({ target: __sveltets_2_any(), props: {  }});editor = $$_savnaC2D6;}
              }else{
                 { const $$_savnaCdiamreM6C = __sveltets_2_ensureComponent(MermaidCanvas); const $$_savnaCdiamreM6 = new $$_savnaCdiamreM6C({ target: __sveltets_2_any(), props: {  }});editor = $$_savnaCdiamreM6;}
              }
              if(!showConvo){
                
                 { svelteHTML.createElement("div", { "class":`ai-bar`,});
                   { svelteHTML.createElement("button", {     "class":`ai-fab`,"onclick":() => (showConvo = true),"title":`Open the assistant: describe a change and the agent redraws the scene`,});
                     { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":14,}});}  
                   }
                 }
              }
             }
            if(showConvo){
               { svelteHTML.createElement("aside", {  "class":`convo-panel`,});isPhone;
                 { const $$_lenaPnoitasrevnoC6C = __sveltets_2_ensureComponent(ConversationPanel); new $$_lenaPnoitasrevnoC6C({ target: __sveltets_2_any(), props: {   editor,"onclose":() => (showConvo = false),}});}
               }
            }
           }
        }
      } else if (!noScenes){
        
          { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {    "what":`scenes`,"variant":`page`,"loading":true,"empty":true,}});}
      }else{
         { svelteHTML.createElement("div", { "class":`hero`,});
           { svelteHTML.createElement("h2", {});    }
           { svelteHTML.createElement("p", { "class":`sub`,});
                            
                
           }
           { svelteHTML.createElement("div", { "class":`modes`,});
             { svelteHTML.createElement("button", {   "class":`mode`,"onclick":() => createBlank('excalidraw'),});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":24,}});}
               { svelteHTML.createElement("span", { "class":`m-title`,});  }
               { svelteHTML.createElement("span", { "class":`m-sub`,});          }
             }
             { svelteHTML.createElement("button", {   "class":`mode`,"onclick":() => createBlank('mermaid'),});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":24,}});}
               { svelteHTML.createElement("span", { "class":`m-title`,});  }
               { svelteHTML.createElement("span", { "class":`m-sub`,});          }
             }
             { svelteHTML.createElement("button", {   "class":`mode`,"onclick":() => createBlank('d2'),});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`layers`,"size":24,}});}
               { svelteHTML.createElement("span", { "class":`m-title`,});  }
               { svelteHTML.createElement("span", { "class":`m-sub`,});         }
             }
           }
         }
      }
     }
   }
}
 PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CanvasPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CanvasPage__SvelteComponent_ = ReturnType<typeof CanvasPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default CanvasPage__SvelteComponent_;