///<reference types="svelte" />
;// ArgoCD sync sheet: optional revision override + prune. Prune is
// destructive (deletes resources not in git) → typed-name confirm in
// `runAction`.

import Modal from '../../lib/components/Modal.svelte';
import type { K8sRow } from '../../lib/api/types';
;

  interface Props {
    row: K8sRow;
    onsubmit: (params: { revision?: string; prune: boolean }) => void;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  

  let { row, onsubmit, onclose }: Props = $props();

  let revision = $state('');
  let prune = $state(false);

  function submit(): void {
    onsubmit({ revision: revision.trim() || undefined, prune });
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Sync application`,"width":460,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":submit,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`sync`,});
     { svelteHTML.createElement("div", { "class":`target mono`,});row.namespace ? `${row.namespace}/` : '';row.name; }
     { svelteHTML.createElement("dl", { "class":`facts`,});
      if(row.extra?.sync){ { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});row.extra.sync; }}
      if(row.extra?.health){ { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});row.extra.health; }}
      if(row.extra?.revision){ { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});row.extra.revision; }}
      if(row.extra?.repo){ { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`mono`,});row.extra.repo;row.extra.path ? ` · ${row.extra.path}` : ''; }}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`k8s-sync-rev`,});  { svelteHTML.createElement("span", { "class":`dim`,});     } }
       { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`k8s-sync-rev`,"class":`input mono`,"bind:value":revision,"placeholder":`HEAD, a branch, tag or SHA`,"onkeydown":(e) => { if (e.key === 'Enter') submit(); },});/*Ωignore_startΩ*/() => revision = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("label", { "class":`checkbox-row`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":prune,});/*Ωignore_startΩ*/() => prune = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             
     }
    if(prune){ { svelteHTML.createElement("span", { "class":`hint danger`,});              }}
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SyncDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SyncDialog__SvelteComponent_ = ReturnType<typeof SyncDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default SyncDialog__SvelteComponent_;