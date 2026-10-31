///<reference types="svelte" />
;
import { onMount } from 'svelte';
import LoopDetail from '../../src/modules/loops/LoopDetail.svelte';
import RunDetail from '../../src/modules/run-with-otto/RunDetail.svelte';
import ConfirmDialog from '../../src/lib/components/ConfirmDialog.svelte';
import ContextMenu from '../../src/lib/components/ContextMenu.svelte';
import type { OttoRun } from '../../src/lib/api/types';
function $$render() {

  
  
  
  
  
  
  let target = $state('A');
  const kind = new URLSearchParams(location.search).get('kind');
  const run = $derived({ id: target, title: `Run ${target}`, status: 'completed', workspace_id: 'ws',
    source_kind: 'channel', source_ref: 'Fixture source', approval_decision: 'approved', proof_status: new URLSearchParams(location.search).get('proof') ?? 'passed',
    pr_draft_json: JSON.stringify({ title: `Run ${target}`, source_branch: 'review', target_branch: 'main' }),
    repo_id: 'repo', repo_path: '/fixture/repo', branch: 'review', base_branch: 'main',
    findings_total: 0, findings_blocking: 0, mode: 'single_agent', provider: 'codex',
  } as OttoRun);
  onMount(() => {
    const change = () => { target = 'B'; };
    window.addEventListener('review-target-change', change);
    return () => window.removeEventListener('review-target-change', change);
  });
;
async () => {
if(kind === 'run'){
   { const $$_liateDnuR0C = __sveltets_2_ensureComponent(RunDetail); new $$_liateDnuR0C({ target: __sveltets_2_any(), props: {   run,"onClose":() => {},}});}
}else{
   { const $$_liateDpooL0C = __sveltets_2_ensureComponent(LoopDetail); new $$_liateDpooL0C({ target: __sveltets_2_any(), props: {    "id":target,"onback":() => {},}});}
}
 { const $$_golaiDmrifnoC0C = __sveltets_2_ensureComponent(ConfirmDialog); new $$_golaiDmrifnoC0C({ target: __sveltets_2_any(), props: {}});} { const $$_uneMtxetnoC0C = __sveltets_2_ensureComponent(ContextMenu); new $$_uneMtxetnoC0C({ target: __sveltets_2_any(), props: {}});}
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AutomationOwnership__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AutomationOwnership__SvelteComponent_ = ReturnType<typeof AutomationOwnership__SvelteComponent_>;
/*Ωignore_endΩ*/export default AutomationOwnership__SvelteComponent_;