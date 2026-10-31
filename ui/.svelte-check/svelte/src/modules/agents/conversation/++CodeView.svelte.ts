///<reference types="svelte" />
;// Read-only source view for the side panel, editor-style: a line-number
// gutter, whole-file syntax colours (lib/hl `highlightLines`, lazy hljs),
// the target line (`file.rs:88`) tinted and scrolled to, optional wrapping.
// Past WINDOW_AT lines the rows go through the windowed list (uniform rows,
// no wrapping) so a 10k-line file mounts a screenful, not 10k nodes.

import { tick } from 'svelte';
import VirtualList from '../../../lib/components/VirtualList.svelte';
import { ensureHljs, highlightLines } from '../../../lib/hl';
;

  interface Props {
    text: string;
    lang: string | null;
    /** 1-based line to reveal. */
    line?: number | null;
    wrap?: boolean;
    /** Number of the first line (a Read of lines 80–84 starts at 80). */
    start?: number;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { text, lang, line = null, wrap = false, start = 1 }: Props = $props();

  const WINDOW_AT = 3000;
  const ROW_H = 19;
  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
  });
  const lines = $derived(highlightLines(text.replace(/\n$/, ''), hlReady ? lang : null));
  const windowed = $derived(lines.length > WINDOW_AT);
  const plain = $derived(windowed ? text.split('\n') : []);
  const gut = $derived(`${String(lines.length + start - 1).length + 1}ch`);
  const rows = $derived(lines.map((html, i) => ({ html, n: i + start })));

  let rootEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const target = line;
    void lines.length;
    if (!target || windowed) return;
    void tick().then(() => rootEl?.querySelector(`[data-ln="${target}"]`)?.scrollIntoView({ block: 'center' }));
  });
;
async () => {

if(windowed){
   { const $$_tsiLlautriV0C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV0 = new $$_tsiLlautriV0C({ target: __sveltets_2_any(), props: {         "items":rows,"estimateHeight":ROW_H,"class":`cv-vlist`,"scrollIndex":line ? line - start : -1,"findText":(r: { html: string; n: number }) => plain[r.n - start] ?? '',row:(r) => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", {   "class":`cv-row`,});r.n === line;__sveltets_2_ensureType(String, Number, gut);
         { svelteHTML.createElement("span", {   "class":`cv-no mono`,"aria-hidden":`true`,});r.n; } { svelteHTML.createElement("span", { "class":`cv-code mono hljs`,}); r.html || ' '; }
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV0.$$prop_def;/*Ωignore_endΩ*/
    
   VirtualList}
}else{
   { const $$_div0 = svelteHTML.createElement("div", {       "class":`cv`,"dir":`ltr`,...__sveltets_2_empty({"data-lang":lang}),});wrap;rootEl = $$_div0;__sveltets_2_ensureType(String, Number, gut);
     { svelteHTML.createElement("pre", { "class":`cv-pre`,}); { svelteHTML.createElement("code", { "class":`hljs`,});   for(let r of __sveltets_2_ensureArray(rows)){r.n; { svelteHTML.createElement("span", {    "class":`cv-row`,...__sveltets_2_empty({"data-ln":r.n}),});r.n === line; { svelteHTML.createElement("span", {   "class":`cv-no mono`,"aria-hidden":`true`,});r.n; } { svelteHTML.createElement("span", { "class":`cv-code mono`,}); r.html; } }} } }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CodeView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CodeView__SvelteComponent_ = ReturnType<typeof CodeView__SvelteComponent_>;
/*Ωignore_endΩ*/export default CodeView__SvelteComponent_;