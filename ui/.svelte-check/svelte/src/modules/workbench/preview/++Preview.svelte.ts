///<reference types="svelte" />
;// Workbench preview pane: picks the renderer for the (already effective)
// language and feeds it a DEBOUNCED copy of the editor content (~250 ms), so
// typing never re-renders a diagram or a big table per keystroke. Async
// renderers (Mermaid / D2 / images) carry their own latest-wins token.

import { untrack } from 'svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import { previewKind } from './kinds';
import MarkdownPreview from './MarkdownPreview.svelte';
import HtmlPreview from './HtmlPreview.svelte';
import ImagePreview from './ImagePreview.svelte';
import DiagramPreview from './DiagramPreview.svelte';
import JsonPreview from './JsonPreview.svelte';
import CsvPreview from './CsvPreview.svelte';
;

  interface Props {
    ws: string;
    lang: string;
    /** Editor text; for `image` the asset id. */
    content: string;
    name: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { ws, lang, content, name }: Props = $props();

  const kind = $derived(previewKind(lang));
  const DEBOUNCE_MS = 250;

  // Debounced content. A kind switch (or a new file) lands immediately.
  let shown = $state(untrack(() => content));
  let lastKind = untrack(() => kind);
  let lastName = untrack(() => name);
  $effect(() => {
    const next = content;
    const k = kind;
    const n = name;
    if (k !== lastKind || n !== lastName) {
      lastKind = k;
      lastName = n;
      shown = next;
      return;
    }
    if (next === untrack(() => shown)) return;
    const t = setTimeout(() => (shown = next), DEBOUNCE_MS);
    return () => clearTimeout(t);
  });
;
async () => {

 { svelteHTML.createElement("div", {         "class":`wb-preview`,...__sveltets_2_empty({"data-testid":`wb-preview`}),...__sveltets_2_empty({"data-kind":kind ?? 'none'}),"aria-label":`Preview of ${name}`,"role":`region`,});
  if(kind === 'markdown'){
     { const $$_weiverPnwodkraM1C = __sveltets_2_ensureComponent(MarkdownPreview); new $$_weiverPnwodkraM1C({ target: __sveltets_2_any(), props: {   ws,"content":shown,}});}
  } else if (kind === 'html'){
     { const $$_weiverPlmtH1C = __sveltets_2_ensureComponent(HtmlPreview); new $$_weiverPlmtH1C({ target: __sveltets_2_any(), props: {   "content":shown,name,}});}
  } else if (kind === 'svg'){
      { const $$_weiverPegamI1C = __sveltets_2_ensureComponent(ImagePreview); new $$_weiverPegamI1C({ target: __sveltets_2_any(), props: {    ws,"content":shown,name,"svg":true,}});}
  } else if (kind === 'image'){
     { const $$_weiverPegamI1C = __sveltets_2_ensureComponent(ImagePreview); new $$_weiverPegamI1C({ target: __sveltets_2_any(), props: {   ws,content,name,}});}
  } else if (kind === 'mermaid' || kind === 'd2'){
     { const $$_weiverPmargaiD1C = __sveltets_2_ensureComponent(DiagramPreview); new $$_weiverPmargaiD1C({ target: __sveltets_2_any(), props: {   kind,"content":shown,}});}
  } else if (kind === 'json'){
     { const $$_weiverPnosJ1C = __sveltets_2_ensureComponent(JsonPreview); new $$_weiverPnosJ1C({ target: __sveltets_2_any(), props: {  "content":shown,}});}
  } else if (kind === 'csv' || kind === 'tsv'){
     { const $$_weiverPvsC1C = __sveltets_2_ensureComponent(CsvPreview); new $$_weiverPvsC1C({ target: __sveltets_2_any(), props: {    "content":shown,"delimiter":kind === 'tsv' ? '\t' : ',',}});}
  }else{
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {       "icon":`eye`,"title":`No preview for this language`,"body":`Preview supports Markdown, HTML, SVG, images, Mermaid, D2, JSON and CSV/TSV.`,}});}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Preview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Preview__SvelteComponent_ = ReturnType<typeof Preview__SvelteComponent_>;
/*Ωignore_endΩ*/export default Preview__SvelteComponent_;