///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// The automations list (left pane). Selecting one opens its editor in the
// main area (AutomationEditor) — the sidebar only lists and creates.
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { Id } from '../../lib/api/types';
;

  interface Props {
    selectedId: Id | null;
    onselect: (id: Id) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { selectedId, onselect }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');

  async function create(): Promise<void> {
    const n = await confirmer.promptText('Name', { title: 'New automation', confirmLabel: 'Create', initial: '' });
    if (!n) return;
    const saved = await apiClient.saveAutomation({ name: n, steps: [] });
    if (saved) onselect(saved.id);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`auto-list-wrap`,});
  
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`automations`,"loading":apiClient.automationsLoading,"error":apiClient.automationsError,"empty":apiClient.automations.length === 0,"onretry":() => void apiClient.loadAutomations({ force: true }),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"icon":`zap`,"title":`No automations yet`,"body":`Run saved requests in order, check each response and pass values (like a login token) to the next request. Useful for smoke tests.`,"actionLabel":canEdit && apiClient.requests.length > 0 ? 'New automation' : undefined,"actionIcon":`plus`,"onaction":canEdit ? create : undefined,}});
      if(apiClient.requests.length === 0){ { svelteHTML.createElement("p", { "class":`hint`,});        }}
     EmptyState}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`head`,});
       { svelteHTML.createElement("span", { "class":`lead`,});     }
      if(canEdit){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`New automation`,"aria-label":`New automation`,"onclick":create,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
      }
     }
     { svelteHTML.createElement("ul", { "class":`auto-list`,});
         for(let a of __sveltets_2_ensureArray(apiClient.automations)){a.id;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {      "class":`auto-pick`,"onclick":() => onselect(a.id),"aria-current":a.id === selectedId ? 'true' : undefined,});a.id === selectedId;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":14,}});}
             { svelteHTML.createElement("span", { "class":`aname`,});a.name; }
             { svelteHTML.createElement("span", { "class":`acount`,});plural(a.steps.length, 'step'); }
           }
         }
      }
     }
   LoadState}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AutomationsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AutomationsView__SvelteComponent_ = ReturnType<typeof AutomationsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default AutomationsView__SvelteComponent_;