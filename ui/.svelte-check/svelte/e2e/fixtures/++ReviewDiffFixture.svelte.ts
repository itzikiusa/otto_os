///<reference types="svelte" />
;
import {onMount, tick} from 'svelte';
import DiffView from '../../src/lib/components/DiffView.svelte';
function $$render() {

  
  
  let before = $state(''), after = $state('');
  onMount(() => {
    const host = window as unknown as {reviewDiff?: (a: string, b: string) => Promise<number>};
    host.reviewDiff = async (a, b) => {
      const started = performance.now();
      before = a; after = b;
      await tick();
      await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      return performance.now() - started;
    };
    return () => { delete host.reviewDiff; };
  });
;
async () => {
 { const $$_weiVffiD0C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD0C({ target: __sveltets_2_any(), props: {      before,after,"mode":`split`,"contextLines":4,}});}
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReviewDiffFixture__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReviewDiffFixture__SvelteComponent_ = ReturnType<typeof ReviewDiffFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReviewDiffFixture__SvelteComponent_;