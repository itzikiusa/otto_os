///<reference types="svelte" />
;// A reusable multi-select for library skill names — used for team skills
// (swarm config) and project skills. Loads the library on mount; renders the
// current selection as removable chips with an autocompleting add input.

import { swarm } from '../../lib/stores/swarm.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { untrack } from 'svelte';
;

  interface Props {
    /** Currently-selected skill names. */
    selected: string[];
    /** Called with the next selection whenever it changes. */
    onchange: (next: string[]) => void;
    label?: string;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { selected, onchange, label }: Props = $props();

  let draft = $state('');

  $effect(() => {
    untrack(() => void swarm.loadLibrarySkills());
  });

  // Suggestions = library skills not already selected, matching the draft.
  const suggestions = $derived(
    swarm.librarySkills
      .filter((s) => !selected.includes(s.name))
      .filter((s) => !draft.trim() || s.name.toLowerCase().includes(draft.trim().toLowerCase())),
  );

  function add(name: string) {
    const n = name.trim();
    if (!n || selected.includes(n)) return;
    onchange([...selected, n]);
    draft = '';
  }

  function remove(name: string) {
    onchange(selected.filter((s) => s !== name));
  }

  const listId = `skill-list-${Math.random().toString(36).slice(2, 8)}`;
;
async () => {

 { svelteHTML.createElement("div", { "class":`picker`,});
  if(label){ { svelteHTML.createElement("span", { "class":`lbl`,});label; }}
   { svelteHTML.createElement("div", { "class":`add`,});
     { svelteHTML.createElement("input", {              "dir":`auto`,"class":`input grow`,"aria-label":`Add a skill`,"placeholder":`e.g. golang-testing`,"bind:value":draft,"list":listId,"onkeydown":(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          add(draft);
        }
      },});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("datalist", { "id":listId,});
         for(let s of __sveltets_2_ensureArray(suggestions)){s.name;
         { svelteHTML.createElement("option", { "value":s.name,});s.description; }
      }
     }
     { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => add(draft),"disabled":!draft.trim(),});  }
   }

  if(swarm.librarySkillsLoading || swarm.librarySkillsError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "what":`skill suggestions`,"variant":`compact`,"loading":swarm.librarySkillsLoading,"error":swarm.librarySkillsError,"empty":true,"onretry":() => void swarm.loadLibrarySkills(true),}});}
  } else if (swarm.librarySkillsLoaded && swarm.librarySkills.length === 0){
     { svelteHTML.createElement("p", { "class":`dim none`,});             }
  }

  if(selected.length){
     { svelteHTML.createElement("div", { "class":`chips`,});
         for(let s of __sveltets_2_ensureArray(selected)){s;
         { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"variant":`outline`,}});
          s;
           { svelteHTML.createElement("button", {       "class":`x`,"onclick":() => remove(s),"aria-label":`Remove ${s}`,"title":`Remove ${s}`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
         Badge}
      }
     }
  }else{
     { svelteHTML.createElement("p", { "class":`dim none`,});   }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillPicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillPicker__SvelteComponent_ = ReturnType<typeof SkillPicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillPicker__SvelteComponent_;