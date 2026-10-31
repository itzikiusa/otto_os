///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Product → story → Design tab: the Design Hall view of this story. A small
// graph — the story, the designs that implement it (an epic also shows its
// children's), and what each of those designs uses — with "Open in Design
// Hall". It sits above the unchanged Design Arena and stays out of the way:
// collapsible (remembered per device) and silent when Design Hall isn't
// available (no feature grant, or a daemon without the graph routes).
import { untrack } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import { router } from '../../lib/router.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { lsGet, lsSet } from '../../lib/storage';
import { designBus } from '../../lib/events.svelte';
import { getLinks, listArtifacts } from '../../lib/api/design';
import type { DesignArtifact, DesignLinksResp } from '../../lib/api/types';
import StudioBadge from './StudioBadge.svelte';
import StatusPill from './StatusPill.svelte';
import { relLabel } from './model';
;

  interface Props {
    storyId: string;
    storyKey: string;
    storyTitle: string;
    isEpic: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { storyId, storyKey, storyTitle, isEpic }: Props = $props();

  const KEY = 'otto.product.designGraph.open';
  let open = $state(lsGet(KEY) !== '0');
  function toggle(): void {
    open = !open;
    lsSet(KEY, open ? '1' : '0');
  }

  let artifacts = $state<DesignArtifact[]>([]);
  let uses = $state<Record<string, { rel: string; target: DesignArtifact }[]>>({});
  let phase = $state<'loading' | 'ready' | 'unavailable' | 'error'>('loading');
  let seq = 0;

  async function load(): Promise<void> {
    const my = ++seq;
    try {
      const list = await listArtifacts({ story_id: storyId, include_children: isEpic, limit: 60 });
      if (my !== seq) return;
      artifacts = list.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      phase = 'ready';
      // Outgoing artifact links for the first dozen designs (the graph's third column).
      const results = await Promise.all(
        artifacts.slice(0, 12).map((a) =>
          getLinks(a.id, 'out').then(
            (r): [string, DesignLinksResp] => [a.id, r],
            (): [string, DesignLinksResp] => [a.id, { links: [], artifacts: [] }],
          ),
        ),
      );
      if (my !== seq) return;
      const next: typeof uses = {};
      for (const [id, r] of results) {
        const byId = new Map(r.artifacts.map((x) => [x.id, x]));
        next[id] = r.links
          .filter((l) => l.dst_kind === 'artifact' && byId.has(l.dst_id))
          .map((l) => ({ rel: l.rel, target: byId.get(l.dst_id)! }));
      }
      uses = next;
    } catch (e) {
      if (my !== seq) return;
      const status = (e as { status?: number }).status;
      phase = status === 403 || status === 404 || status === 405 ? 'unavailable' : 'error';
    }
  }

  $effect(() => {
    void storyId;
    void isEpic;
    void designBus.resyncTick;
    untrack(() => {
      if (!auth.can('design', 'view')) {
        phase = 'unavailable';
        return;
      }
      phase = 'loading';
      void load();
    });
  });

  let seen = designBus.seq;
  let t: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      if (phase === 'unavailable' || !evs.some((e) => e.type !== 'design_learning_update')) return;
      if (t) clearTimeout(t);
      t = setTimeout(() => void load(), 400);
    });
  });

  const openHall = () => router.go(`design/story/${encodeURIComponent(storyId)}`);
  const openArtifact = (id: string) => router.go(`design/a/${encodeURIComponent(id)}`);
;
async () => {

if(phase !== 'unavailable'){
   { svelteHTML.createElement("section", {     "class":`dgs`,"aria-label":`Design Hall`,...__sveltets_2_empty({"data-testid":`design-graph-strip`}),});
     { svelteHTML.createElement("header", {});
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":toggle,"aria-expanded":open,"aria-label":open ? 'Collapse Design Hall designs' : 'Expand Design Hall designs',"title":open ? 'Collapse' : 'Expand',});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":14,}});}
       }
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`designHall`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`h`,});  }
       { svelteHTML.createElement("span", { "class":`dim`,});
        if(phase === 'loading'){ } else if (phase === 'error'){   }else{plural(artifacts.length, 'design');    isEpic ? 'epic and its stories' : 'story';}
       }
      if(phase === 'error'){ { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void load(),});  }}
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":openHall,...__sveltets_2_empty({"data-testid":`design-open-in-hall`}),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}     }
     }
    if(open && phase === 'ready'){
      if(artifacts.length === 0){
         { svelteHTML.createElement("p", { "class":`empty dim`,});                                }
      }else{
         { svelteHTML.createElement("div", { "class":`graph`,});
           { svelteHTML.createElement("div", {   "class":`story-node`,"title":storyTitle,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}
             { svelteHTML.createElement("span", { "class":`key`,});storyKey; }
             { svelteHTML.createElement("span", { "class":`st`,});storyTitle; }
           }
           { svelteHTML.createElement("ul", { "class":`rows`,});
               for(let a of __sveltets_2_ensureArray((artifacts.slice(0, 12)))){a.id;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("span", {   "class":`edge`,"aria-hidden":`true`,}); }
                 { svelteHTML.createElement("span", { "class":`rel dim`,});  }
                 { svelteHTML.createElement("button", {     "class":`node`,"onclick":() => openArtifact(a.id),"title":`Open ${a.title} in Design Hall`,});
                   { const $$_egdaBoidutS5C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS5C({ target: __sveltets_2_any(), props: {    "studio":a.studio,"size":16,}});}
                   { svelteHTML.createElement("span", { "class":`nt`,});a.title; }
                  if(a.head_seq != null){ { svelteHTML.createElement("span", { "class":`dim`,}); a.head_seq; }}
                   { const $$_lliPsutatS5C = __sveltets_2_ensureComponent(StatusPill); new $$_lliPsutatS5C({ target: __sveltets_2_any(), props: {  "status":a.status,}});}
                 }
                   for(let u of __sveltets_2_ensureArray(uses[a.id] ?? [])){u.target.id + u.rel;
                   { svelteHTML.createElement("span", {   "class":`arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});} }
                   { svelteHTML.createElement("span", { "class":`rel dim`,});relLabel(u.rel); }
                   { svelteHTML.createElement("button", {     "class":`node small`,"onclick":() => openArtifact(u.target.id),"title":`Open ${u.target.title}`,});
                     { const $$_egdaBoidutS5C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS5C({ target: __sveltets_2_any(), props: {  "studio":u.target.studio,}});}
                     { svelteHTML.createElement("span", { "class":`nt`,});u.target.title; }
                   }
                }
               }
            }
           }
          if(artifacts.length > 12){
             { svelteHTML.createElement("button", {   "class":`btn small ghost more`,"onclick":openHall,});artifacts.length - 12;     }
          }
         }
      }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DesignGraphStrip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DesignGraphStrip__SvelteComponent_ = ReturnType<typeof DesignGraphStrip__SvelteComponent_>;
/*Ωignore_endΩ*/export default DesignGraphStrip__SvelteComponent_;