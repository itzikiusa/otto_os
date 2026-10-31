///<reference types="svelte" />
;// Pick the live session a workbench file is PASTED into (Send to → Terminal /
// agent session…). Picking still asks a confirm naming the session; the text
// goes in as a bracketed paste with no Enter, so nothing executes.

import Modal from '../../../lib/components/Modal.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { ws as wsStore } from '../../../lib/stores/workspace.svelte';
import type { Session } from '../../../lib/api/types';
import { sessionState } from '../../../lib/status';
import { pasteTargets } from './sendTo';
;

  interface Props {
    ws: string;
    onpick: (session: Session) => void;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { ws, onpick, onclose }: Props = $props();

  let query = $state('');
  const all = $derived(pasteTargets(wsStore.sessions, ws));
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return all;
    return all.filter((s) => `${s.title} ${s.provider} ${s.kind} ${s.cwd}`.toLowerCase().includes(q));
  });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Paste into which session?`,"width":520,onclose,}});
   { svelteHTML.createElement("div", {   "class":`sp`,...__sveltets_2_empty({"data-testid":`wb-session-picker`}),});
     { svelteHTML.createElement("p", { "class":`sp-hint`,});              }
    if(all.length > 0){
       { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input sp-filter`,"type":`search`,"placeholder":`Filter sessions…`,"aria-label":`Filter sessions`,"bind:value":query,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
    }
    if(all.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {         "variant":`panel`,"icon":`terminal`,"title":`No live sessions`,"body":`Start an agent or terminal session in this workspace, then send the file again.`,}});}
    } else if (shown.length === 0){
       { svelteHTML.createElement("p", { "class":`sp-hint`,});   query;  }
    }else{
       { svelteHTML.createElement("ul", { "class":`sp-list`,});
           for(let s of __sveltets_2_ensureArray(shown)){s.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {       "class":`sp-row`,"type":`button`,"onclick":() => onpick(s),"title":`Paste into ${s.title || s.id}`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":s.kind === 'agent' ? 'assistant' : 'terminal',"size":14,}});}
               { svelteHTML.createElement("span", { "class":`sp-title`,});s.title || s.id; }
               { svelteHTML.createElement("span", { "class":`sp-meta`,});s.provider;  sessionState(s, wsStore.statusMap[s.id], wsStore.needsYou[s.id] === true).label; }
             }
           }
        }
       }
    }
   }
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SessionPickerModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SessionPickerModal__SvelteComponent_ = ReturnType<typeof SessionPickerModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default SessionPickerModal__SvelteComponent_;