///<reference types="svelte" />
;// Cross-link card: shows the Product story that originated a swarm project
// (via Plan → Swarm). Rendered in the Kanban toolbar when the selected
// project carries a story_id. Calls GET /swarm/tasks/{tid}/story using the
// first task of the project to resolve the back-link, or falls back to
// GET /product/stories/{sid} when the project.story_id is already known
// directly (avoids needing a task).
//
// The card is intentionally minimal — title + stage badge + a "View story"
// link that navigates to the Product section. When no link exists (project
// was not seeded from a story) the card renders nothing.

import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { badgeTone, storyStage } from '../../lib/status';
import { api } from '../../lib/api/client';
import { product } from '../../lib/stores/product.svelte';
import { router } from '../../lib/router.svelte';
import type { ProductStory } from '../product/types';
import type { SwarmProject } from './types';
;

  interface Props {
    project: SwarmProject;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { project }: Props = $props();

  // ── Fetch ─────────────────────────────────────────────────────────────────
  let story = $state<ProductStory | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    const sid = project.story_id ?? null;
    if (!sid) {
      story = null;
      return;
    }
    load(sid);
  });

  async function load(sid: string): Promise<void> {
    loading = true;
    error = null;
    try {
      // GET /product/stories/{sid} returns ProductStoryDetail; we only need
      // the `story` field from it.
      const detail = await api.get<{ story: ProductStory }>(`/product/stories/${sid}`);
      story = detail.story ?? null;
    } catch (e) {
      // Supplementary: one compact line + Retry, never a page-level error.
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ── Navigation ─────────────────────────────────────────────────────────────
  async function viewStory(): Promise<void> {
    if (!story) return;
    product.selectedId = story.id;
    router.go('product');
  }

  const stage = $derived(story?.stage ? storyStage(story.stage) : null);
;
async () => {

if(!story && (loading || error) && project.story_id){
   { svelteHTML.createElement("div", { "class":`slc-state`,});
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`the linked story`,"variant":`compact`,loading,error,"empty":true,"rows":1,"onretry":() => void load(project.story_id!),}});}
   }
} else if (story){
   { svelteHTML.createElement("div", { "class":`story-link-card`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":12,}});}
     { svelteHTML.createElement("span", { "class":`slc-label dim`,});  }
     { svelteHTML.createElement("span", {   "class":`slc-title`,"title":story.title,});story.title; }
    if(stage){
       { const $$_egdaB1C = __sveltets_2_ensureComponent(Badge); new $$_egdaB1C({ target: __sveltets_2_any(), props: {    "tone":badgeTone(stage.tone),"label":stage.label,}});}
    }
     { svelteHTML.createElement("button", {     "class":`btn small ghost slc-view`,"onclick":viewStory,"title":`Open this story in Product`,});
       
     }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StoryLinkCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StoryLinkCard__SvelteComponent_ = ReturnType<typeof StoryLinkCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default StoryLinkCard__SvelteComponent_;