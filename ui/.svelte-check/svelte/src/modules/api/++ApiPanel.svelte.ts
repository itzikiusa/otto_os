///<reference types="svelte" />
;// Compact API client for the right-side panel. Reuses RequestBuilder +
// ResponseViewer; a slim collection/history dropdown replaces the big tree.

import { untrack } from 'svelte';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import RequestBuilder from './RequestBuilder.svelte';
import ResponseViewer from './ResponseViewer.svelte';
import EnvSelector from './EnvSelector.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  // Load on first mount / workspace change — keyed on the workspace only
  // (loadAll's synchronous prologue reads the tabs, which must not re-trigger).
  $effect(() => {
    if (ws.currentId) untrack(() => void apiClient.loadAll());
  });

  // A flat, filterable picker (one shared menu, built on open) — a native
  // <select> mounted one <option> per saved request for as long as the panel
  // was open. Pinned rows (New request + the 15 most recent history entries)
  // always show; saved requests are searched by method/name/url, 50 at a time.
  function openPicker(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'New request', icon: 'plus', pinned: true, action: () => apiClient.newDraft() },
      ...apiClient.history.slice(0, 15).map((h): MenuItem => ({
        label: `${h.method} · ${h.url}`,
        icon: 'clock',
        pinned: true,
        action: () => void apiClient.selectHistory(h.id),
      })),
    ];
    if (apiClient.requests.length > 0) {
      items.push({ separator: true, pinned: true });
      for (const r of apiClient.requests) {
        items.push({ label: `${r.method} · ${r.name} · ${r.url}`, action: () => void apiClient.openRequest(r.id) });
      }
    }
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Search saved requests', maxVisible: 50 });
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`panel`,});
  if(apiClient.historyLoadingId){ { svelteHTML.createElement("span", { "class":`note`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 11px`,"aria-hidden":`true`,}); }    }}
  if(apiClient.requestsLoadError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`saved requests`,"variant":`compact`,"empty":true,"error":apiClient.requestsLoadError,"loading":apiClient.loading,"onretry":() => void apiClient.loadAll(),}});}
  }
   { svelteHTML.createElement("div", { "class":`picker-row`,});
     { svelteHTML.createElement("button", {           "type":`button`,"class":`input picker`,"aria-label":`Load request`,"aria-haspopup":`menu`,"title":`Load a saved or recent request`,"onclick":openPicker,});
       { svelteHTML.createElement("span", { "class":`picker-label`,});  }
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
     }
   }

   { svelteHTML.createElement("div", { "class":`builder-wrap`,});
      { const $$_redliuBtseuqeR2C = __sveltets_2_ensureComponent(RequestBuilder); new $$_redliuBtseuqeR2C({ target: __sveltets_2_any(), props: {"compact":true,}});}
   }

   { svelteHTML.createElement("details", { "class":`env-fold`,});
     { svelteHTML.createElement("summary", {}); if(apiClient.activeEnv){' ';  { svelteHTML.createElement("span", { "class":`env-on`,});apiClient.activeEnv.name; }} }
      { const $$_rotceleSvnE2C = __sveltets_2_ensureComponent(EnvSelector); new $$_rotceleSvnE2C({ target: __sveltets_2_any(), props: {"compact":true,}});}
   }

   { svelteHTML.createElement("div", { "class":`resp-wrap`,});
      { const $$_reweiVesnopseR2C = __sveltets_2_ensureComponent(ResponseViewer); new $$_reweiVesnopseR2C({ target: __sveltets_2_any(), props: {"compact":true,}});}
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ApiPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ApiPanel__SvelteComponent_ = ReturnType<typeof ApiPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ApiPanel__SvelteComponent_;