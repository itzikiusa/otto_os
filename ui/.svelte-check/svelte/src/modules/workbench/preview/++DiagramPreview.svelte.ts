///<reference types="svelte" />
;// Mermaid / D2 source → SVG through the app's lazy bridges (the same ones
// the vault and Canvas use: lib/render/mermaid.ts — securityLevel 'strict' — and
// canvas/d2.ts — the WASM worker). Latest-wins: a render superseded by a
// newer edit is skipped/discarded by sequence token.

import { untrack } from 'svelte';
import { renderMermaid } from '../../../lib/render/mermaid';
import { renderD2 } from '../../canvas/d2';
import { ui } from '../../../lib/stores/ui.svelte';
import PreviewError from './PreviewError.svelte';
;

  interface Props {
    kind: 'mermaid' | 'd2';
    content: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { kind, content }: Props = $props();

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let attempt = $state(0);

  let seq = 0;
  let idSeq = 0;
  $effect(() => {
    void attempt;
    const src = content;
    const k = kind;
    const dark = ui.resolvedScheme === 'dark';
    const mine = ++seq;
    untrack(() => {
      if (!src.trim()) {
        svg = null;
        error = null;
        loading = false;
        return;
      }
      loading = svg === null;
      const id = `wb-diag-${++idSeq}`;
      const isStale = () => mine !== seq;
      const run = k === 'd2' ? renderD2(id, src, { dark, isStale }) : renderMermaid(id, src, { dark, isStale });
      void run.then((r) => {
        if (mine !== seq || r.stale) return;
        loading = false;
        if (r.svg) {
          svg = r.svg;
          error = null;
        } else {
          error = r.error ?? 'Diagram error';
        }
      });
    });
  });

  /** Mermaid errors often carry "line N" — surface it in the header. */
  const errLine = $derived.by(() => {
    const m = error ? /line (\d+)/i.exec(error) : null;
    return m ? Number(m[1]) : undefined;
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`diag`,});
  if(error){
     { const $$_rorrEweiverP1C = __sveltets_2_ensureComponent(PreviewError); new $$_rorrEweiverP1C({ target: __sveltets_2_any(), props: {         "title":kind === 'd2' ? 'D2 error' : 'Mermaid error',"message":error,"line":errLine,"onretry":() => attempt++,}});}
  }
  if(loading){
     { svelteHTML.createElement("div", {   "class":`state`,"aria-busy":`true`,}); kind === 'd2' ? 'D2' : 'Mermaid';  }
  } else if (!content.trim()){
     { svelteHTML.createElement("div", { "class":`state`,});  kind === 'd2' ? 'D2' : 'Mermaid';     }
  }
  if(svg){
    
     { svelteHTML.createElement("div", {  "class":`figure`,});!!error; svg; }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DiagramPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DiagramPreview__SvelteComponent_ = ReturnType<typeof DiagramPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default DiagramPreview__SvelteComponent_;