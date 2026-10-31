///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// The stage of the artifact view (and each side of Compare): renders one
// version of a design by format with the EXISTING viewers/editors —
// DeviceFrame + a sandboxed iframe for HTML, the Canvas Mermaid/D2 renderers,
// the Design Arena's Excalidraw board and scene3d viewport — and, when
// editable, the shared CodeEditor for text sources. It never saves: every
// edit goes up through `onchange` and the view owns versions + conflicts.
//
// Untrusted content stays isolated: HTML and rendered diagrams only ever run
// inside `sandbox=""` iframes (no scripts, opaque origin); SVG renders as an
// <img>; scene3d JSON is parsed + validated before it reaches the viewport.
import { onDestroy, untrack } from 'svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import Icon from '../../lib/components/Icon.svelte';
import type { DesignArtifact } from '../../lib/api/types';
import DeviceFrame, { type DeviceKind } from '../../lib/design/DeviceFrame.svelte';
import DesignBoard from '../../lib/design/DesignBoard.svelte';
import {
    Scene3DViewport,
    emptyScene,
    parseScene,
    serializeScene,
    type Scene3dDoc,
    type Scene3dObject,
  } from '../../lib/design/scene3d';
import { renderMermaid } from '../../lib/render/mermaid';
import { renderD2 } from '../canvas/d2';
import { brandColors, contrastRatio, renderKind } from './model';
import SiteStudio from './site/SiteStudio.svelte';
import { modelScope } from './studio3d/sources';
;

  interface Props {
    artifact: DesignArtifact;
    /** Text/JSON source of the shown version (null for binaries). */
    source: string | null;
    /** Object URL of the shown version's bytes (binaries). */
    blobUrl?: string | null;
    /** The design has no version yet: a binary has nothing to load, so say so
     *  instead of "Loading…" forever. */
    noVersion?: boolean;
    readonly?: boolean;
    /** Show the source editor beside the preview (text formats). */
    showSource?: boolean;
    device?: DeviceKind;
    /** scene3d selection (the view's Hierarchy/Inspector share it). */
    selectedId?: string | null;
    /** A compact render for Compare panes (no source pane, no chrome). */
    compact?: boolean;
    /** The editor holds unsaved edits (the site publish sheet warns). */
    dirty?: boolean;
    onchange?: (source: string) => void;
  };;

  // ── otto-canvas (an imported Canvas scene): {type, format, source} ─────────
  interface CanvasDoc {
    type?: string;
    format?: string;
    source?: string;
    [k: string]: unknown;
  };;
  type Inner = { kind: 'mermaid' | 'd2' | 'excalidraw' | 'html' | 'svg' | null; src: string };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    artifact,
    source,
    dirty = undefined,
    blobUrl = null,
    noVersion = false,
    readonly = false,
    showSource = false,
    device = 'none',
    selectedId = $bindable<string | null>(null),
    compact = false,
    onchange,
  }: Props = $props()/*Ωignore_startΩ*/;selectedId;/*Ωignore_endΩ*/;

  const kind = $derived(renderKind(artifact.format));

  // "Fit" HTML: a fixed desktop viewport scaled down (never up) to the pane.
  const FIT_W = 1280;
  let fitW = $state(0);
  let fitH = $state(0);
  const fitScale = $derived(fitW > 0 ? Math.min(1, fitW / FIT_W) : 1);
  const editable = $derived(!readonly && !!onchange);

  const canvasDoc = $derived.by<CanvasDoc | null>(() => {
    if (kind !== 'canvas' || !source) return null;
    try {
      const d = JSON.parse(source);
      return d && typeof d === 'object' ? (d as CanvasDoc) : null;
    } catch {
      return null;
    }
  });

  function innerOf(doc: CanvasDoc | null, src: string | null): Inner {
    if (kind === 'canvas') {
      const f = doc?.format;
      const k = f === 'mermaid' || f === 'd2' || f === 'excalidraw' ? f : null;
      return { kind: k, src: typeof doc?.source === 'string' ? doc.source : '' };
    }
    if (kind === 'mermaid' || kind === 'd2' || kind === 'excalidraw' || kind === 'html' || kind === 'svg') {
      return { kind, src: src ?? '' };
    }
    return { kind: null, src: '' };
  }
  /** The LIVE inner source: the source pane, edits and interactive boards. */
  const inner = $derived<Inner>(innerOf(canvasDoc, source));

  // ── Preview source (debounced while typing) ───────────────────────────────
  // Replacing an iframe's `srcdoc` tears the document down and re-parses,
  // re-lays-out and re-fetches it — per keystroke that is tens-to-hundreds of
  // ms and a flashing preview. Edits typed in the source pane refresh the
  // preview once typing settles; any other change (version switch, a live
  // update, a board edit) lands immediately.
  const PREVIEW_DEBOUNCE_MS = 350;
  let preview = $state<string | null>(untrack(() => source));
  let lastTyped: string | null = null;
  $effect(() => {
    const s = source;
    if (s === null || s !== lastTyped || untrack(() => preview) === null) {
      preview = s;
      return;
    }
    const t = setTimeout(() => (preview = s), PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(t);
  });
  const previewDoc = $derived.by<CanvasDoc | null>(() => {
    if (kind !== 'canvas') return null;
    if (preview === source) return canvasDoc;
    try {
      const d = preview ? JSON.parse(preview) : null;
      return d && typeof d === 'object' ? (d as CanvasDoc) : null;
    } catch {
      return null;
    }
  });
  /** What the (non-interactive) previews render: iframe, SVG, diagrams. */
  const shown = $derived<Inner>(preview === source ? inner : innerOf(previewDoc, preview));

  /** Emit an edit of the inner source, re-wrapped for canvas docs. */
  function emitInner(next: string): void {
    if (!editable) return;
    if (kind === 'canvas' && canvasDoc) onchange?.(JSON.stringify({ ...canvasDoc, source: next }));
    else onchange?.(next);
  }

  // ── Diagram rendering (mermaid / d2) into a sandboxed frame ───────────────
  let diagramSvg = $state<string | null>(null);
  let diagramError = $state<string | null>(null);
  let renderSeq = 0;
  $effect(() => {
    const k = shown.kind;
    const src = shown.src;
    if (k !== 'mermaid' && k !== 'd2') return;
    const my = ++renderSeq;
    const t = setTimeout(async () => {
      const isStale = () => my !== renderSeq;
      const r = k === 'mermaid' ? await renderMermaid(`dh-stage-${my}`, src, { isStale }) : await renderD2(`dh-${my}`, src, { isStale });
      if (my !== renderSeq) return;
      diagramSvg = r.svg ?? null;
      diagramError = r.svg ? null : (r.error ?? 'Diagram error');
    }, 150);
    return () => clearTimeout(t);
  });

  function svgDoc(svg: string): string {
    return (
      '<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;background:#fff}' + // ui-guards: allow — rendered page canvas (the artifact’s own white page)
      'body{padding:24px;display:flex;justify-content:center}svg{max-width:100%;height:auto}</style></head>' +
      `<body>${svg}</body></html>`
    );
  }

  const svgDataUrl = $derived(
    shown.kind === 'svg' && shown.src.trim() ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(shown.src)}` : null,
  );

  // ── scene3d / models ──────────────────────────────────────────────────────
  const sceneParse = $derived(kind === 'scene3d' && source ? parseScene(source) : null);
  const sceneDoc = $derived<Scene3dDoc | null>(sceneParse?.ok ? sceneParse.doc : null);
  const modelDoc = $derived.by<Scene3dDoc | null>(() => {
    if (kind !== 'model') return null;
    const base = emptyScene();
    const hero: Scene3dObject = {
      id: 'model',
      name: artifact.title,
      type: 'gltf',
      attachment_id: artifact.id,
      position: [0, 0, 0],
      rotation: [0, 0, 0],
      scale: [1, 1, 1],
    };
    return { ...base, objects: [...base.objects, hero] };
  });
  /** gltf references: the model's own bytes, else a v2 `otto://design` src or a
   *  legacy id (a Design Hall artifact, else a Product attachment). */
  function resolveAttachment(aid: string): Promise<string> {
    if (kind === 'model' && aid === artifact.id && blobUrl) return Promise.resolve(blobUrl);
    return models.resolve(aid);
  }
  /** This stage's hold on the model blobs it shows (the cache may revoke them after). */
  const models = modelScope();
  onDestroy(() => models.release());

  // ── Brand kit ─────────────────────────────────────────────────────────────
  const brandDoc = $derived.by<Record<string, unknown> | null>(() => {
    if (kind !== 'brand' || !preview) return null;
    try {
      return JSON.parse(preview) as Record<string, unknown>;
    } catch {
      return null;
    }
  });
  const swatches = $derived(brandColors(brandDoc));

  // Source pane: which file the editor highlights as.
  const editorPath = $derived(
    ({ html: 'design.html', svg: 'design.svg', mermaid: 'design.mmd', d2: 'design.d2' } as Record<string, string>)[
      inner.kind ?? ''
    ] ?? 'design.json',
  );
  const hasSourcePane = $derived(
    !compact && showSource && (inner.kind === 'html' || inner.kind === 'svg' || inner.kind === 'mermaid' || inner.kind === 'd2' || kind === 'brand' || kind === 'json' || kind === 'scene3d'),
  );
  /** The source pane edits the INNER source for canvas docs, the document otherwise. */
  const paneSource = $derived(kind === 'canvas' ? inner.src : (source ?? ''));
  function onPane(v: string): void {
    if (!editable) return;
    if (kind === 'canvas') {
      // emitInner re-wraps the doc; remember the wrapped value it emits.
      lastTyped = canvasDoc ? JSON.stringify({ ...canvasDoc, source: v }) : null;
      emitInner(v);
    } else {
      lastTyped = v;
      onchange?.(v);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`stage`,});hasSourcePane;compact;
   { svelteHTML.createElement("div", {   "class":`view`,...__sveltets_2_empty({"data-testid":`design-stage`}),});
    if(artifact.format === 'otto-site'){
       { const $$_oidutSetiS2C = __sveltets_2_ensureComponent(SiteStudio); new $$_oidutSetiS2C({ target: __sveltets_2_any(), props: {        artifact,source,dirty,"readonly":!editable,compact,"onchange":(s) => editable && onchange?.(s),}});}
    } else if (kind === 'html' || inner.kind === 'html'){
      if(compact || device === 'none'){
        
         { const $$_div2 = svelteHTML.createElement("div", {   "class":`fit`,});fitW= $$_div2.clientWidth;fitH= $$_div2.clientHeight;
           { svelteHTML.createElement("iframe", {           "class":`doc scaled`,"title":artifact.title,"sandbox":"","srcdoc":shown.src,});__sveltets_2_ensureType(String, Number, `${FIT_W}px`);__sveltets_2_ensureType(String, Number, `${Math.ceil(fitH / fitScale)}px`);__sveltets_2_ensureType(String, Number, `scale(${fitScale})`); }
          if(!compact){ { svelteHTML.createElement("span", { "class":`fit-label`,});  FIT_W;   Math.round(fitScale * 100);  }}
         }
      }else{
         { const $$_emarFeciveD2C = __sveltets_2_ensureComponent(DeviceFrame); new $$_emarFeciveD2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },device,}});
           { svelteHTML.createElement("iframe", {     "class":`doc`,"title":artifact.title,"sandbox":"","srcdoc":shown.src,}); }
         DeviceFrame}
      }
    } else if (shown.kind === 'svg'){
       { svelteHTML.createElement("div", { "class":`paper center`,});
        if(svgDataUrl){ { svelteHTML.createElement("img", {      "class":`svg`,"src":svgDataUrl,"alt":artifact.title,});}}else{ { svelteHTML.createElement("p", { "class":`msg`,});  }}
       }
    } else if (shown.kind === 'mermaid' || shown.kind === 'd2'){
      if(diagramError){
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: diagramError, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`msg err`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});} diagramError; }}
      }
      if(diagramSvg){
         { svelteHTML.createElement("iframe", {     "class":`doc`,"title":artifact.title,"sandbox":"","srcdoc":svgDoc(diagramSvg),}); }
      } else if (!diagramError){
         { svelteHTML.createElement("p", { "class":`msg`,});  }
      }
    } else if (inner.kind === 'excalidraw'){
       { svelteHTML.createElement("div", { "class":`board`,});
         { const $$_draoBngiseD3C = __sveltets_2_ensureComponent(DesignBoard); new $$_draoBngiseD3C({ target: __sveltets_2_any(), props: {      "source":inner.src,"readonly":!editable,"onchange":emitInner,}});}
       }
    } else if (kind === 'canvas'){
       { svelteHTML.createElement("p", { "class":`msg`,});               }
    } else if (kind === 'scene3d'){
      if(sceneDoc){
         { const $$_tropweiVD3enecS2C = __sveltets_2_ensureComponent(Scene3DViewport); const $$_tropweiVD3enecS2 = new $$_tropweiVD3enecS2C({ target: __sveltets_2_any(), props: {         "doc":sceneDoc,"readonly":!editable,selectedId,"onchange":(d) => editable && onchange?.(serializeScene(d)),resolveAttachment,}});/*Ωignore_startΩ*/() => selectedId = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_tropweiVD3enecS2.$$bindings = 'selectedId';}
      }else{
         { svelteHTML.createElement("div", { "class":`msg err`,});
           { svelteHTML.createElement("p", {});     editable ? ' — fix it in the source view' : '';  }
          if(sceneParse && !sceneParse.ok){
             { svelteHTML.createElement("ul", {});
                 for(let i of __sveltets_2_ensureArray((sceneParse.issues.slice(0, 8)))){i.path + i.message;
                 { svelteHTML.createElement("li", {}); { svelteHTML.createElement("span", { "class":`mono`,});i.path || '(root)'; }  i.message; }
              }
             }
          }
         }
      }
    } else if (kind === 'model' && modelDoc && blobUrl){
       { const $$_tropweiVD3enecS2C = __sveltets_2_ensureComponent(Scene3DViewport); new $$_tropweiVD3enecS2C({ target: __sveltets_2_any(), props: {     "doc":modelDoc,"readonly":true,"onchange":() => {},resolveAttachment,}});}
    } else if (kind === 'image' && blobUrl){
       { svelteHTML.createElement("div", { "class":`paper center`,}); { svelteHTML.createElement("img", {      "class":`img`,"src":blobUrl,"alt":artifact.title,});} }
    } else if (kind === 'pdf' && blobUrl){
       { svelteHTML.createElement("object", {       "class":`doc`,"data":blobUrl,"type":`application/pdf`,"title":artifact.title,});
         { svelteHTML.createElement("p", { "class":`msg`,});      { svelteHTML.createElement("a", {   "href":blobUrl,"download":`${artifact.title}.pdf`,});   }  }
       }
    } else if (kind === 'brand'){
      if(brandDoc){
         { svelteHTML.createElement("div", { "class":`brand`,});
           { svelteHTML.createElement("h3", {});  }
          if(swatches.length){
             { svelteHTML.createElement("div", { "class":`swatches`,});
                 for(let c of __sveltets_2_ensureArray(swatches)){c.name;
                const onWhite = contrastRatio(c.value, '#ffffff');
                 { svelteHTML.createElement("div", { "class":`swatch`,});
                   { svelteHTML.createElement("span", {  "class":`chip-color`,});__sveltets_2_ensureType(String, Number, c.value); }
                   { svelteHTML.createElement("span", { "class":`sw-name`,});c.name; }
                   { svelteHTML.createElement("span", { "class":`mono`,});c.value; }
                  if(onWhite != null){
                     { svelteHTML.createElement("span", {    "class":`ratio`,"title":`Contrast as text on white`,});onWhite < 4.5;  onWhite; }
                  }
                 }
              }
             }
          }else{
             { svelteHTML.createElement("p", { "class":`msg`,});         { svelteHTML.createElement("span", { "class":`mono`,});  }    }
          }
           { svelteHTML.createElement("p", { "class":`note`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}                    { svelteHTML.createElement("a", {   "href":`#/design/brand/${encodeURIComponent(artifact.id)}`,...__sveltets_2_empty({"data-testid":`design-open-brand-kit`}),});   }  }
         }
      }else{
         { svelteHTML.createElement("p", { "class":`msg err`,});       }
      }
    } else if (kind === 'json'){
       { svelteHTML.createElement("p", { "class":`msg`,});                 }
    } else if ((kind === 'image' || kind === 'model' || kind === 'pdf') && !blobUrl){
      
       { svelteHTML.createElement("p", { "class":`msg`,});noVersion ? 'No preview yet — this design has no saved version.' : `Loading ${kind === 'pdf' ? 'the PDF' : kind === 'model' ? 'the 3D model' : 'the image'}…`; }
    }else{
       { svelteHTML.createElement("p", { "class":`msg`,});    artifact.format;   }
    }
   }
  if(hasSourcePane){
     { svelteHTML.createElement("div", {   "class":`source`,"aria-label":`Source`,});
       { const $$_rotidEedoC2C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC2C({ target: __sveltets_2_any(), props: {            "path":editorPath,"root":"","content":paneSource,"readOnly":!editable,"completionSource":() => null,"onchange":onPane,}});}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('selectedId'), slots: {}, events: {} }}
const ArtifactStage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ArtifactStage__SvelteComponent_ = ReturnType<typeof ArtifactStage__SvelteComponent_>;
/*Ωignore_endΩ*/export default ArtifactStage__SvelteComponent_;