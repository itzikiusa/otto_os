///<reference types="svelte" />
;// Tags panel (left sidebar mode): every tag with its count; click → search.

import { vault } from './vault.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
function $$render() {

  
  
  

  let filter = $state('');
  const shown = $derived(
    vault.tags.filter((t) => !filter || t.tag.toLowerCase().includes(filter.toLowerCase())),
  );
;
async () => {

 { svelteHTML.createElement("div", { "class":`tags`,});
   { svelteHTML.createElement("input", {          "dir":`ltr`,"type":`search`,"bind:value":filter,"placeholder":`Filter tags…`,"aria-label":`Filter tags`,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`tags`,"loading":vault.tagsLoading,"error":vault.tagsError,"empty":vault.tags.length === 0,"onretry":() => void vault.loadTags(),"variant":`compact`,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`dim`,});             }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
    if(shown.length === 0){
       { svelteHTML.createElement("div", { "class":`dim`,});   filter;  }
    }
     { svelteHTML.createElement("div", { "class":`list`,});
         for(let t of __sveltets_2_ensureArray(shown)){t.tag;
         { svelteHTML.createElement("button", {   "class":`tag-row`,"onclick":() => vault.searchTag(t.tag),});
           { svelteHTML.createElement("span", { "class":`tag-name`,}); t.tag; }
           { svelteHTML.createElement("span", { "class":`count`,});t.count; }
         }
      }
     }
   LoadState}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TagsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TagsPanel__SvelteComponent_ = ReturnType<typeof TagsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default TagsPanel__SvelteComponent_;