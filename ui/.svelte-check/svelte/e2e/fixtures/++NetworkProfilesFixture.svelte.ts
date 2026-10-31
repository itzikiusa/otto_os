///<reference types="svelte" />
;
import NetworkProfilePicker from '../../src/modules/connections/NetworkProfilePicker.svelte';
import SessionNetworkStatus from '../../src/modules/connections/SessionNetworkStatus.svelte';
function $$render() {

  
  
  let selected = $state('');
  let workspaceId = $state('w');
;
async () => {
  { const $$_rekciPeliforPkrowteN0C = __sveltets_2_ensureComponent(NetworkProfilePicker); new $$_rekciPeliforPkrowteN0C({ target: __sveltets_2_any(), props: {     workspaceId,"value":selected,"onchange":(id) => selected = id,"editable":true,}});}
 { svelteHTML.createElement("output", { ...__sveltets_2_empty({"data-testid":`selected`}),});selected; }
 { svelteHTML.createElement("button", { "onclick":() => { workspaceId = 'other'; selected = ''; },});  }
 { const $$_sutatSkrowteNnoisseS0C = __sveltets_2_ensureComponent(SessionNetworkStatus); new $$_sutatSkrowteNnoisseS0C({ target: __sveltets_2_any(), props: {        "sessionId":`s`,"workspaceId":`w`,"selectedProfileId":`p`,"editable":false,}});}
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NetworkProfilesFixture__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NetworkProfilesFixture__SvelteComponent_ = ReturnType<typeof NetworkProfilesFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default NetworkProfilesFixture__SvelteComponent_;