///<reference types="svelte" />
;
import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import type { WorkItem } from '../../lib/api/types';
import {
    KIND_ICON,
    KIND_LABEL,
    RISK_LABEL,
    riskColor,
    workStatus,
    fmtCost,
    relTime,
    ownerLabel,
    riskIsElevated,
  } from './lib';
import { now } from '../../lib/stores/now.svelte';
import { auth } from '../../lib/stores/auth.svelte';
;

  interface Props {
    items: WorkItem[];
    needsApproval: Set<string>;
    selectedId: string | null;
    onOpen: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  

  let { items, needsApproval, selectedId, onOpen }: Props = $props();

  function shortRepo(p: string | null): string {
    if (!p) return '';
    const parts = p.replace(/\/+$/, '').split('/');
    return parts[parts.length - 1] || p;
  }
;
async () => {

 { svelteHTML.createElement("div", {     "class":`wi-list`,"role":`group`,"aria-label":`Work items`,});
     for(let it of __sveltets_2_ensureArray(items)){it.id;
    const owner = ownerLabel(it.owner, auth.me?.id);
     { svelteHTML.createElement("button", {        "class":`wi-row`,"aria-current":it.id === selectedId ? 'true' : undefined,"onclick":() => onOpen(it.id),});it.id === selectedId;
       { svelteHTML.createElement("span", {   "class":`wi-icon`,"title":KIND_LABEL[it.kind],}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[it.kind],"size":14,}});} }
       { svelteHTML.createElement("span", { "class":`wi-main`,});
         { svelteHTML.createElement("span", {   "class":`wi-title`,"title":it.title,});it.title; }
         { svelteHTML.createElement("span", { "class":`wi-sub`,});
           { svelteHTML.createElement("span", { "class":`wi-kindlabel`,});KIND_LABEL[it.kind]; }
          if(it.repo_id){ { svelteHTML.createElement("span", { "class":`wi-dot`,});  } { svelteHTML.createElement("span", {   "class":`mono wi-clip`,"title":it.repo_id,});shortRepo(it.repo_id); }}
          if(it.branch){ { svelteHTML.createElement("span", { "class":`wi-dot`,});  } { svelteHTML.createElement("span", {   "class":`mono wi-clip`,"title":it.branch,});it.branch; }}
          if(owner){ { svelteHTML.createElement("span", { "class":`wi-dot`,});  } { svelteHTML.createElement("span", {   "class":`wi-clip`,"title":owner,});owner; }}
         }
       }
       { svelteHTML.createElement("span", { "class":`wi-meta`,});
        if(needsApproval.has(it.id)){
           { svelteHTML.createElement("span", {   "class":`badge-approve`,"title":`Needs human approval`,});  }
        }
         { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":workStatus(it.status),}});}
        if(riskIsElevated(it.risk_level)){
           { svelteHTML.createElement("span", {     "class":`chip-risk`,"style":`--c:${riskColor(it.risk_level)}`,"title":`Risk (policy): ${RISK_LABEL[it.risk_level]}`,});RISK_LABEL[it.risk_level];  }
        }
         { svelteHTML.createElement("span", {   "class":`wi-cost mono`,"title":`Cost so far`,});fmtCost(it.cost_so_far); }
        
         { svelteHTML.createElement("span", {   "class":`wi-time dim`,"title":new Date(it.last_event_at ?? it.updated_at).toLocaleString(),});now() && relTime(it.last_event_at ?? it.updated_at); }
       }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WorkItemList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkItemList__SvelteComponent_ = ReturnType<typeof WorkItemList__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkItemList__SvelteComponent_;