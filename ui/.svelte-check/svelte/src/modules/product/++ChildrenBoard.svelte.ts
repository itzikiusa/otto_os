///<reference types="svelte" />
;// ChildrenBoard — the epic Overview's "Children" board (design §3.2): one column
// per folder (unfiled first), a card per child story / doc, and a roll-up of
// counts across the whole epic. Clicking a card opens the child; the epic's
// Design tab is where all children's artifacts are reviewed together.

import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { storyStage } from '../../lib/status';
import { product } from '../../lib/stores/product.svelte';
import type { ProductStory } from './types';
;

  interface Props {
    epicId: string;
    onaddchild?: (e: MouseEvent) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  const { epicId, onaddchild }: Props = $props();

  const children = $derived<ProductStory[]>(product.childrenOf(epicId));
  /** Folder → children (unfiled `''` first, then A→Z). */
  const folders = $derived.by(() => {
    const map = new Map<string, ProductStory[]>();
    for (const c of children) {
      const list = map.get(c.folder ?? '') ?? [];
      list.push(c);
      map.set(c.folder ?? '', list);
    }
    return [...map.entries()]
      .sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b, undefined, { sensitivity: 'base' })))
      .map(([name, list]) => ({ name, list }));
  });
  const rollup = $derived.by(() => {
    const stages: Record<string, number> = {};
    let docs = 0;
    for (const c of children) {
      if (c.tree_kind === 'doc') docs++;
      else stages[c.stage] = (stages[c.stage] ?? 0) + 1;
    }
    return { total: children.length, docs, stories: children.length - docs, stages, folders: folders.filter((f) => f.name).length };
  });
;
async () => {

 { svelteHTML.createElement("section", { "class":`children-board`,});
   { svelteHTML.createElement("div", { "class":`cb-head`,});
     { svelteHTML.createElement("span", { "class":`cb-title`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}  }
     { svelteHTML.createElement("span", { "class":`cb-rollup`,});
      rollup.total;   rollup.stories; rollup.stories === 1 ? 'story' : 'stories';  rollup.docs; rollup.docs === 1 ? 'doc' : 'docs';
      if(rollup.folders){ rollup.folders; rollup.folders === 1 ? 'folder' : 'folders';}
         for(let [st, n] of __sveltets_2_ensureArray(Object.entries(rollup.stages))){st;
         { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {    "status":storyStage(st),"label":`${n} ${storyStage(st).label.toLowerCase()}`,}});}
      }
     }
    if(onaddchild){
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onaddchild,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
    }
   }
  if(children.length === 0){
     { svelteHTML.createElement("p", { "class":`cb-empty`,});            { svelteHTML.createElement("strong", {});  }  }
  }else{
     { svelteHTML.createElement("div", { "class":`cb-columns`,});
         for(let f of __sveltets_2_ensureArray(folders)){f.name;
         { svelteHTML.createElement("div", { "class":`cb-col`,});
           { svelteHTML.createElement("div", { "class":`cb-col-head`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});} f.name || 'Unfiled';  { svelteHTML.createElement("span", { "class":`cb-count`,});f.list.length; }
           }
             for(let c of __sveltets_2_ensureArray(f.list)){c.id;
             { svelteHTML.createElement("button", {     "class":`cb-card`,"onclick":() => void product.select(c.id),"title":c.source_key,});
               { svelteHTML.createElement("span", { "class":`cb-card-title`,});c.title; }
               { svelteHTML.createElement("span", { "class":`cb-card-meta`,});
                if(c.tree_kind === 'doc'){
                   { svelteHTML.createElement("span", { "class":`chip`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":12,}});}  }
                }else{
                   { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {    "status":storyStage(c.stage),"variant":`text`,}});}
                }
                if(c.source_kind !== 'draft'){ { svelteHTML.createElement("span", { "class":`mono key`,});c.source_key; }}
               }
             }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ChildrenBoard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ChildrenBoard__SvelteComponent_ = ReturnType<typeof ChildrenBoard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ChildrenBoard__SvelteComponent_;