///<reference types="svelte" />
;// Global confirmation / prompt dialog — mounted once in App. Driven by
// `confirmer`. In prompt mode it shows a text input (native prompt() is a
// no-op in the Tauri webview).

import Modal from './Modal.svelte';
import PathField from './PathField.svelte';
import { confirmer } from '../confirm.svelte';
function $$render() {

  
  
  
  
  
  

  function onPrimary(): void {
    if (confirmer.isPrompt) confirmer.submit();
    else confirmer.resolve(true);
  }
;
async () => {

if(confirmer.open){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":confirmer.title,"onclose":() => confirmer.dismiss(),"width":400,footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => confirmer.dismiss(),});confirmer.cancelLabel; }
      if(confirmer.choices){
           for(let opt of __sveltets_2_ensureArray(confirmer.choices)){opt.value;
           { svelteHTML.createElement("button", {         "class":`btn`,...__sveltets_2_empty({"data-autofocus":opt.kind === 'primary' ? '' : undefined}),"onclick":() => confirmer.pick(opt.value),});opt.kind === 'primary';opt.kind === 'danger';
            opt.label;
           }
        }
      }else{
        
         { svelteHTML.createElement("button", {         "class":`btn`,...__sveltets_2_empty({"data-autofocus":confirmer.danger ? undefined : ''}),"onclick":onPrimary,});!confirmer.danger;confirmer.danger;
          confirmer.confirmLabel;
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    if(confirmer.message){ { svelteHTML.createElement("p", { "class":`cf-msg`,});confirmer.message; }}
    if(confirmer.isPrompt){
        const promptInput/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("input", {                     "dir":`auto`,"class":`input cf-input`,"aria-label":confirmer.message || confirmer.title,"aria-invalid":confirmer.inputError ? 'true' : undefined,"aria-describedby":confirmer.inputError ? 'cf-input-error' : undefined,"bind:value":confirmer.inputValue,"oninput":() => (confirmer.inputError = ''),"placeholder":confirmer.placeholder,...__sveltets_2_empty({"data-autofocus":true}),"spellcheck":`false`,"onkeydown":(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            confirmer.submit();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            confirmer.dismiss();
          }
        },});/*Ωignore_startΩ*/() => confirmer.inputValue = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      };return __sveltets_2_any(0)};
      if(confirmer.browseFolder){
         { const $$_dleiFhtaP1C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP1 = new $$_dleiFhtaP1C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:confirmer.inputValue,}});/*Ωignore_startΩ*/() => confirmer.inputValue = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP1.$$bindings = 'value';;__sveltets_2_ensureSnippet(promptInput()); PathField}
      }else{;__sveltets_2_ensureSnippet(promptInput());}
      if(confirmer.inputError){
         { svelteHTML.createElement("p", {     "class":`field-error`,"id":`cf-input-error`,"role":`alert`,});confirmer.inputError; }
      }
    }
    if(confirmer.choices && confirmer.checkboxLabel){
       { svelteHTML.createElement("label", { "class":`cf-remember`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":confirmer.checkboxChecked,});/*Ωignore_startΩ*/() => confirmer.checkboxChecked = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        confirmer.checkboxLabel;
       }
    }
    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const ConfirmDialog__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type ConfirmDialog__SvelteComponent_ = InstanceType<typeof ConfirmDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConfirmDialog__SvelteComponent_;