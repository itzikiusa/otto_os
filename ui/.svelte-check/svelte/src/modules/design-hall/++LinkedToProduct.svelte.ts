///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// "Linked to Product": the epic tree of stories that designs `implement`,
// with per-studio artifact counts. A story row opens its designs in the Hall.
import Icon from '../../lib/components/Icon.svelte';
import StudioBadge from './StudioBadge.svelte';
import { countTotal, epicTree, studioInfo } from './model';
import { library } from './library.svelte';
import type { DesignStudio } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  

  const rows = $derived(epicTree(library.hits, Object.values(library.stories)));
  let open = $state<Record<string, boolean>>({});
  const isOpen = (id: string, i: number) => open[id] ?? i === 0;

  function toggle(id: string, i: number): void {
    open = { ...open, [id]: !isOpen(id, i) };
  }

  const entries = (c: Partial<Record<DesignStudio, number>>) =>
    Object.entries(c) as [DesignStudio, number][];
;
async () => {

if(rows.length === 0){
   { svelteHTML.createElement("p", { "class":`empty`,});
                     
            
   }
}else{
   { svelteHTML.createElement("div", {   "class":`tree card`,...__sveltets_2_empty({"data-testid":`design-linked-product`}),});
        for(let r of __sveltets_2_ensureArray(rows)){let i = 1;r.story.id;
      const kids = r.children.length;
       { svelteHTML.createElement("div", { "class":`epic`,});
        if(kids){
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => toggle(r.story.id, i),"aria-expanded":isOpen(r.story.id, i),"aria-label":`${isOpen(r.story.id, i) ? 'Collapse' : 'Expand'} ${r.story.source_key}`,"title":`${isOpen(r.story.id, i) ? 'Collapse' : 'Expand'} ${r.story.source_key}`,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":isOpen(r.story.id, i) ? 'chevronDown' : 'chevronRight',"size":14,}});}
           }
        }else{
           { svelteHTML.createElement("span", { "class":`gap`,}); }
        }
         { svelteHTML.createElement("a", {   "class":`row-link`,"href":`#/design/story/${encodeURIComponent(r.story.id)}`,});
           { svelteHTML.createElement("span", { "class":`key`,});r.story.source_key; }
           { svelteHTML.createElement("span", { "class":`title`,});r.story.title; }
         }
         { svelteHTML.createElement("span", { "class":`counts`,});
             for(let [s, n] of __sveltets_2_ensureArray(entries(r.own))){s;
             { svelteHTML.createElement("span", {   "class":`count`,"title":`${n} in ${studioInfo(s).name}`,}); { const $$_egdaBoidutS4C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS4C({ target: __sveltets_2_any(), props: {  "studio":s,}});} n; }
          }
         }
         { svelteHTML.createElement("span", { "class":`meta`,});
          if(r.childCount){plural(r.childCount, 'story', 'stories');  kids;  }else{plural(countTotal(r.own), 'design');}
         }
       }
      if(kids && isOpen(r.story.id, i)){
           for(let c of __sveltets_2_ensureArray(r.children)){c.story.id;
           { svelteHTML.createElement("a", {   "class":`child`,"href":`#/design/story/${encodeURIComponent(c.story.id)}`,});
             { svelteHTML.createElement("span", {   "class":`elbow`,"aria-hidden":`true`,}); }
             { svelteHTML.createElement("span", { "class":`key`,});c.story.source_key; }
             { svelteHTML.createElement("span", { "class":`title`,});c.story.title; }
             { svelteHTML.createElement("span", { "class":`counts`,});
                 for(let [s, n] of __sveltets_2_ensureArray(entries(c.counts))){s;
                 { svelteHTML.createElement("span", {   "class":`count`,"title":`${n} in ${studioInfo(s).name}`,}); { const $$_egdaBoidutS4C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS4C({ target: __sveltets_2_any(), props: {  "studio":s,}});} studioInfo(s).name; n; }
              }
             }
           }
        }
      }
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LinkedToProduct__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LinkedToProduct__SvelteComponent_ = ReturnType<typeof LinkedToProduct__SvelteComponent_>;
/*Ωignore_endΩ*/export default LinkedToProduct__SvelteComponent_;