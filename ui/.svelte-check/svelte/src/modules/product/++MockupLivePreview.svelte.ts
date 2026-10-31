///<reference types="svelte" />
;// MockupLivePreview — renders an in-memory design SOURCE (not a stored
// attachment) for the live "Create with AI" preview, for every design format:
//   • html / mermaid → SANDBOXED iframe with the most-restrictive sandbox=""
//     (scripts OFF, no allow-same-origin) — the source is agent-authored and
//     untrusted. Mermaid is rendered to an SVG string first (never inlined).
//   • excalidraw → the read-only DesignBoard island.
//   • scene3d → the read-only Scene3DViewport (validated JSON only; a doc that
//     doesn't parse shows the raw source so you can see what the agent wrote).

import { renderMermaid } from '../../lib/render/mermaid';
import type { DesignFormat } from './types';
import { svgDoc } from '../../lib/design/format';
import DesignBoard from '../../lib/design/DesignBoard.svelte';
import { Scene3DViewport, parseScene, type Scene3dDoc } from '../../lib/design/scene3d';
import { product } from '../../lib/stores/product.svelte';
;

  interface Props {
    format: DesignFormat;
    content: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const { format, content }: Props = $props();

  let mermaidSvg = $state<string | null>(null);
  let mermaidErr = $state<string | null>(null);
  let seq = 0;

  // Re-render mermaid whenever the source changes. HTML needs no pre-processing —
  // it goes straight into the iframe srcdoc.
  $effect(() => {
    const src = content ?? '';
    if (format !== 'mermaid') {
      mermaidSvg = null;
      mermaidErr = null;
      return;
    }
    const text = src.trim();
    if (!text) {
      mermaidSvg = null;
      mermaidErr = null;
      return;
    }
    const token = ++seq;
    // Lazy + serialized (lib/render/mermaid.ts); a superseded render is skipped.
    void renderMermaid(`otto-mockup-live-${Date.now()}-${token}`, text, { isStale: () => token !== seq }).then(
      ({ svg, error, stale }) => {
        if (stale || token !== seq) return;
        if (svg !== undefined) {
          mermaidSvg = svg;
          mermaidErr = null;
        } else {
          mermaidErr = error ?? 'Diagram error';
        }
      },
    );
  });

  /** The VALIDATED scene3d doc, or null while the agent's JSON is still
   *  incomplete / invalid (the raw source is shown instead). */
  const sceneDoc = $derived.by<Scene3dDoc | null>(() => {
    if (format !== 'scene3d' || !content?.trim()) return null;
    return parseScene(content).doc;
  });
  const resolveAttachment = (aid: string) => product.attachmentBlobUrl(aid);
;
async () => {

 { svelteHTML.createElement("div", { "class":`live-box`,});
  if(!content || !content.trim()){
     { svelteHTML.createElement("div", { "class":`live-msg`,});    }
  } else if (format === 'html'){
     { svelteHTML.createElement("iframe", {     "class":`live-frame`,"title":`Mockup preview`,"sandbox":"","srcdoc":content,}); }
  } else if (format === 'excalidraw'){
      { const $$_draoBngiseD1C = __sveltets_2_ensureComponent(DesignBoard); new $$_draoBngiseD1C({ target: __sveltets_2_any(), props: {  "source":content,"readonly":true,}});}
  } else if (format === 'scene3d'){
    if(sceneDoc){
       { const $$_tropweiVD3enecS1C = __sveltets_2_ensureComponent(Scene3DViewport); new $$_tropweiVD3enecS1C({ target: __sveltets_2_any(), props: {      "doc":sceneDoc,"readonly":true,"resolveAttachment":resolveAttachment,"onchange":() => {},}});}
    }else{
       { svelteHTML.createElement("pre", { "class":`live-src`,});content; }
    }
  } else if (mermaidErr){
     { svelteHTML.createElement("div", { "class":`live-msg err`,});  mermaidErr; }
  } else if (mermaidSvg !== null){
     { svelteHTML.createElement("iframe", {     "class":`live-frame`,"title":`Mockup preview`,"sandbox":"","srcdoc":svgDoc(mermaidSvg),}); }
  }else{
     { svelteHTML.createElement("div", { "class":`live-msg`,});  }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MockupLivePreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MockupLivePreview__SvelteComponent_ = ReturnType<typeof MockupLivePreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default MockupLivePreview__SvelteComponent_;