///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
// Cross-link card: shows the swarm project created from this product story
// (via Plan → Swarm), including task counts, run count, and accumulated cost.
// Calls GET /product/stories/{sid}/swarm on mount and whenever the story id
// changes. Always renders 200 (null project = no link yet). Clicking "Open in
// Swarm" navigates to the swarm Kanban for that project.
import Icon from '../../lib/components/Icon.svelte';
import { api } from '../../lib/api/client';
import { swarm } from '../../lib/stores/swarm.svelte';
import { router } from '../../lib/router.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import type { StorySwarmLink } from './types';
;

  interface Props {
    storyId: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { storyId }: Props = $props();

  // ── Fetch ─────────────────────────────────────────────────────────────────
  let link = $state<StorySwarmLink | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    if (!storyId) return;
    load(storyId);
  });

  async function load(sid: string): Promise<void> {
    loading = true;
    error = null;
    try {
      link = await api.get<StorySwarmLink>(`/product/stories/${sid}/swarm`);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // ── Navigation ─────────────────────────────────────────────────────────────
  async function openInSwarm(): Promise<void> {
    if (!link?.project || !ws.currentId) return;
    await swarm.openProject(ws.currentId, link.project.swarm_id, link.project.id);
    router.go('swarm');
  }

  // ── Derived display values ─────────────────────────────────────────────────
  const doneTasks = $derived(
    link?.tasks.filter((t) => t.status === 'done' || t.status === 'cancelled').length ?? 0,
  );
  const totalTasks = $derived(link?.tasks.length ?? 0);
  const runCount = $derived(link?.runs.length ?? 0);
  const costFmt = $derived(
    link && link.cost_usd > 0
      ? `$${link.cost_usd < 0.01 ? link.cost_usd.toFixed(4) : link.cost_usd.toFixed(2)}`
      : null,
  );
  const hasData = $derived(link != null && link.project != null);
;
async () => {

if(loading){
   { svelteHTML.createElement("div", { "class":`swarm-link-card`,}); { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":1,"height":20,"label":`the swarm link`,}});} }
} else if (error){
  
} else if (hasData && link){
   { svelteHTML.createElement("div", { "class":`swarm-link-card`,});
     { svelteHTML.createElement("div", { "class":`slc-head`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":13,}});}
       { svelteHTML.createElement("span", { "class":`slc-title`,});  }
       { svelteHTML.createElement("span", { "class":`slc-name`,});link.project!.name; }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(costFmt){
         { svelteHTML.createElement("span", { "class":`chip dim`,});costFmt; }
      }
       { svelteHTML.createElement("button", {     "class":`btn small ghost slc-open`,"onclick":openInSwarm,"title":`Open this project in the Agent Swarm board`,});
          
       }
     }

     { svelteHTML.createElement("div", { "class":`slc-stats`,});
       { svelteHTML.createElement("span", { "class":`stat`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
        doneTasks; totalTasks; 
       }
      if(runCount > 0){
         { svelteHTML.createElement("span", { "class":`stat`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});}
          plural(runCount, 'run');
         }
      }
      if(link.prs.length > 0){
         { svelteHTML.createElement("span", { "class":`stat`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`pr`,"size":12,}});}
          plural(link.prs.length, 'PR');
         }
      }
      if(link.artifacts.length > 0){
         { svelteHTML.createElement("span", { "class":`stat`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":12,}});}
          plural(link.artifacts.length, 'artifact');
         }
      }
     }

    if(link.tasks.length > 0){
       { svelteHTML.createElement("div", { "class":`task-pills`,});
           for(let t of __sveltets_2_ensureArray((link.tasks.slice(0, 6)))){t.id;
           { svelteHTML.createElement("span", {    "class":`task-pill ${t.status === 'done' || t.status === 'cancelled' ? 'done' : t.status === 'in_progress' ? 'active' : ''}`,"title":t.title,});t.title; }
        }
        if(link.tasks.length > 6){
           { svelteHTML.createElement("span", { "class":`task-pill dim`,}); link.tasks.length - 6;  }
        }
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SwarmLinkCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SwarmLinkCard__SvelteComponent_ = ReturnType<typeof SwarmLinkCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default SwarmLinkCard__SvelteComponent_;