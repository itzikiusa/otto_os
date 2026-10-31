///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Spatial Hall (beta) — a showcase view, not the way to browse: one gallery
// wall with a bay per project (framed thumbnails, 3D work on plinths). Pure
// CSS in Phase 0 (the three.js room is v3). Click-to-open only; the bay chips
// jump between bays; "Back to grid" is always there.
import { untrack } from 'svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { router } from '../../lib/router.svelte';
import { lsGet, lsSet } from '../../lib/storage';
import type { DesignArtifact } from '../../lib/api/types';
import ArtifactThumb from './ArtifactThumb.svelte';
import { library } from './library.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  $effect(() => {
    untrack(() => {
      if (!library.loaded) void library.load();
    });
  });

  const BANNER_KEY = 'otto.design.spatialBanner';
  let bannerOpen = $state(lsGet(BANNER_KEY) !== 'dismissed');
  function dismissBanner(): void {
    bannerOpen = false;
    lsSet(BANNER_KEY, 'dismissed');
  }

  interface Bay {
    id: string;
    name: string;
    sub: string;
    flat: DesignArtifact[];
    plinth: DesignArtifact | null;
  }
  const bays = $derived.by((): Bay[] => {
    const live = library.artifacts.filter((a) => a.status !== 'archived');
    const out: Bay[] = [];
    for (const p of library.projects.filter((x) => !x.archived)) {
      const mine = live
        .filter((a) => a.project_id === p.id)
        .sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      if (!mine.length) continue;
      const plinth = mine.find((a) => a.studio === '3d') ?? null;
      out.push({
        id: p.id,
        name: p.name,
        sub: `${plural(mine.length, 'artifact')}`,
        flat: mine.filter((a) => a !== plinth).slice(0, 4),
        plinth,
      });
    }
    const unfiled = live.filter((a) => !a.project_id).sort((a, b) => b.updated_at.localeCompare(a.updated_at));
    if (unfiled.length) {
      const plinth = unfiled.find((a) => a.studio === '3d') ?? null;
      out.push({ id: '_unfiled', name: 'Unfiled', sub: `${unfiled.length} artifacts`, flat: unfiled.filter((a) => a !== plinth).slice(0, 4), plinth });
    }
    return out;
  });

  function jump(id: string): void {
    const el = document.getElementById(`bay-${id}`);
    const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    el?.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', inline: 'center', block: 'nearest' });
    el?.focus({ preventScroll: true });
  }

  const open = (a: DesignArtifact) => router.go(`design/a/${encodeURIComponent(a.id)}`);
;
async () => {

 { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
   { svelteHTML.createElement("div", {   "class":`room`,...__sveltets_2_empty({"data-testid":`design-spatial`}),});
    if(bannerOpen){
       { svelteHTML.createElement("div", {   "class":`banner`,"role":`note`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
         { svelteHTML.createElement("span", {});                 }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":dismissBanner,"aria-label":`Dismiss`,"title":`Dismiss`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
       }
    }
    if(library.loading && !library.loaded){
       { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":2,"height":180,}});} }
    } else if (library.error && !library.loaded){
       { svelteHTML.createElement("div", {   "class":`pad err`,"role":`alert`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}     
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void library.load(),});  }
       }
    } else if (bays.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`gallery`,"title":`Nothing to exhibit yet`,"body":`Projects with designs become bays on this wall. File a design into a project to see it here.`,}});}
    }else{
       { svelteHTML.createElement("div", {     "class":`wall`,"role":`list`,"aria-label":`Project bays`,});
           for(let b of __sveltets_2_ensureArray(bays)){b.id;
           { svelteHTML.createElement("div", {         "class":`bay`,"id":`bay-${b.id}`,"role":`listitem`,"tabindex":-1,"aria-label":b.name,});
             { svelteHTML.createElement("div", {   "class":`spot`,"aria-hidden":`true`,}); }
             { svelteHTML.createElement("div", { "class":`works`,});
                 for(let a of __sveltets_2_ensureArray(b.flat)){a.id;
                 { svelteHTML.createElement("button", {     "class":`frame`,"onclick":() => open(a),"title":`Open ${a.title}`,});
                   { svelteHTML.createElement("span", { "class":`canvas`,});  { const $$_bmuhTtcafitrA7C = __sveltets_2_ensureComponent(ArtifactThumb); new $$_bmuhTtcafitrA7C({ target: __sveltets_2_any(), props: {  "artifact":a,"live":true,}});} }
                   { svelteHTML.createElement("span", { "class":`work-caption`,});a.title; }
                 }
              }
             }
            if(b.plinth){
               { svelteHTML.createElement("button", {     "class":`plinth`,"onclick":() => open(b.plinth!),"title":`Open ${b.plinth.title}`,});
                 { svelteHTML.createElement("span", { "class":`obj`,}); { const $$_bmuhTtcafitrA6C = __sveltets_2_ensureComponent(ArtifactThumb); new $$_bmuhTtcafitrA6C({ target: __sveltets_2_any(), props: {  "artifact":b.plinth,}});} }
                 { svelteHTML.createElement("span", {   "class":`base`,"aria-hidden":`true`,}); }
                 { svelteHTML.createElement("span", { "class":`work-caption`,});b.plinth.title; }
               }
            }
             { svelteHTML.createElement("div", { "class":`plaque`,});
               { svelteHTML.createElement("strong", {});b.name; }
               { svelteHTML.createElement("span", {});b.sub; }
             }
           }
        }
       }
       { svelteHTML.createElement("div", {   "class":`floor`,"aria-hidden":`true`,}); }
       { svelteHTML.createElement("nav", {   "class":`dock`,"aria-label":`Bays`,});
           for(let b of __sveltets_2_ensureArray(bays)){b.id;
           { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => jump(b.id),});b.name; }
        }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('design'),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});}    }
       }
    }
   }
 PageBody}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SpatialView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SpatialView__SvelteComponent_ = ReturnType<typeof SpatialView__SvelteComponent_>;
/*Ωignore_endΩ*/export default SpatialView__SvelteComponent_;