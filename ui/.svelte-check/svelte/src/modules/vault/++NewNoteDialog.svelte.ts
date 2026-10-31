///<reference types="svelte" />
;// New-note dialog: name + folder + (OKF vaults) a concept-template picker.

import { vault } from './vault.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { okfConceptTemplate } from './okfTemplate';
import Modal from '../../lib/components/Modal.svelte';

;type $$ComponentProps =  { open?: boolean; dir?: string };function $$render() {

  
  
  
  
  

  let {
    open = $bindable(false),
    dir = $bindable(''),
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props()/*Ωignore_startΩ*/;open;dir;/*Ωignore_endΩ*/;

  let name = $state('');
  let template = $state('blank');
  let input = $state<HTMLInputElement | undefined>();

  $effect(() => {
    if (open) {
      name = '';
      template = vault.current?.okf ? 'concept' : 'blank';
      requestAnimationFrame(() => input?.focus());
    }
  });

  const OKF_TYPES = ['Service', 'Reference', 'Decision', 'Runbook', 'Playbook', 'Metric', 'Dataset', 'Attested Computation'];
  let okfType = $state('Reference');

  function body(title: string): string {
    if (template === 'concept') {
      return okfConceptTemplate(okfType, title, auth.me ? `human:${auth.me.id}` : 'process:otto-note-editor');
    }
    return `# ${title}\n\n`;
  }

  function create(): void {
    const n = name.trim();
    if (!n) return;
    const file = n.endsWith('.md') ? n : `${n}.md`;
    const path = dir ? `${dir}/${file}` : file;
    open = false;
    void vault.createNote(path, body(n.replace(/\.md$/i, '')));
  }
;
async () => {

if(open){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`New note${dir ? ` in ${dir}/` : ''}`,"width":440,"onclose":() => (open = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (open = false),});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!name.trim(),"onclick":create,});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`nn-body`,});
       { const $$_input2 = svelteHTML.createElement("input", {             "dir":`auto`,"bind:value":name,"class":`nn-name`,"placeholder":`Note name`,"aria-label":`Note name`,"onkeydown":(e) => {
          if (e.key === 'Enter') create();
        },});input = $$_input2;/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(vault.current?.okf){
         { svelteHTML.createElement("div", { "class":`nn-template`,});
           { svelteHTML.createElement("label", {});
             { svelteHTML.createElement("input", {     "type":`radio`,"value":`concept`,});template = __sveltets_2_any(null);}
             
           }
           { svelteHTML.createElement("label", {});
             { svelteHTML.createElement("input", {     "type":`radio`,"value":`blank`,});template = __sveltets_2_any(null);}
            
           }
          if(template === 'concept'){
             { svelteHTML.createElement("select", {   "bind:value":okfType,"aria-label":`OKF concept type`,});/*Ωignore_startΩ*/() => okfType = __sveltets_2_any(null);/*Ωignore_endΩ*/
                 for(let t of __sveltets_2_ensureArray(OKF_TYPES)){t; { svelteHTML.createElement("option", { "value":t,});t; }}
             }
          }
         }
      }
     }
    
   Modal}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings('open', 'dir'), slots: {}, events: {} }}
const NewNoteDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NewNoteDialog__SvelteComponent_ = ReturnType<typeof NewNoteDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default NewNoteDialog__SvelteComponent_;