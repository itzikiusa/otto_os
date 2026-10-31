///<reference types="svelte" />
;// Workspace allowlists and policy-as-code stay close to the server registry,
// but behind a side drawer so the common server/tool flow remains compact.
// The slide-over itself (scrim, Esc/outside close, focus trap, modal
// registration) is the shared shell/Drawer.

import Icon from '../../lib/components/Icon.svelte';
import Drawer from '../../shell/Drawer.svelte';
import type { McpServerDetail } from '../../lib/api/types';
import AllowlistsTab from './AllowlistsTab.svelte';
import PoliciesTab from './PoliciesTab.svelte';
;

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
    onClose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { wsId, servers, onClose }: Props = $props();

  let allowlistsOpen = $state(true);
  let policiesOpen = $state(false);

  // The parent mounts this only while it is wanted; the Drawer closes itself
  // (✕, Esc, scrim) by flipping `open`, which unmounts us via onClose.
  let open = $state(true);
  $effect(() => {
    if (!open) onClose();
  });
;
async () => {

 { const $$_rewarD0C = __sveltets_2_ensureComponent(Drawer); const $$_rewarD0 = new $$_rewarD0C({ target: __sveltets_2_any(), props: {      children:() => { return __sveltets_2_any(0); },open,"side":`right`,"title":`Rules`,"width":`min(720px, 100%)`,}});/*Ωignore_startΩ*/() => open = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rewarD0.$$bindings = 'open';
   { svelteHTML.createElement("div", {   "class":`rules`,...__sveltets_2_empty({"data-testid":`mcp-rules-drawer`}),});
     { svelteHTML.createElement("section", { "class":`rule-section`,});
       { svelteHTML.createElement("button", {         "class":`section-head`,"type":`button`,"aria-expanded":allowlistsOpen,"onclick":() => (allowlistsOpen = !allowlistsOpen),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":allowlistsOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}
         { svelteHTML.createElement("span", {});  }
       }
      if(allowlistsOpen){
         { svelteHTML.createElement("div", { "class":`section-body`,}); { const $$_baTstsilwollA4C = __sveltets_2_ensureComponent(AllowlistsTab); new $$_baTstsilwollA4C({ target: __sveltets_2_any(), props: {  wsId,servers,}});} }
      }
     }

     { svelteHTML.createElement("section", { "class":`rule-section`,});
       { svelteHTML.createElement("button", {         "class":`section-head`,"type":`button`,"aria-expanded":policiesOpen,"onclick":() => (policiesOpen = !policiesOpen),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":policiesOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}
         { svelteHTML.createElement("span", {});  }
       }
      if(policiesOpen){
         { svelteHTML.createElement("div", { "class":`section-body`,}); { const $$_baTseiciloP4C = __sveltets_2_ensureComponent(PoliciesTab); new $$_baTseiciloP4C({ target: __sveltets_2_any(), props: {  wsId,servers,}});} }
      }
     }
   }
 Drawer}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RulesDrawer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RulesDrawer__SvelteComponent_ = ReturnType<typeof RulesDrawer__SvelteComponent_>;
/*Ωignore_endΩ*/export default RulesDrawer__SvelteComponent_;