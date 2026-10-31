///<reference types="svelte" />
;// The thread list: the four pinned Spaces (01–04) and Recent threads, each
// with its needs-you count. ↑/↓ move between rows, Enter opens.

import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { assistant } from '../../lib/stores/assistant.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { groupThreads, spaceLabel, SPACE_SLOTS } from '../../lib/assistant/model';
import type { AssistantThread } from '../../lib/api/types';
;

  interface Props {
    threads: AssistantThread[];
    selected: string | null;
    onopen: (id: string) => void;
    onnew: () => void;
    onnewspace: (slot: 1 | 2 | 3 | 4) => void;
    creating?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { threads, selected, onopen, onnew, onnewspace, creating = false }: Props = $props();

  const groups = $derived(groupThreads(threads));
  const needs = $derived(assistant.needsByThread);

  let listEl = $state<HTMLElement | null>(null);
  function onKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const rows = Array.from(listEl?.querySelectorAll<HTMLButtonElement>('button.th-row') ?? []);
    const at = rows.indexOf(document.activeElement as HTMLButtonElement);
    if (at < 0) return;
    e.preventDefault();
    rows[Math.max(0, Math.min(rows.length - 1, at + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus();
  }
;
async () => {

 { const $$_nav0 = svelteHTML.createElement("nav", {    "class":`list`,"aria-label":`Assistant threads`,});listEl = $$_nav0;
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("span", { "class":`section-title`,});  }
   }
   { svelteHTML.createElement("ul", { "class":`rows`,});
       for(let slot of __sveltets_2_ensureArray(SPACE_SLOTS)){slot;
      const t = groups.spaces[slot - 1];
       { svelteHTML.createElement("li", {});
        if(t){
           { svelteHTML.createElement("button", {          "class":`th-row`,"onkeydown":onKey,"aria-current":selected === t.id ? 'page' : undefined,"onclick":() => onopen(t.id),"title":t.title,});selected === t.id;
             { svelteHTML.createElement("span", { "class":`num mono`,});spaceLabel(slot); }
             { svelteHTML.createElement("span", { "class":`title`,});t.title; }
            if(t.incognito){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eyeOff`,"size":12,}});}}
            if(needs[t.id]){ { svelteHTML.createElement("span", {   "class":`needs`,"title":`${needs[t.id]} waiting on you`,});needs[t.id]; }}
           }
        }else{
           { svelteHTML.createElement("button", {         "class":`th-row empty-slot`,"onkeydown":onKey,"onclick":() => onnewspace(slot),"disabled":creating,"title":`Start space ${spaceLabel(slot)}`,});
             { svelteHTML.createElement("span", { "class":`num mono`,});spaceLabel(slot); }
             { svelteHTML.createElement("span", { "class":`title`,});  }
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
           }
        }
       }
    }
   }
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("span", { "class":`section-title`,});  }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":onnew,"disabled":creating,"aria-label":`New thread`,"title":`New thread`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}
     }
   }
  if(groups.recent.length){
     { svelteHTML.createElement("ul", { "class":`rows`,});
         for(let t of __sveltets_2_ensureArray(groups.recent)){t.id;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {          "class":`th-row`,"onkeydown":onKey,"aria-current":selected === t.id ? 'page' : undefined,"onclick":() => onopen(t.id),"title":t.title,});selected === t.id;
             { svelteHTML.createElement("span", { "class":`title`,});t.title; }
            if(t.incognito){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eyeOff`,"size":12,}});}}
            if(needs[t.id]){
               { svelteHTML.createElement("span", {   "class":`needs`,"title":`${needs[t.id]} waiting on you`,});needs[t.id]; }
            }else{
               { svelteHTML.createElement("time", {     "class":`when`,"datetime":t.updated_at,"title":new Date(t.updated_at).toLocaleString(),});rel(t.updated_at); }
            }
           }
         }
      }
     }
  }else{
     { svelteHTML.createElement("p", { "class":`none`,});    }
  }
  if(assistant.threadsMoreError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`older threads`,"error":assistant.threadsMoreError,"empty":true,"onretry":() => void assistant.loadMoreThreads(),}});}
  } else if (assistant.threadsHasMore){
     { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":assistant.threadsMoreLoading,"onclick":() => void assistant.loadMoreThreads(),});assistant.threadsMoreLoading ? 'Loading threads…' : 'Load more threads'; }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ThreadList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ThreadList__SvelteComponent_ = ReturnType<typeof ThreadList__SvelteComponent_>;
/*Ωignore_endΩ*/export default ThreadList__SvelteComponent_;