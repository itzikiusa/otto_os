///<reference types="svelte" />
;
  // The currently-mounted canvas, held at MODULE scope (only one is open) so an
  // in-flight generate() survives a brief remount.
  let liveId: string | null = null;
;;// The Mermaid canvas: a live preview of the scene's `.mermaid` SOURCE PLUS a full
// editor. You edit the diagram three ways, all writing the SAME `canvas.mermaid`
// file: (1) the agent via "Ask Otto", (2) directly in the Code panel (the Mermaid
// source, with live preview), (3) — nothing is converted to Excalidraw; this stays
// Mermaid. Mermaid's own renderer draws the full rich spectrum (subgraphs, classDef
// colours, every shape). Pan/zoom the preview.


import { PAN_LABEL } from './panKeys';
import { PanZoom, type Transform } from './pointerZoom';
import { toastAgentEdit } from './agentUndo';
import { toastError } from '../../lib/toastError';
import { announce } from '../../lib/announce.svelte';
import { onMount, onDestroy, tick, untrack } from 'svelte';
import { canvas } from '../../lib/stores/canvas.svelte';
import { canvasDocBus } from '../../lib/events.svelte';
import { toasts } from '../../lib/toast.svelte';
import { renderMermaid } from '../../lib/render/mermaid';
import { svgToPngDownload } from './export';
import { copyText } from '../../lib/clipboard';
import { ui } from '../../lib/stores/ui.svelte';
import type { CanvasDoc, CanvasFormat } from '../../lib/canvas/types';
import { docFromAssist } from './assistDoc';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
;

  interface Props {
    readonly?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { readonly = false }: Props = $props();

  // The scene THIS editor is mounted for — saves target THIS id, never
  // canvas.currentId (which on a scene switch already points at the next scene).
  const sceneId = canvas.currentId;
  const saveContext = canvas.saveContext;

  let surface = $state<HTMLDivElement | null>(null);
  let content = $state<HTMLDivElement | null>(null);
  let svgHtml = $state('');
  let renderError = $state('');
  let notMermaid = $state(false);
  let generating = $state(false);
  let codeOpen = $state(false);

  // Pan/zoom transform (driven by PanZoom — pointerZoom.ts).
  let view = $state<Transform>({ scale: 1, tx: 0, ty: 0 });
  let userAdjusted = false;
  let dragging = $state(false);
  const pz = new PanZoom({
    surface: () => surface,
    get: () => view,
    set: (t) => (view = t),
    adjusted: () => userAdjusted,
    setAdjusted: (on) => (userAdjusted = on),
    onDragging: (on) => (dragging = on),
  });
  let renderToken = 0;
  let lastRendered = '';
  let lastRenderedDark = false;

  // The agent + the user share ONE file. Save the edited Mermaid SOURCE to THIS
  // scene (guarded: if the scene switched, drop the stale save).
  async function saveMermaid(value: string): Promise<void> {
    if (canvas.saveContext !== saveContext || canvas.currentId !== sceneId || !sceneId) return;
    const doc: CanvasDoc = { type: 'otto-canvas', version: 1, format: 'mermaid' as CanvasFormat, source: value };
    canvas.source = value; // drives the live preview re-render
    canvas.rawDoc = doc;
    try {
      await canvas.persistDoc(sceneId, doc, saveContext);
    } catch (e) {
      if (canvas.currentId === sceneId)
        toastError('Couldn’t save the canvas', e);
    }
  }
  let codeTimer: ReturnType<typeof setTimeout> | null = null;
  /** The code-pane text a pending debounce has not saved yet. */
  let pendingCode: string | null = null;
  function onCode(value: string): void {
    if (canvas.saveContext !== saveContext) return;
    if (codeTimer) clearTimeout(codeTimer);
    pendingCode = value;
    // Unsaved typing wins over live agent pushes (ingestDoc skips while dirty).
    if (sceneId) canvas.stageDoc(sceneId, { type: 'otto-canvas', version: 1, format: 'mermaid', source: value }, saveContext);
    codeTimer = setTimeout(() => {
      codeTimer = null;
      pendingCode = null;
      void saveMermaid(value);
    }, 500);
  }
  /** Unmount (scene switch / navigation) inside the debounce window: PUT the
   *  last typed text straight to THIS scene — `saveMermaid` would drop it, since
   *  currentId already points at the next scene. */
  function flushPendingCode(): void {
    if (codeTimer) clearTimeout(codeTimer);
    codeTimer = null;
    if (pendingCode === null || !sceneId) return;
    const doc: CanvasDoc = { type: 'otto-canvas', version: 1, format: 'mermaid' as CanvasFormat, source: pendingCode };
    pendingCode = null;
    void canvas.persistDoc(sceneId, doc, saveContext).catch((e: unknown) =>
      toastError('Couldn’t save the canvas', e),
    );
  }

  /** Render the current source to SVG (Mermaid native) and auto-fit. */
  async function renderNow(src: string): Promise<void> {
    const token = ++renderToken;
    const text = (src ?? '').trim();
    if (!text) {
      svgHtml = '';
      renderError = '';
      notMermaid = false;
      lastRendered = '';
      return;
    }
    // Guard: a scene whose source is Excalidraw JSON (e.g. mislabelled) is not
    // Mermaid — show a clear message instead of a raw mermaid parse error.
    if (text.startsWith('{') && /"(type|elements)"\s*:/.test(text.slice(0, 80))) {
      notMermaid = true;
      svgHtml = '';
      renderError = '';
      return;
    }
    notMermaid = false;
    // The pasteboard follows the app scheme, so the diagram does too (the
    // light theme's dark labels vanished on the dark board).
    const dark = ui.resolvedScheme === 'dark';
    if (text === lastRendered && dark === lastRenderedDark) return; // nothing changed
    const out = await renderMermaid(`cv-${sceneId ?? 'x'}-${token}`, text, { dark, isStale: () => token !== renderToken });
    if (token !== renderToken) return; // superseded
    if (out.error || !out.svg) {
      renderError = out.error || 'Couldn’t render the diagram';
      announce(`Diagram error: ${renderError}`);
      return; // keep the last good SVG on screen
    }
    renderError = '';
    lastRendered = text;
    lastRenderedDark = dark;
    svgHtml = out.svg;
    await tick();
    pz.sizeSvg(content);
  }

   function fitView(): void {
    pz.fitView();
  }

  function fileBase(): string {
    return (canvas.scene?.title ?? 'canvas').replace(/[^\w.-]+/g, '-');
  }
  /** Exports always use the LIGHT theme: a downloaded diagram lands in docs
   *  and slides (the PNG gets a white backing), whatever the app scheme. */
  async function exportSvg(): Promise<SVGSVGElement | null> {
    const src = (canvas.source ?? '').trim();
    if (!src) return null;
    const out = await renderMermaid(`cv-export-${sceneId ?? 'x'}-${++renderToken}`, src, { dark: false });
    if (!out.svg) {
      toasts.error('Couldn’t export the diagram', out.error);
      return null;
    }
    const host = document.createElement('div');
    host.innerHTML = out.svg;
    return host.querySelector('svg');
  }
  async function downloadSvg(): Promise<void> {
    const svg = await exportSvg();
    if (!svg) return;
    const blob = new Blob([new XMLSerializer().serializeToString(svg)], { type: 'image/svg+xml' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${fileBase()}.svg`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  async function downloadPng(): Promise<void> {
    const svg = await exportSvg();
    if (!svg) return;
    svgToPngDownload(svg, `${fileBase()}.png`);
  }
  async function copySource(): Promise<void> {
    const ok = await copyText(canvas.source ?? '');
    if (ok) toasts.success('Copied', 'Mermaid source copied to clipboard.');
    else toasts.error('Couldn’t copy', 'Select the text and copy it manually.');
  }

  /** Ask the agent to edit this scene's .mermaid source. */
   async function generate(prompt: string): Promise<boolean> {
    const p = prompt.trim();
    if (!p || generating) return false;
    generating = true;
    userAdjusted = false;
    // The result belongs to THIS scene even if the user switches away while
    // the agent works (the server commits it there).
    const sceneId = canvas.currentId;
    const saveContext = canvas.saveContext;
    canvas.pushConvo('user', p, sceneId);
    try {
      const res = await canvas.assist(p, 'flow');
      // Read before finishAssistTurn: a stop or a save during the turn must not
      // offer Undo back to the pre-turn diagram.
      const kept = canvas.assistKeptSaved(res.note, sceneId);
      // A stop's body is the scene as read at stop time. Reload the stored
      // scene instead, so a save that landed after that read is the one shown.
      if ((res.note ?? '').startsWith('Stopped')) {
        if (sceneId) await canvas.reloadAfterStop(sceneId);
        canvas.pushConvo('assistant', res.note || 'Saved changes stay on the board.', sceneId);
        if (canvas.currentId !== sceneId || canvas.saveContext !== saveContext) {
          toasts.info('Saved changes kept', 'The scene you asked from still has your saved edits.');
        } else toasts.info('Saved changes kept', res.note || 'Your edits stay on the board.');
        return true;
      }
      const doc = docFromAssist(res, 'mermaid');
      const src = doc?.source ?? '';
      if (!src.trim()) {
        canvas.pushConvo('assistant', res.note || 'No diagram was produced.', sceneId);
        if (kept) toasts.info('Saved changes kept', 'Your edits stay on the board.');
        else toasts.info('Nothing to draw', res.note || 'The agent did not return a diagram.');
        return false;
      }
      if (canvas.currentId !== sceneId || canvas.saveContext !== saveContext) {
        if (kept) toasts.info('Saved changes kept', 'The scene you asked from still has your saved edits.');
        else toasts.success('Ask Otto finished', 'The diagram was saved to the scene you asked from.');
        return true;
      }
      // ingestDoc drops a source that is not the document saved during this turn.
      const applied = canvas.ingestDoc(doc!, sceneId);
      canvas.pushConvo('assistant', res.note || (kept ? 'Saved changes stay on the board.' : 'Updated the canvas.'), sceneId);
      if (kept) toasts.info('Saved changes kept', res.note || 'Your edits stay on the board.');
      else if (applied) toastAgentEdit(sceneId, 'Otto edited the diagram', res.note || 'Diagram updated.');
      void canvas.refreshSession();
      return true;
    } catch (e) {
      canvas.pushConvo('assistant', `Failed: ${e instanceof Error ? e.message : String(e)}`, sceneId);
      toastError('Couldn’t ask Otto', e);
      return false;
    } finally {
      canvas.finishAssistTurn(sceneId);
      generating = false;
    }
  }
   function isGenerating(): boolean {
    return generating;
  }

  // Live edits: translate the canvas_updated push into the store.
  // Tracks ONLY the bus tick; the store consumes each push once, so a save
  // (dirty → false) can no longer re-apply a stale push (r3-02-01).
  $effect(() => {
    const t = canvasDocBus.tick;
    untrack(() => canvas.ingestPush(t, canvasDocBus.sceneId, canvasDocBus.doc));
  });

  // Render whenever the source changes (open / generate / live / code edit).
  $effect(() => {
    const src = canvas.source; // dependency
    const _scheme = ui.resolvedScheme; // dependency: re-theme on a scheme switch
    void renderNow(src ?? '');
  });

  onMount(() => {
    liveId = canvas.currentId;
    const observer = new ResizeObserver(() => { if (!userAdjusted && svgHtml) pz.fit(); });
    if (surface) observer.observe(surface);
    return () => observer.disconnect();
  });
  onDestroy(() => {
    flushPendingCode();
    if (liveId === canvas.currentId) liveId = null;
  });
;
async () => {



 { svelteHTML.createElement("div", { "class":`board`,});
   { svelteHTML.createElement("div", { "class":`lanes`,});
    if(codeOpen && !readonly){
       { svelteHTML.createElement("aside", { "class":`code-pane`,});
         { svelteHTML.createElement("div", { "class":`code-head`,});
           { svelteHTML.createElement("span", {}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":13,}});}   }
           { svelteHTML.createElement("span", { "class":`code-hint`,});     }
         }
         { svelteHTML.createElement("div", { "class":`code-body`,});
           { const $$_rotidEedoC4C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC4C({ target: __sveltets_2_any(), props: {            "path":`canvas.mermaid`,"root":"","content":canvas.source ?? '',"readOnly":false,"completionSource":() => null,"onchange":onCode,}});}
         }
       }
    }

     { svelteHTML.createElement("div", { "class":`preview-wrap`,});
      
      
       { const $$_div3 = svelteHTML.createElement("div", {                         "class":`surface`,"role":`application`,"tabindex":0,"aria-label":PAN_LABEL,"title":`Drag to pan · scroll to zoom · arrow keys pan when focused (Shift = bigger steps)`,"onkeydown":(e) => pz.key(e),"onwheel":(e) => pz.wheel(e),"onpointerdown":(e) => pz.pointerDown(e),"onpointermove":(e) => pz.pointerMove(e),"onpointerup":(e) => pz.pointerUp(e),"onpointercancel":(e) => pz.pointerUp(e),});dragging;surface = $$_div3;
        if(svgHtml){
           { const $$_div4 = svelteHTML.createElement("div", {      "class":`content`,"style":`transform: translate(${view.tx}px, ${view.ty}px) scale(${view.scale}); transform-origin: 0 0;`,});content = $$_div4;
            
             svgHtml;
           }
        } else if (notMermaid){
           { svelteHTML.createElement("div", { "class":`empty`,});
             { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {      "icon":`shapes`,"title":`This canvas holds Excalidraw content`,"body":`It’s labeled Mermaid but contains an Excalidraw scene. Create a new Excalidraw canvas to edit those shapes.`,}});}
           }
        } else if (!renderError){
           { svelteHTML.createElement("div", { "class":`empty`,});
             { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {      "icon":`branch`,"title":`Mermaid diagram`,"body":`Describe it in Ask Otto, or open Code to edit the Mermaid yourself — both write the same diagram and render here live.`,}});}
           }
        }

        if(renderError){
           { svelteHTML.createElement("div", { "class":`err`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}   renderError;
           }
        }
       }

       { svelteHTML.createElement("div", { "class":`mode-bar`,});
         { svelteHTML.createElement("span", { "class":`mode-chip`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});} if(readonly){ } }
        if(!readonly){
           { svelteHTML.createElement("button", {          "class":`code-toggle`,"aria-pressed":codeOpen,"onclick":() => (codeOpen = !codeOpen),"title":`Edit the Mermaid source`,});codeOpen;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} 
           }
        }
       }

      if(svgHtml){
         { svelteHTML.createElement("div", { "class":`zoombar`,});
           { svelteHTML.createElement("button", {     "onclick":() => pz.zoomBy(1 / 1.2),"title":`Zoom out`,"aria-label":`Zoom out`,});
             { svelteHTML.createElement("svg", {               "width":`14`,"height":`14`,"viewBox":`0 0 16 16`,"fill":`none`,"stroke":`currentColor`,"stroke-width":`1.4`,"stroke-linecap":`round`,"aria-hidden":`true`,}); { svelteHTML.createElement("path", {  "d":`M3.5 8h9`,});} }
           }
           { svelteHTML.createElement("button", {       "class":`pct`,"onclick":fitView,"title":`Fit to screen`,"aria-label":`Zoom ${Math.round(view.scale * 100)}% — fit to screen`,});Math.round(view.scale * 100);  }
           { svelteHTML.createElement("button", {     "onclick":() => pz.zoomBy(1.2),"title":`Zoom in`,"aria-label":`Zoom in`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
           { svelteHTML.createElement("span", { "class":`sep`,}); }
           { svelteHTML.createElement("button", {     "onclick":() => void downloadSvg(),"title":`Download SVG`,"aria-label":`Download SVG`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":14,}});}
           }
           { svelteHTML.createElement("button", {     "onclick":() => void downloadPng(),"title":`Download PNG`,"aria-label":`Download PNG`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":14,}});}
           }
           { svelteHTML.createElement("button", {     "onclick":() => void copySource(),"title":`Copy source`,"aria-label":`Copy source`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":14,}});}
           }
         }
      }
     }
   }
 }


};
return { props: {} as any as Props, exports: {} as any as { fitView: typeof fitView,generate: typeof generate,isGenerating: typeof isGenerating }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MermaidCanvas__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MermaidCanvas__SvelteComponent_ = ReturnType<typeof MermaidCanvas__SvelteComponent_>;
/*Ωignore_endΩ*/export default MermaidCanvas__SvelteComponent_;