///<reference types="svelte" />
;
import Badge from '../../lib/components/Badge.svelte';
// Per-session Canvas references: the scenes attached to the focused agent
// session (via GET/POST/DELETE /sessions/{id}/canvas-refs). Each row shows a
// format chip + an expandable inline SVG preview (mermaid/d2 — rendered from
// the scene's live source; Excalidraw rows show a static "board" card
// instead of mounting the heavy editor here). Live updates arrive via
// canvasRefsBus (attach/detach) and canvasDocBus (an open scene's source
// changing while an agent edits it).
import { ws } from '../../lib/stores/workspace.svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { ui } from '../../lib/stores/ui.svelte';
import { router } from '../../lib/router.svelte';
import { canvasRefsBus, canvasDocBus } from '../../lib/events.svelte';
import { api } from '../../lib/api/client';
import { rel } from '../../lib/stores/now.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import type { CanvasDoc, CanvasFormat, CanvasScene, CanvasSceneSummary } from '../../lib/canvas/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const session = $derived(ws.activeSession);
  const sessionId = $derived(session?.id ?? null);

  let refs = $state<CanvasSceneSummary[]>([]);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  // Which session `refs` belongs to. On a session switch the previous
  // session's rows are dropped at once (not shown under the new session until
  // the fetch lands), and a slower response for an older session is ignored.
  let refsFor: string | null = null;
  let loadSeq = 0;

  async function load(): Promise<void> {
    const sid = sessionId;
    const seq = ++loadSeq;
    if (!sid) {
      refs = [];
      refsFor = null;
      return;
    }
    if (refsFor !== sid) {
      refs = [];
      refsFor = sid;
    }
    loading = true;
    loadError = null;
    try {
      const rows = await api.get<CanvasSceneSummary[]>(`/sessions/${sid}/canvas-refs`);
      if (seq === loadSeq) refs = rows;
    } catch (e) {
      if (seq === loadSeq) loadError = loadErrorText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  // Load whenever the focused session changes.
  $effect(() => {
    const sid = sessionId;
    if (sid) void load();
  });

  // Reload on attach/detach for THIS session (from anywhere: this panel, the
  // Canvas module's "Attach to session" flow, or an MCP write tool).
  $effect(() => {
    const t = canvasRefsBus.tick;
    if (t > 0 && canvasRefsBus.sessionId === sessionId) void load();
  });

  // -- expandable inline preview -----------------------------------------

  let expanded = $state<Record<string, boolean>>({});
  interface Preview {
    key: string;
    svg?: string;
    error?: string;
    loading?: boolean;
  }
  let previews = $state<Record<string, Preview>>({});

  async function renderFromDoc(sceneId: string, doc: CanvasDoc, key: string): Promise<void> {
    const format: CanvasFormat = doc.format ?? 'mermaid';
    if (format === 'excalidraw') return; // static card — nothing to render
    previews = { ...previews, [sceneId]: { key, loading: true } };
    const renderId = `canvas-panel-${sceneId}`;
    const source = doc.source ?? '';
    // Both renderers theme by scheme — a light Mermaid figure on the dark
    // panel was unreadable (same defect as ActionCard, S18-307).
    const dark = ui.resolvedScheme === 'dark';
    // The bridges pull the render queue. Load one only when a preview expands,
    // so opening the panel does not fetch Mermaid or D2.
    try {
      const out =
        format === 'd2'
          ? await (await import('../canvas/d2')).renderD2(renderId, source, { sketch: doc.sketch, dark })
          : await (await import('../../lib/render/mermaid')).renderMermaid(renderId, source, { dark });
      previews = { ...previews, [sceneId]: { key, svg: out.svg, error: out.error } };
    } catch (e) {
      previews = { ...previews, [sceneId]: { key, error: loadErrorText(e) } };
    }
  }

  async function togglePreview(ref: CanvasSceneSummary): Promise<void> {
    const open = !expanded[ref.id];
    expanded = { ...expanded, [ref.id]: open };
    if (!open || ref.format === 'excalidraw') return;

    const key = `${ref.id}:${ref.updated_at}:${ui.resolvedScheme}`;
    if (previews[ref.id]?.key === key) return; // cached — same scene revision

    previews = { ...previews, [ref.id]: { key, loading: true } };
    try {
      const scene = await api.get<CanvasScene>(`/canvas/scenes/${ref.id}`);
      let doc: CanvasDoc | null = null;
      try {
        doc = JSON.parse(scene.doc_json) as CanvasDoc;
      } catch {
        doc = null;
      }
      if (!doc || typeof doc.source !== 'string') {
        previews = { ...previews, [ref.id]: { key, error: 'No renderable source' } };
        return;
      }
      await renderFromDoc(ref.id, doc, key);
    } catch (e) {
      previews = { ...previews, [ref.id]: { key, error: loadErrorText(e) } };
    }
  }

  // Live re-render: an open, expanded, referenced scene's source changed
  // while an agent edits it (canvas_updated). Render straight from the pushed
  // doc — no refetch needed.
  $effect(() => {
    const sceneId = canvasDocBus.sceneId;
    const tick = canvasDocBus.tick;
    if (tick === 0 || !sceneId) return;
    if (!expanded[sceneId] || !refs.some((r) => r.id === sceneId)) return;
    const doc = canvasDocBus.doc as CanvasDoc | null;
    if (!doc || typeof doc.source !== 'string') return;
    void renderFromDoc(sceneId, doc, `live:${tick}`);
  });

  // -- row actions ---------------------------------------------------------

  async function openInCanvas(sceneId: string): Promise<void> {
    try {
      const { canvas } = await import('../../lib/stores/canvas.svelte');
      canvas.pendingOpenId = sceneId;
      router.go('canvas');
    } catch (e) {
      toastError('Couldn’t open Canvas', e);
    }
  }

  async function detach(sceneId: string): Promise<void> {
    const sid = sessionId;
    if (!sid) return;
    try {
      await api.del(`/sessions/${sid}/canvas-refs/${sceneId}`);
      refs = refs.filter((r) => r.id !== sceneId);
    } catch (e) {
      toastError('Couldn’t detach the canvas', e);
    }
  }

  // -- footer: attach existing / create new --------------------------------

  let attachOpen = $state(false);
  let attachQuery = $state('');
  let allScenes = $state<CanvasSceneSummary[]>([]);
  let attachLoading = $state(false);
  let attachError = $state<string | null>(null);
  let creating = $state(false);

  // Re-list on every open: scenes created since the last open (here, on the
  // Canvas page or by an agent) must show up, and a failed fetch must not
  // read as "no scenes".
  async function loadAllScenes(): Promise<void> {
    attachLoading = true;
    attachError = null;
    try {
      allScenes = await api.get<CanvasSceneSummary[]>('/canvas/scenes');
    } catch (e) {
      attachError = loadErrorText(e);
    } finally {
      attachLoading = false;
    }
  }

  async function toggleAttachPicker(): Promise<void> {
    attachOpen = !attachOpen;
    if (attachOpen) await loadAllScenes();
  }

  const attachCandidates = $derived(
    allScenes
      .filter((s) => s.workspace_id === session?.workspace_id)
      .filter((s) => !refs.some((r) => r.id === s.id))
      .filter((s) => s.title.toLowerCase().includes(attachQuery.trim().toLowerCase())),
  );

  async function attachExisting(sceneId: string): Promise<void> {
    const sid = sessionId;
    if (!sid) return;
    try {
      await api.post(`/sessions/${sid}/canvas-refs`, { scene_id: sceneId });
      attachOpen = false;
      attachQuery = '';
      await load();
    } catch (e) {
      toastError('Couldn’t attach the canvas', e);
    }
  }

  async function createNewScene(): Promise<void> {
    const sid = sessionId;
    const wsId = session?.workspace_id;
    if (!sid || !wsId || creating) return;
    creating = true;
    try {
      const doc: CanvasDoc = { type: 'otto-canvas', version: 1, format: 'mermaid', source: '' };
      const created = await api.post<CanvasScene>(`/workspaces/${wsId}/canvas/scenes`, {
        title: 'Untitled canvas',
        doc,
      });
      await api.post(`/sessions/${sid}/canvas-refs`, { scene_id: created.id });
      await openInCanvas(created.id);
      await load();
    } catch (e) {
      toastError('Couldn’t create canvas', e);
    } finally {
      creating = false;
    }
  }

  // -- display helpers -------------------------------------------------------

  const FORMAT_LABEL: Record<CanvasFormat, string> = {
    mermaid: 'mermaid',
    d2: 'd2',
    excalidraw: 'board',
  };

  function formatOf(ref: CanvasSceneSummary): CanvasFormat {
    return ref.format ?? 'mermaid';
  }

  function label(s: CanvasSceneSummary): string {
    return s.section ? `${s.section.replace(/\//g, ' / ')} · ${s.title}` : s.title;
  }

;
async () => {

if(!session){
   { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`shapes`,"title":`No session selected`,"body":`Open or focus an agent session to see its referenced canvases.`,}});}
}else{
   { svelteHTML.createElement("div", { "class":`canvas-panel`,});
     { svelteHTML.createElement("div", { "class":`cp-list`,});
      
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`this session’s canvases`,"variant":`compact`,loading,"error":loadError,"empty":refs.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`shapes`,"title":`No canvases referenced`,"body":`Attach one below, or ask the agent to draw a diagram.`,}});}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
        
         { svelteHTML.createElement("ul", { "class":`refs`,});
             for(let ref of __sveltets_2_ensureArray(refs)){ref.id;
             { svelteHTML.createElement("li", { "class":`ref-row`,});
              
               { svelteHTML.createElement("div", { "class":`ref-main`,});
                 { svelteHTML.createElement("button", {     "class":`ref-body`,"aria-expanded":!!expanded[ref.id],"onclick":() => void togglePreview(ref),});
                   { svelteHTML.createElement("span", { "class":`ref-chevron`,});
                    if(formatOf(ref) !== 'excalidraw'){
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":expanded[ref.id] ? 'chevronDown' : 'chevronRight',"size":12,}});}
                    }
                   }
                   { svelteHTML.createElement("span", { "class":`ref-text`,});
                     { svelteHTML.createElement("span", { "class":`ref-title`,});label(ref); }
                     { svelteHTML.createElement("span", { "class":`ref-meta`,});
                       { const $$_egdaB9C = __sveltets_2_ensureComponent(Badge); new $$_egdaB9C({ target: __sveltets_2_any(), props: {    "tone":formatOf(ref) === 'd2' ? 'accent' : 'neutral',"label":FORMAT_LABEL[formatOf(ref)],}});}
                       { svelteHTML.createElement("span", {   "class":`ref-time mono`,"title":new Date(ref.updated_at).toLocaleString(),});rel(ref.updated_at); }
                     }
                   }
                 }
                 { svelteHTML.createElement("div", { "class":`ref-actions`,});
                   { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Open in Canvas`,"aria-label":`Open in Canvas`,"onclick":() => openInCanvas(ref.id),});
                     { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":13,}});}
                   }
                   { svelteHTML.createElement("button", {         "class":`icon-btn`,"title":`Detach from this session`,"aria-label":`Detach from this session`,"onclick":() => detach(ref.id),});
                     { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});}
                   }
                 }
               }
              if(expanded[ref.id]){
                 { svelteHTML.createElement("div", { "class":`ref-preview`,});
                  if(formatOf(ref) === 'excalidraw'){
                     { svelteHTML.createElement("div", { "class":`board-card`,});
                       { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":16,}});}
                       { svelteHTML.createElement("span", {});        }
                     }
                  } else if (previews[ref.id]?.loading){
                     { svelteHTML.createElement("p", { "class":`empty-line dim`,});  }
                  } else if (previews[ref.id]?.error){
                     { svelteHTML.createElement("p", { "class":`empty-line dim`,});    previews[ref.id]?.error; }
                  } else if (previews[ref.id]?.svg){
                    
                     { svelteHTML.createElement("div", { "class":`svg-wrap`,}); previews[ref.id]?.svg; }
                  }else{
                     { svelteHTML.createElement("p", { "class":`empty-line dim`,});  }
                  }
                 }
              }
             }
          }
         }
       LoadState}
     }

     { svelteHTML.createElement("div", { "class":`cp-footer`,});
      if(attachOpen){
         { svelteHTML.createElement("div", { "class":`attach-picker`,});
           { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`attach-search`,"placeholder":`Filter scenes to attach…`,"aria-label":`Filter scenes to attach`,"bind:value":attachQuery,"spellcheck":`false`,});/*Ωignore_startΩ*/() => attachQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(attachLoading || attachError){
             { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          "what":`scenes`,"variant":`compact`,"loading":attachLoading,"error":attachError,"empty":true,"onretry":() => void loadAllScenes(),}});}
          } else if (attachCandidates.length === 0){
             { svelteHTML.createElement("p", { "class":`empty-line dim`,});
              attachQuery.trim()
                ? `No scenes match "${attachQuery.trim()}".`
                : 'No other scenes in this workspace. Use New scene to create one.';
             }
          }else{
             { svelteHTML.createElement("ul", { "class":`candidates`,});
                 for(let c of __sveltets_2_ensureArray(attachCandidates)){c.id;
                 { svelteHTML.createElement("li", {});
                   { svelteHTML.createElement("button", {   "class":`candidate-btn`,"onclick":() => attachExisting(c.id),});label(c); }
                 }
              }
             }
          }
         }
      }
       { svelteHTML.createElement("div", { "class":`cp-footer-actions`,});
         { svelteHTML.createElement("button", {      "class":`footer-btn`,"aria-expanded":attachOpen,"onclick":toggleAttachPicker,});attachOpen;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plug`,"size":12,}});}  
         }
         { svelteHTML.createElement("button", {     "class":`footer-btn`,"disabled":creating,"onclick":createNewScene,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
         }
       }
     }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CanvasPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CanvasPanel__SvelteComponent_ = ReturnType<typeof CanvasPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default CanvasPanel__SvelteComponent_;