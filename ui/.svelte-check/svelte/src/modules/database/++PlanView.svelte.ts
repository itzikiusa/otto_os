///<reference types="svelte" />
;// Normalized query-plan tree (from POST …/db/query-plan). Renders the engine's
// EXPLAIN as a collapsible op·object·est-rows tree with red badges on costly
// patterns (full scans, filesort, temp tables), plus a raw-JSON toggle.

import Icon from '../../lib/components/Icon.svelte';
import type { DbQueryPlan, DbPlanNode } from '../../lib/api/types';
;

  interface Props {
    plan: DbQueryPlan;
    onclose: () => void;
  };function $$render() {
 const node/*Ωignore_positionΩ*/ = (n: DbPlanNode, id: string, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const kids = n.children ?? [];
  const open = !collapsed.has(id);
   { svelteHTML.createElement("div", {   "class":`plan-row`,"style":`padding-inline-start: ${depth * 16 + 6}px`,});
    if(kids.length > 0){
       { svelteHTML.createElement("button", {       "class":`plan-caret`,"onclick":() => toggle(id),"aria-label":open ? 'Collapse node' : 'Expand node',"title":open ? 'Collapse node' : 'Expand node',});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
       }
    }else{
       { svelteHTML.createElement("span", { "class":`plan-caret-spacer`,}); }
    }
     { svelteHTML.createElement("span", { "class":`plan-op`,});n.op; }
    if(n.object){ { svelteHTML.createElement("span", { "class":`plan-object mono`,});n.object; }}
    if(n.est_rows != null){ { svelteHTML.createElement("span", { "class":`plan-rows`,}); Math.round(n.est_rows).toLocaleString();  }}
    if(n.detail){ { svelteHTML.createElement("span", {   "class":`plan-detail`,"title":n.detail,});n.detail; }}
        for(let w of __sveltets_2_ensureArray(n.warnings ?? [])){let wi = 1;wi;
       { svelteHTML.createElement("span", {   "class":`plan-warn`,"title":w,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}w; }
    }
   }
  if(open){
        for(let child of __sveltets_2_ensureArray(kids)){let i = 1;i;
      ;__sveltets_2_ensureSnippet(node(child, `${id}.${i}`, depth + 1));
    }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  

  let { plan, onclose }: Props = $props();

  let rawMode = $state(false);
  // Node paths the user has collapsed (default: all expanded).
  let collapsed = $state<Set<string>>(new Set());
  function toggle(id: string): void {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`plan-panel`,});
   { svelteHTML.createElement("div", { "class":`plan-head`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});}
     { svelteHTML.createElement("span", { "class":`plan-title`,});  }
     { svelteHTML.createElement("span", { "class":`plan-engine mono`,});plan.engine; }
     { svelteHTML.createElement("span", { "class":`plan-grow`,}); }
     { svelteHTML.createElement("button", {        "class":`plan-btn`,"onclick":() => (rawMode = !rawMode),"title":`Toggle the engine’s raw EXPLAIN JSON`,});rawMode;
      
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});}rawMode ? 'Tree' : 'Raw JSON';
     }
     { svelteHTML.createElement("button", {       "class":`plan-close`,"onclick":onclose,"aria-label":`Close plan`,"title":`Close plan`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});} }
   }
  if(rawMode){
     { svelteHTML.createElement("pre", { "class":`plan-raw mono`,});JSON.stringify(plan.raw, null, 2); }
  }else{
     { svelteHTML.createElement("div", { "class":`plan-tree`,});
      ;__sveltets_2_ensureSnippet(node(plan.root, '0', 0));
     }
  }
 }




};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PlanView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PlanView__SvelteComponent_ = ReturnType<typeof PlanView__SvelteComponent_>;
/*Ωignore_endΩ*/export default PlanView__SvelteComponent_;