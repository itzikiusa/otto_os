///<reference types="svelte" />
;// http_request and api_run: method, URL and an optional JSON body.

import type { WorkflowNode } from '../../../lib/api/types';
import JsonLabel from './JsonLabel.svelte';
import { jsonValue, paramJson, paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; onzoom: (field: string, label: string) => void };function $$render() {

  
  
  
  

  let { node, set, onzoom }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

if(node.kind === 'api_run'){
   { svelteHTML.createElement("label", { "for":`np-method`,});  }
   { svelteHTML.createElement("select", {       "id":`np-method`,"value":paramStr(node, 'method') || 'GET',"onchange":(e) => set('method', e.currentTarget.value),});
       for(let m of __sveltets_2_ensureArray((['GET','POST','PUT','PATCH','DELETE']))){m;
       { svelteHTML.createElement("option", { "value":m,});m; }
    }
   }
   { svelteHTML.createElement("label", { "for":`np-url`,});  }
   { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`np-url`,"type":`url`,"placeholder":`https://api.example.com/endpoint`,"value":paramStr(node, 'url'),"oninput":(e) => set('url', e.currentTarget.value),});}
   { const $$_lebaLnosJ0C = __sveltets_2_ensureComponent(JsonLabel); new $$_lebaLnosJ0C({ target: __sveltets_2_any(), props: {      "forId":`np-body`,"label":`Body (JSON, optional)`,"onzoom":() => onzoom('body', 'Body (JSON, optional)'),}});}
   { svelteHTML.createElement("textarea", {            "dir":`ltr`,"id":`np-body`,"rows":3,"placeholder":`&#123;&#125;`,"value":paramJson(node, 'body'),"oninput":(e) => set('body', jsonValue(e.currentTarget.value)),}); }
}else{
   { svelteHTML.createElement("label", { "for":`np-method`,});  }
   { svelteHTML.createElement("select", {       "id":`np-method`,"value":paramStr(node, 'method') || 'GET',"onchange":(e) => set('method', e.currentTarget.value),});
       for(let m of __sveltets_2_ensureArray((['GET','POST','PUT','PATCH','DELETE','HEAD']))){m;
       { svelteHTML.createElement("option", { "value":m,});m; }
    }
   }
   { svelteHTML.createElement("label", { "for":`np-url`,});  }
   { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`np-url`,"type":`url`,"placeholder":`https://example.com/api`,"value":paramStr(node, 'url'),"oninput":(e) => set('url', e.currentTarget.value),});}
   { const $$_lebaLnosJ0C = __sveltets_2_ensureComponent(JsonLabel); new $$_lebaLnosJ0C({ target: __sveltets_2_any(), props: {      "forId":`np-body`,"label":`Body (JSON, optional)`,"onzoom":() => onzoom('body', 'Body (JSON, optional)'),}});}
   { svelteHTML.createElement("textarea", {            "dir":`ltr`,"id":`np-body`,"rows":3,"placeholder":`&#123;&#125;`,"value":paramJson(node, 'body'),"oninput":(e) => set('body', jsonValue(e.currentTarget.value)),}); }
}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HttpRequest__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HttpRequest__SvelteComponent_ = ReturnType<typeof HttpRequest__SvelteComponent_>;
/*Ωignore_endΩ*/export default HttpRequest__SvelteComponent_;