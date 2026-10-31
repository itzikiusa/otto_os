///<reference types="svelte" />
;
import AgentDocuments from '../../src/modules/personal-agents/AgentDocuments.svelte';
import ConfirmDialog from '../../src/lib/components/ConfirmDialog.svelte';
import Announcer from '../../src/lib/components/Announcer.svelte';
import { router } from '../../src/lib/router.svelte';
import PathField from '../../src/lib/components/PathField.svelte';
function $$render() {

  
  
  
  
  
  let path = $state('~/original');
  let submitted = $state(false);
  const params = new URLSearchParams(location.search);
;
async () => {
  { const $$_stnemucoDtnegA0C = __sveltets_2_ensureComponent(AgentDocuments); new $$_stnemucoDtnegA0C({ target: __sveltets_2_any(), props: {        "agentId":`a`,"workspaceId":`w`,"kind":params.get('kind') === 'context' ? 'context' : 'memory',"editable":!params.has('viewer'),"sharedWorkspace":true,}});}
 { svelteHTML.createElement("form", { "onsubmit":(e) => { e.preventDefault(); submitted = true; },});
   { const $$_dleiFhtaP1C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP1 = new $$_dleiFhtaP1C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:path,}});/*Ωignore_startΩ*/() => path = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP1.$$bindings = 'value'; { svelteHTML.createElement("input", {    "aria-label":`Working folder`,"bind:value":path,});/*Ωignore_startΩ*/() => path = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
   { svelteHTML.createElement("output", {});submitted ? 'submitted' : 'not submitted'; }
 }

 { svelteHTML.createElement("button", { "onclick":() => router.go("fixture-away"),});  }
 { const $$_golaiDmrifnoC0C = __sveltets_2_ensureComponent(ConfirmDialog); new $$_golaiDmrifnoC0C({ target: __sveltets_2_any(), props: {}});}

 { const $$_recnuonnA0C = __sveltets_2_ensureComponent(Announcer); new $$_recnuonnA0C({ target: __sveltets_2_any(), props: {}});}
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PersonalDocumentsFixture__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PersonalDocumentsFixture__SvelteComponent_ = ReturnType<typeof PersonalDocumentsFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default PersonalDocumentsFixture__SvelteComponent_;