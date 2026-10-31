///<reference types="svelte" />
;// db_query: a statement against one of the workspace's Database Explorer
// connections, picked by name · engine · environment.

import type { WorkflowNode } from '../../../lib/api/types';
import IdPicker from './IdPicker.svelte';
import type { InspectorOptions } from './options.svelte';
import { connectionOptions, paramNum, paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  

  let { node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-conn`,});  }
 { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                    "id":`np-conn`,"value":paramStr(node, 'connection_id'),"cache":pickers.dbConnections,"key":wsId,"toOptions":connectionOptions,"noun":`connection`,"placeholder":`DB-Explorer connection id`,"what":`database connections`,"emptyHint":`No database connections in this workspace yet — add one in Database Explorer.`,"onchange":(v) => set('connection_id', v),}});}
 { svelteHTML.createElement("label", { "for":`np-stmt`,});    }
  { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`np-stmt`,"rows":4,"placeholder":`SELECT * FROM users LIMIT 100`,"value":paramStr(node, 'statement'),"oninput":(e) => set('statement', e.currentTarget.value),}); }
 { svelteHTML.createElement("label", { "for":`np-maxrows`,});    }
 { svelteHTML.createElement("input", {             "id":`np-maxrows`,"type":`number`,"min":`1`,"max":`1000`,"value":paramNum(node, 'max_rows', 100),"oninput":(e) => set('max_rows', Number(e.currentTarget.value)),});}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DbQuery__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DbQuery__SvelteComponent_ = ReturnType<typeof DbQuery__SvelteComponent_>;
/*Ωignore_endΩ*/export default DbQuery__SvelteComponent_;