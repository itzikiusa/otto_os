///<reference types="svelte" />
;// Mounts the personal-agent Autonomy form and Memory inspector against
// routed fixture APIs (see desktop-personal-autonomy.spec.ts).

import AgentAutonomy from '../../src/modules/personal-agents/AgentAutonomy.svelte';
import AgentMemoryInspector from '../../src/modules/personal-agents/AgentMemoryInspector.svelte';
import { router } from '../../src/lib/router.svelte';
import ConfirmDialog from '../../src/lib/components/ConfirmDialog.svelte';
function $$render() {

  
  
  
  
  
  
  const params = new URLSearchParams(location.search);
;
async () => {
if(params.get('view') === 'memory'){
   { const $$_rotcepsnIyromeMtnegA0C = __sveltets_2_ensureComponent(AgentMemoryInspector); new $$_rotcepsnIyromeMtnegA0C({ target: __sveltets_2_any(), props: {    "agentId":`a`,"editable":!params.has('viewer'),}});}
}else{
   { const $$_ymonotuAtnegA0C = __sveltets_2_ensureComponent(AgentAutonomy); new $$_ymonotuAtnegA0C({ target: __sveltets_2_any(), props: {    "agentId":`a`,"editable":!params.has('viewer'),}});}
}
 { const $$_golaiDmrifnoC0C = __sveltets_2_ensureComponent(ConfirmDialog); new $$_golaiDmrifnoC0C({ target: __sveltets_2_any(), props: {}});}

 { svelteHTML.createElement("button", { "onclick":() => router.go("fixture-away"),});  }
};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const PersonalAutonomyFixture__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type PersonalAutonomyFixture__SvelteComponent_ = InstanceType<typeof PersonalAutonomyFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default PersonalAutonomyFixture__SvelteComponent_;