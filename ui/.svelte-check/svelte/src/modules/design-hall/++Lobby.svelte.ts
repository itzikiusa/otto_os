///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { toastError } from '../../lib/toastError';
import { onTabKey } from './tabKeys';
// Design Hall lobby (Grid view): the prompt hero, all seven studios, the
// Continue strip, projects, the Linked-to-Product epic tree, and a rail with
// what Otto captured for learning + agent activity. Dashboard-style: every
// region owns its loading / empty / error state; the library snapshot is
// shared (library.svelte.ts) and refreshed by live design events.
import { untrack } from 'svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon, { asIcon } from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import StatusDot from '../../lib/components/StatusDot.svelte';
import { router } from '../../lib/router.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { toasts } from '../../lib/toast.svelte';
import { designBus, events } from '../../lib/events.svelte';
import { sessionState } from '../../lib/status';
import { search as searchApi } from '../../lib/api/design';
import { isAbortError } from '../../lib/api/client';
import type { DesignSearchHit, DesignStudio } from '../../lib/api/types';
import ArtifactCard from './ArtifactCard.svelte';
import ProjectCard from './ProjectCard.svelte';
import LinkedToProduct from './LinkedToProduct.svelte';
import LearnedCard from './LearnedCard.svelte';
import StudioBadge from './StudioBadge.svelte';
import SpatialView from './SpatialView.svelte';
import { STUDIOS, filterProjects, studioInfo, titleFromPrompt, type ProjectFilter } from './model';
import { createDesign } from './create';
import { generateFromBrief, referenceOf, suggestReferences } from './assist/handoff';
import { library, LIBRARY_LIMIT } from './library.svelte';
import { loadErrorText } from '../../lib/loadError';
import LoadState from '../../lib/components/LoadState.svelte';
;

  interface Props {
    view: 'grid' | 'spatial';
    onnew: (e: MouseEvent) => void;
    onimport: () => void;
    onnewproject: () => void;
  };function $$render() {
  const searchField/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("label", {  "class":`search`,...__sveltets_2_empty({"data-keep":true}),});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":14,}});}
     { svelteHTML.createElement("input", {              "dir":`auto`,"type":`search`,"placeholder":`Search designs, stories, references…`,"aria-label":`Search all designs`,"bind:value":q,"onkeydown":onSearchKey,...__sveltets_2_empty({"data-testid":`design-search`}),});/*Ωignore_startΩ*/() => q = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { view, onnew, onimport, onnewproject }: Props = $props();

  // ── Library + live refresh ────────────────────────────────────────────────
  $effect(() => {
    void designBus.resyncTick;
    untrack(() => void library.load());
  });
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      library.applyEvents(evs); // patch one card, or a debounced full reload
    });
  });

  const live = $derived(library.hits.filter((h) => h.artifact.status !== 'archived'));
  const recent = $derived(
    [...live].sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at)).slice(0, 8),
  );
  const refCount = (id: string) => library.hitOf(id)?.reference_count ?? 0;

  // ── Projects ──────────────────────────────────────────────────────────────
  let pfilter = $state<ProjectFilter>('all');
  const allProjects = $derived(library.projects.filter((p) => !p.archived));
  const projects = $derived(filterProjects(library.projects, library.artifacts, pfilter, auth.me?.id));
  const PFILTERS: { id: ProjectFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'mine', label: 'Mine' },
    { id: 'epic', label: 'By epic' },
    { id: 'shipped', label: 'Shipped' },
  ];
  function epicLabel(id: string | null): string | null {
    if (!id) return null;
    const s = library.stories[id];
    return s ? `${s.source_key} · ${s.title}` : 'Linked epic';
  }

  // ── Agent activity: designs agents drafted, with their live session ──────
  const agentWork = $derived(
    live
      .filter((h) => h.artifact.created_by_kind === 'agent')
      .sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at))
      .slice(0, 4)
      .map((h) => ({
        a: h.artifact,
        session: h.artifact.created_session_id
          ? (ws.getSession(h.artifact.created_session_id) ?? null)
          : null,
      })),
  );

  // ── Prompt hero: Generate = create the draft + hand the brief to Otto (one
  // generate turn, or a variants run), then open it on the Otto tab. "Draft
  // only" keeps the brief on an empty draft without an agent. ─────────────
  let prompt = $state('');
  let heroStudio = $state<DesignStudio>('frames');
  let heroProject = $state('');
  let creating = $state(false);
  const heroStudios = STUDIOS.filter((s) => s.formats.length > 0);
  const TRY = ['Launch page with a product hero', 'Social tile in portrait', 'Checkout flow diagram', 'Product shot on a plinth'];
  const TRY_STUDIO: DesignStudio[] = ['frames', 'graphics', 'whiteboard', '3d'];
  let variants = $state(1);
  let useRefs = $state(true);
  /** The team's best matches for the brief — offered to Otto first as [R1..R3]. */
  let refHits = $state<DesignSearchHit[]>([]);
  let refCtl: AbortController | null = null;
  let refTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const p = prompt.trim();
    const on = useRefs;
    const wsId = ws.currentId;
    if (refTimer) clearTimeout(refTimer);
    if (!on || p.length < 4 || !wsId) {
      refCtl?.abort();
      refHits = [];
      return;
    }
    refTimer = setTimeout(() => void loadRefs(p, wsId), 450);
  });
  async function loadRefs(p: string, wsId: string): Promise<void> {
    refCtl?.abort();
    const mine = new AbortController();
    refCtl = mine;
    try {
      const hits = await suggestReferences(p, wsId, mine.signal);
      if (refCtl === mine) refHits = hits;
    } catch {
      /* no preview; Otto still searches the library itself */
    }
  }

  async function generate(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'New designs are filed under a workspace.');
      return;
    }
    creating = true;
    try {
      const r = await generateFromBrief({
        workspaceId: wsId,
        studio: heroStudio,
        format: studioInfo(heroStudio).formats[0],
        title: titleFromPrompt(prompt),
        projectId: heroProject || null,
        brief: prompt,
        variants,
        references: useRefs ? refHits.map(referenceOf) : [],
      });
      prompt = '';
      if (r.assistError) toasts.warn('Draft created, but Otto couldn’t start', r.assistError);
      router.go(`design/a/${encodeURIComponent(r.artifactId)}/otto`);
    } catch (e) {
      toastError('Couldn’t create the draft', e);
    } finally {
      creating = false;
    }
  }

  async function createFromPrompt(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'New designs are filed under a workspace.');
      return;
    }
    creating = true;
    try {
      const id = await createDesign({
        workspaceId: wsId,
        studio: heroStudio,
        format: studioInfo(heroStudio).formats[0],
        title: titleFromPrompt(prompt),
        projectId: heroProject || null,
        brief: prompt,
      });
      prompt = '';
      router.go(`design/a/${encodeURIComponent(id)}`);
    } catch (e) {
      toastError('Couldn’t create the draft', e);
    } finally {
      creating = false;
    }
  }

  function openStudio(id: DesignStudio): void {
    if (id === 'brand') router.go('design/brand');
    else if (id === 'spatial') router.go('design/spatial');
    else router.go(`design/studio/${id}`);
  }

  // ── Global search (header field) ─────────────────────────────────────────
  let q = $state('');
  let results = $state<DesignSearchHit[]>([]);
  let searching = $state(false);
  let searchError = $state<string | null>(null);
  let ctl: AbortController | null = null;
  let debounce: ReturnType<typeof setTimeout> | null = null;
  const searchActive = $derived(q.trim().length > 0);

  $effect(() => {
    const term = q.trim();
    if (debounce) clearTimeout(debounce);
    if (!term) {
      ctl?.abort();
      results = [];
      searching = false;
      searchError = null;
      return;
    }
    searching = true;
    debounce = setTimeout(() => void runSearch(term), 220);
  });

  async function runSearch(term: string): Promise<void> {
    ctl?.abort();
    const mine = (ctl = new AbortController());
    try {
      const hits = await searchApi(term, { limit: 60 }, mine.signal);
      if (ctl !== mine) return;
      results = hits;
      searchError = null;
    } catch (e) {
      if (isAbortError(e) || ctl !== mine) return;
      searchError = loadErrorText(e);
    } finally {
      if (ctl === mine) searching = false;
    }
  }

  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === 'Escape' && q) {
      e.preventDefault();
      q = '';
    }
  }

  function goView(v: 'grid' | 'spatial'): void {
    router.go(v === 'grid' ? 'design' : 'design/spatial');
  }
;
async () => {




 { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP0 = new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {   "title":`Design Hall`,"class":`dh-header`,tabs:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`tablist`,"aria-label":`Lobby view`,});
       { svelteHTML.createElement("button", {          "role":`tab`,"aria-selected":view === 'grid',"tabindex":view === 'grid' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => goView('grid'),});view === 'grid';
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":view === 'spatial',"tabindex":view === 'spatial' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => goView('spatial'),...__sveltets_2_empty({"data-testid":`design-spatial-tab`}),});view === 'spatial';
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`gallery`,"size":12,}});}   { svelteHTML.createElement("span", { "class":`beta`,});  }
       }
     }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(!viewport.isPhone){;__sveltets_2_ensureSnippet(searchField());}
     { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`download`}),"onclick":onimport,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
     }
     { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":onnew,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`design-new`}),...__sveltets_2_empty({"data-keep":true}),});searchActive || view === 'spatial';
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {tabs, actions} = $$_redaeHegaP0.$$prop_def;/*Ωignore_endΩ*/
  
  
 PageHeader}

if(view === 'spatial'){
   { const $$_weiVlaitapS0C = __sveltets_2_ensureComponent(SpatialView); new $$_weiVlaitapS0C({ target: __sveltets_2_any(), props: {}});}
}else{
 { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
   { svelteHTML.createElement("div", { "class":`lobby-wrap`,});
  if(viewport.isPhone){ { svelteHTML.createElement("div", { "class":`phone-search`,});;__sveltets_2_ensureSnippet(searchField()); }}
   { svelteHTML.createElement("div", {   "class":`lobby`,...__sveltets_2_empty({"data-testid":`design-lobby`}),});
     { svelteHTML.createElement("div", { "class":`main`,});
      if(searchActive){
         { svelteHTML.createElement("section", { "aria-labelledby":`dh-results-h`,});
           { svelteHTML.createElement("div", { "class":`sec-head`,});
             { svelteHTML.createElement("h2", { "id":`dh-results-h`,});  }
             { svelteHTML.createElement("span", { "class":`dim`,});searching ? 'Searching…' : `${results.length} for “${q.trim()}” · shipped first`; }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (q = ''),});  }
           }
          if(searchError){
             {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: 'Couldn’t search. Check the connection and try again.', assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`inline-err`,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
               { svelteHTML.createElement("span", { "title":searchError,});        }
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void runSearch(q.trim()),});  }
             }}
          } else if (!searching && results.length === 0){
             { svelteHTML.createElement("p", { "class":`dim`,});   q.trim();              }
          }else{
             { svelteHTML.createElement("div", { "class":`cards`,});
                 for(let h of __sveltets_2_ensureArray(results)){h.artifact.id;
                 { const $$_draCtcafitrA6C = __sveltets_2_ensureComponent(ArtifactCard); new $$_draCtcafitrA6C({ target: __sveltets_2_any(), props: {      "artifact":h.artifact,"stories":library.storyKeys(h.artifact.id),"referenceCount":h.reference_count,}});}
              }
             }
          }
         }
      }else{
        
         { svelteHTML.createElement("section", {   "class":`hero card`,"aria-labelledby":`dh-hero-h`,});
           { svelteHTML.createElement("h2", { "id":`dh-hero-h`,});      }
           { svelteHTML.createElement("p", { "class":`dim`,});                        }
           { svelteHTML.createElement("div", { "class":`composer`,});
             { svelteHTML.createElement("textarea", {                "dir":`auto`,"class":`input`,"rows":2,"placeholder":`Describe it… e.g. “a launch page for the loyalty programme with a 3D card hero”`,"aria-label":`Describe the design`,"bind:value":prompt,"onkeydown":(e) => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && prompt.trim() && !creating) {
                  e.preventDefault();
                  void generate();
                }
              },...__sveltets_2_empty({"data-testid":`design-prompt`}),});/*Ωignore_startΩ*/() => prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
             { svelteHTML.createElement("div", { "class":`composer-bar`,});
               { svelteHTML.createElement("label", { "class":`sel`,});
                 { svelteHTML.createElement("span", {});  }
                 { svelteHTML.createElement("select", {     "class":`input`,"bind:value":heroStudio,"aria-label":`Studio for the draft`,});/*Ωignore_startΩ*/() => heroStudio = __sveltets_2_any(null);/*Ωignore_endΩ*/
                     for(let s of __sveltets_2_ensureArray(heroStudios)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.name; }}
                 }
               }
               { svelteHTML.createElement("label", { "class":`sel`,});
                 { svelteHTML.createElement("span", {});  }
                 { svelteHTML.createElement("select", {     "class":`input`,"bind:value":heroProject,"aria-label":`Project for the draft`,});/*Ωignore_startΩ*/() => heroProject = __sveltets_2_any(null);/*Ωignore_endΩ*/
                   { svelteHTML.createElement("option", {"value":"",});  }
                     for(let p of __sveltets_2_ensureArray(allProjects)){p.id; { svelteHTML.createElement("option", { "value":p.id,});p.name; }}
                 }
               }
               { svelteHTML.createElement("label", { "class":`sel`,});
                 { svelteHTML.createElement("span", {});  }
                 { svelteHTML.createElement("select", {       "class":`input`,"bind:value":variants,"aria-label":`How many directions Otto drafts`,...__sveltets_2_empty({"data-testid":`design-prompt-variants`}),});/*Ωignore_startΩ*/() => variants = __sveltets_2_any(null);/*Ωignore_endΩ*/
                     for(let n of __sveltets_2_ensureArray(([1, 2, 3, 4]))){n; { svelteHTML.createElement("option", { "value":n,});n; }}
                 }
               }
               { svelteHTML.createElement("button", {          "class":`pill-toggle`,"aria-pressed":useRefs,"onclick":() => (useRefs = !useRefs),"title":`Offer your team’s closest past designs to Otto first, as references it can cite`,...__sveltets_2_empty({"data-testid":`design-prompt-refs`}),});useRefs;
                if(useRefs){ { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}  
               }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("span", { "class":`go`,});
               { svelteHTML.createElement("button", {         "class":`btn ghost`,"disabled":!prompt.trim() || creating,"onclick":createFromPrompt,...__sveltets_2_empty({"data-testid":`design-prompt-create`}),"title":`Save the brief on an empty draft, without asking Otto`,});
                 
               }
               { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":!prompt.trim() || creating,"onclick":generate,...__sveltets_2_empty({"data-testid":`design-prompt-generate`}),"title":`Create the draft and ask Otto to design it (⌘Enter)`,});
                 { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":13,}});} creating ? 'Starting…' : 'Generate';
               }
               }
             }
            if(useRefs && refHits.length){
               { svelteHTML.createElement("div", {     "class":`refs`,"aria-label":`References Otto gets first`,...__sveltets_2_empty({"data-testid":`design-prompt-ref-chips`}),});
                 { svelteHTML.createElement("span", { "class":`dim`,});  }
                    for(let h of __sveltets_2_ensureArray(refHits)){let i = 1;h.artifact.id;
                   { svelteHTML.createElement("a", {     "class":`chip`,"href":`#/design/a/${encodeURIComponent(h.artifact.id)}`,"title":`${h.artifact.title} (${h.artifact.status}) — offered as R${i + 1}`,});
                    i + 1;  h.artifact.title;
                   }
                }
               }
            }
           }
           { svelteHTML.createElement("div", { "class":`try`,});
             { svelteHTML.createElement("span", { "class":`dim`,});  }
                for(let t of __sveltets_2_ensureArray(TRY)){let i = 1;t;
               { svelteHTML.createElement("button", {   "class":`chip as-btn`,"onclick":() => { prompt = t; heroStudio = TRY_STUDIO[i]; },});t; }
            }
           }
           { svelteHTML.createElement("p", { "class":`phase`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}                 }
         }

        
         { svelteHTML.createElement("section", { "aria-labelledby":`dh-studios-h`,});
           { svelteHTML.createElement("div", { "class":`sec-head`,});
             { svelteHTML.createElement("h2", { "id":`dh-studios-h`,});  }
             { svelteHTML.createElement("span", { "class":`dim`,});      }
           }
           { svelteHTML.createElement("div", {   "class":`studios`,...__sveltets_2_empty({"data-testid":`design-studios`}),});
               for(let s of __sveltets_2_ensureArray(STUDIOS)){s.id;
               { svelteHTML.createElement("button", {     "class":`studio`,"onclick":() => openStudio(s.id),...__sveltets_2_empty({"data-testid":`design-studio-${s.id}`}),});
                 { const $$_egdaBoidutS7C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS7C({ target: __sveltets_2_any(), props: {    "studio":s.id,"size":28,}});}
                 { svelteHTML.createElement("span", { "class":`s-name`,});s.name; }
                 { svelteHTML.createElement("span", { "class":`s-blurb`,});s.blurb; }
                if(s.phase === 'planned'){
                   { svelteHTML.createElement("span", {   "class":`s-tag`,"title":`Planned for ${s.roadmap}`,});s.roadmap; }
                }
               }
            }
           }
         }

        
         { svelteHTML.createElement("section", { "aria-labelledby":`dh-continue-h`,});
           { svelteHTML.createElement("div", { "class":`sec-head`,});
             { svelteHTML.createElement("h2", { "id":`dh-continue-h`,});  }
             { svelteHTML.createElement("span", { "class":`dim`,});  }
           }
          if(library.loading && !library.loaded){
             { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {    "rows":1,"height":220,}});}
          } else if (library.error && !library.loaded){
             { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the design library`,"error":library.error,"empty":true,"onretry":() => void library.load(),}});}
          } else if (recent.length === 0){
             { svelteHTML.createElement("p", { "class":`dim`,});                      }
          }else{
             { svelteHTML.createElement("div", {   "class":`strip`,...__sveltets_2_empty({"data-testid":`design-continue`}),});
                 for(let h of __sveltets_2_ensureArray(recent)){h.artifact.id;
                  { const $$_draCtcafitrA6C = __sveltets_2_ensureComponent(ArtifactCard); new $$_draCtcafitrA6C({ target: __sveltets_2_any(), props: {      "artifact":h.artifact,"stories":library.storyKeys(h.artifact.id),"referenceCount":refCount(h.artifact.id),"live":true,}});}
              }
             }
          }
         }

        
         { svelteHTML.createElement("section", { "aria-labelledby":`dh-projects-h`,});
           { svelteHTML.createElement("div", { "class":`sec-head`,});
             { svelteHTML.createElement("h2", { "id":`dh-projects-h`,});  }
             { svelteHTML.createElement("span", { "class":`dim`,});projects.length;  allProjects.length; }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(allProjects.length){
               { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Filter projects`,});
                   for(let f of __sveltets_2_ensureArray(PFILTERS)){f.id;
                   { svelteHTML.createElement("button", {    "aria-pressed":pfilter === f.id,"onclick":() => (pfilter = f.id),});pfilter === f.id;f.label; }
                }
               }
            }
           }
          if(library.loading && !library.loaded){
             { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {    "rows":2,"height":120,}});}
          } else if (allProjects.length === 0){
             { svelteHTML.createElement("div", { "class":`none`,});
               { svelteHTML.createElement("p", { "class":`dim`,});             }
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onnewproject,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
             }
          } else if (projects.length === 0){
             { svelteHTML.createElement("p", { "class":`dim`,});      { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => (pfilter = 'all'),});  } }
          }else{
            if(library.truncated){
               { svelteHTML.createElement("p", {   "class":`dim`,...__sveltets_2_empty({"data-testid":`lobby-truncated`}),});      LIBRARY_LIMIT;        }
            }
             { svelteHTML.createElement("div", { "class":`projects`,});
                  for(let p of __sveltets_2_ensureArray(projects)){let i = 1;p.id;
                 { const $$_draCtcejorP6C = __sveltets_2_ensureComponent(ProjectCard); new $$_draCtcejorP6C({ target: __sveltets_2_any(), props: {        "project":p,"artifacts":library.artifacts,"epicLabel":epicLabel(p.epic_story_id),"live":i < 6,}});}
              }
             }
          }
         }

        
         { svelteHTML.createElement("section", { "aria-labelledby":`dh-linked-h`,});
           { svelteHTML.createElement("div", { "class":`sec-head`,});
             { svelteHTML.createElement("h2", { "id":`dh-linked-h`,});   }
             { svelteHTML.createElement("span", { "class":`dim`,});       }
           }
          if(library.loading && !library.loaded){
             { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {    "rows":3,"height":32,}});}
          }else{
             { const $$_tcudorPoTdekniL5C = __sveltets_2_ensureComponent(LinkedToProduct); new $$_tcudorPoTdekniL5C({ target: __sveltets_2_any(), props: {}});}
          }
         }
      }
     }

     { svelteHTML.createElement("aside", {   "class":`rail`,"aria-label":`Learning and activity`,});
       { const $$_draCdenraeL4C = __sveltets_2_ensureComponent(LearnedCard); new $$_draCdenraeL4C({ target: __sveltets_2_any(), props: {}});}
      if(agentWork.length){
         { svelteHTML.createElement("section", {   "class":`card activity`,"aria-labelledby":`dh-activity-h`,});
           { svelteHTML.createElement("header", {});
             { svelteHTML.createElement("span", { "class":`ico`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":14,}});} }
             { svelteHTML.createElement("h2", { "id":`dh-activity-h`,});  }
           }
           { svelteHTML.createElement("ul", {});
               for(let w of __sveltets_2_ensureArray(agentWork)){w.a.id;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("div", { "class":`act-line`,});
                  if(w.session){ { const $$_toDsutatS8C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS8C({ target: __sveltets_2_any(), props: {   "state":sessionState(w.session, ws.statusMap[w.session.id], ws.needsYou[w.session.id] === true, { stale: events.state !== 'connected' }),}});}}
                   { svelteHTML.createElement("span", {}); { svelteHTML.createElement("span", { "class":`who`,});w.session?.title ?? w.a.created_session_title ?? 'An agent'; } 
                     { svelteHTML.createElement("a", { "href":`#/design/a/${encodeURIComponent(w.a.id)}`,});w.a.title; } }
                 }
                 { svelteHTML.createElement("div", { "class":`act-meta`,});
                   { svelteHTML.createElement("span", { "title":new Date(w.a.updated_at).toLocaleString(),});rel(w.a.updated_at); }
                  if(w.session){
                     { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => ws.navigateToSession(w.session!.id),});  }
                  }
                 }
               }
            }
           }
         }
      }
       { svelteHTML.createElement("nav", {   "class":`card quick`,"aria-label":`Design Hall pages`,});
        
         { svelteHTML.createElement("a", { "href":`#/design/brand`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`palette`,"size":14,}});}   }
         { svelteHTML.createElement("a", { "href":`#/canvas`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":asIcon('shapes'),"size":14,}});}    }
       }
     }
   }
   }
 PageBody}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Lobby__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Lobby__SvelteComponent_ = ReturnType<typeof Lobby__SvelteComponent_>;
/*Ωignore_endΩ*/export default Lobby__SvelteComponent_;