///<reference types="svelte" />
;// Inline prefix-filter for a Redis keyspace node. Typing a prefix + Enter
// re-scans the keyspace with `MATCH <prefix>*` (bounded server-side) so huge
// databases load a narrow, responsive set instead of every key.

import Icon from '../../lib/components/Icon.svelte';
import { database } from '../../lib/stores/database.svelte';
import type { SchemaNode } from '../../lib/api/types';

;type $$ComponentProps =  { node: SchemaNode; depth: number };function $$render() {

  
  
  
  
  
  

  let { node, depth }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Local edit buffer for the filter input. Seeded from the node's current
  // filter and re-synced whenever the node (or its stored filter) changes, so a
  // reused component never shows a stale prefix.
  let draft = $state('');
  const active = $derived(database.nodeFilter(node.id));
  $effect(() => {
    draft = active;
  });

  function apply(): void {
    void database.applyNodeFilter(node, draft);
  }
  function clear(): void {
    draft = '';
    void database.applyNodeFilter(node, '');
  }
;
async () => {


 { svelteHTML.createElement("div", {       "class":`kf`,"role":`group`,"aria-label":`Key filter`,"style":`padding-inline-start: ${(depth + 1) * 13 + 4}px`,});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
   { svelteHTML.createElement("input", {              "dir":`auto`,"aria-label":`Filter keys by prefix`,"class":`kf-input`,"placeholder":`filter by prefix…`,"bind:value":draft,"onkeydown":(e) => {
      if (e.key === 'Enter') apply();
      else if (e.key === 'Escape') clear();
    },"onblur":apply,});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  if(active){
     { svelteHTML.createElement("button", {       "class":`kf-clear`,"title":`Clear filter`,"aria-label":`Clear filter`,"onclick":clear,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RedisKeyFilter__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RedisKeyFilter__SvelteComponent_ = ReturnType<typeof RedisKeyFilter__SvelteComponent_>;
/*Ωignore_endΩ*/export default RedisKeyFilter__SvelteComponent_;