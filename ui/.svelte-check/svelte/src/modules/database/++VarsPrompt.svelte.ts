///<reference types="svelte" />
;// Run-time prompt for query variables that have no value yet (`:name` /
// `{name}` / `{{name}}`): one row per missing name with the same value +
// type + escape controls as the editor's variables bar. Enter (or Run)
// hands every row back through `onsubmit`; the caller persists them with
// `database.setVar` and re-runs. Opened by Run when a value is missing and
// when a saved query containing placeholders is opened.

import { untrack } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import { defaultVarSpec, type VarSpec } from '../../lib/database/sql-util';
;

  interface Props {
    /** The names still lacking a value, in statement order. */
    names: string[];
    /** The tab's current specs (type / escape are kept even when the value is empty). */
    vars: Record<string, VarSpec>;
    onsubmit: (vals: Record<string, VarSpec>) => void;
    oncancel: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { names, vars, onsubmit, oncancel }: Props = $props();

  // Local drafts seeded ONCE from the tab's specs (the owner mounts a fresh
  // prompt per run, so the initial props are the whole story) — nothing is
  // written back until Run.
  let drafts = $state<Record<string, VarSpec>>(
    untrack(() => Object.fromEntries(names.map((n) => [n, { ...(vars[n] ?? defaultVarSpec()) }]))),
  );
  const missing = $derived(names.filter((n) => !(drafts[n]?.value ?? '').trim()));

  function submit(): void {
    if (missing.length > 0) return;
    onsubmit(drafts);
  }
  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Query variables`,"width":480,"onclose":oncancel,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":oncancel,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":missing.length > 0,"onclick":submit,"title":missing.length > 0 ? `Still missing: ${missing.join(', ')}` : 'Run with these values (Enter)',});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`vp`,...__sveltets_2_empty({"data-testid":`vars-prompt`}),});
     { svelteHTML.createElement("p", { "class":`vp-hint`,});
         names.length === 1 ? 'a variable' : `${names.length} variables`;  
                     
     }
       for(let name of __sveltets_2_ensureArray(names)){name;
       { svelteHTML.createElement("div", { "class":`vp-row`,});
         { svelteHTML.createElement("span", {   "class":`vp-name mono`,"title":name,});name; }
         { svelteHTML.createElement("input", {              "dir":`auto`,"class":`input vp-input`,"bind:value":drafts[name].value,"placeholder":drafts[name].type === 'number' ? '123' : 'value',"spellcheck":`false`,"onkeydown":onKeydown,"aria-label":`Value for ${name}`,});/*Ωignore_startΩ*/() => drafts[name].value = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("select", {       "class":`input vp-type`,"bind:value":drafts[name].type,"title":`How to substitute: string (quoted), number (raw), or raw (verbatim)`,"aria-label":`Type for ${name}`,});/*Ωignore_startΩ*/() => drafts[name].type = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`string`,});  }
           { svelteHTML.createElement("option", { "value":`number`,});  }
           { svelteHTML.createElement("option", { "value":`raw`,});  }
         }
        if(drafts[name].type === 'string'){
           { svelteHTML.createElement("label", {   "class":`vp-esc`,"title":`Escape quotes inside the value`,});
             { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":drafts[name].escape,});/*Ωignore_startΩ*/() => drafts[name].escape = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            
           }
        }
       }
    }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const VarsPrompt__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type VarsPrompt__SvelteComponent_ = ReturnType<typeof VarsPrompt__SvelteComponent_>;
/*Ωignore_endΩ*/export default VarsPrompt__SvelteComponent_;