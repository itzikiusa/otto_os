///<reference types="svelte" />
;// Deny with an optional reason (patterns §5). The reason goes back to the
// agent so it can adjust; leaving it empty is fine. A skipped question uses
// the same sheet with its own title / verb / hint.

import Modal from './Modal.svelte';
;

  interface Props {
    action: string;
    onclose: () => void;
    ondeny: (reason: string | null) => void;
    title?: string;
    /** The confirm button's verb. */
    confirmLabel?: string;
    /** Overrides the "Otto won't {action}" line under the reason. */
    hint?: string;
  };function $$render() {

  
  
  
  

  let { action, onclose, ondeny, title = 'Deny request', confirmLabel = 'Deny', hint }: Props = $props();
  let reason = $state('');

  function submit(): void {
    const r = reason.trim();
    ondeny(r ? r : null);
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },title,"width":440,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":submit,});confirmLabel; }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`deny-reason`,});  }
     { svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`deny-reason`,"class":`input deny-reason`,"rows":3,"bind:value":reason,"placeholder":`Not now — ask me again after lunch`,"onkeydown":(e) => {
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          submit();
        }
      },});/*Ωignore_startΩ*/() => reason = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("span", { "class":`hint`,});hint ?? `Otto won’t ${action.toLowerCase()}. It sees your reason and can suggest something else.`; }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DenySheet__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DenySheet__SvelteComponent_ = ReturnType<typeof DenySheet__SvelteComponent_>;
/*Ωignore_endΩ*/export default DenySheet__SvelteComponent_;