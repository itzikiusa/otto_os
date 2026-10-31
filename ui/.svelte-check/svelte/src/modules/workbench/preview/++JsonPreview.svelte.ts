///<reference types="svelte" />
;// JSON tree view: the DB Explorer's lazily-rendered collapsible tree
// (lib/components/JsonTree.svelte, standalone mode — a collapsed container renders
// one summary line, long arrays chunk) so a multi-MB document stays cheap.
// Parse errors show inline with line/column.

import JsonTree from '../../../lib/components/JsonTree.svelte';
import { parseJsonWithPos } from './kinds';
import PreviewError from './PreviewError.svelte';
;

  interface Props {
    content: string;
  };function $$render() {

  
  
  
  
  
  
  

  let { content }: Props = $props();

  const parsed = $derived(parseJsonWithPos(content));
;
async () => {

 { svelteHTML.createElement("div", { "class":`json`,});
  if(parsed.ok){
     { svelteHTML.createElement("div", { "class":`tree`,}); { const $$_eerTnosJ2C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ2C({ target: __sveltets_2_any(), props: {  "value":parsed.value,}});} }
  } else if (!content.trim()){
     { svelteHTML.createElement("div", { "class":`state`,});       }
  }else{
     { const $$_rorrEweiverP1C = __sveltets_2_ensureComponent(PreviewError); new $$_rorrEweiverP1C({ target: __sveltets_2_any(), props: {        "title":`Invalid JSON`,"message":parsed.error ?? 'Parse error',"line":parsed.line,"col":parsed.col,}});}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const JsonPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type JsonPreview__SvelteComponent_ = ReturnType<typeof JsonPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default JsonPreview__SvelteComponent_;