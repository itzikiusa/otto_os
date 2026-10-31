///<reference types="svelte" />
;// MockupsTab — kept as a thin alias of the Design Arena so the `mockups` tab id
// (deep links, E2E) keeps resolving; the tab's LABEL is "Design" (design §6).
// Above the arena: the Design Hall graph of this story (designs that implement
// it + what they use, "Open in Design Hall"). The arena itself is unchanged.

import DesignArena from './design/DesignArena.svelte';
import DesignGraphStrip from '../design-hall/DesignGraphStrip.svelte';
import { product } from '../../lib/stores/product.svelte';
function $$render() {

  
  
  
  
  
  
  

  const story = $derived(product.detail?.story ?? null);
  const isEpic = $derived(!!story && (story.tree_kind === 'epic' || product.childrenOf(story.id).length > 0));
;
async () => {

if(story){
  story.id; {
     { const $$_pirtShparGngiseD0C = __sveltets_2_ensureComponent(DesignGraphStrip); new $$_pirtShparGngiseD0C({ target: __sveltets_2_any(), props: {       "storyId":story.id,"storyKey":story.source_key,"storyTitle":story.title,isEpic,}});}
  }
}
 { const $$_anerAngiseD0C = __sveltets_2_ensureComponent(DesignArena); new $$_anerAngiseD0C({ target: __sveltets_2_any(), props: {}});}
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MockupsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MockupsTab__SvelteComponent_ = ReturnType<typeof MockupsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default MockupsTab__SvelteComponent_;