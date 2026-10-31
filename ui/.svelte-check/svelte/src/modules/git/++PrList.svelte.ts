///<reference types="svelte" />
;// PR list with state filter chips. Click → PrDetail route.

import { api, ApiError } from '../../lib/api/client';
import type { PrListResp, PrState, PrSummary } from '../../lib/api/types';
import { router } from '../../lib/router.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import CreatePr from './CreatePr.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { IconName } from '../../lib/components/Icon.svelte';
import { invalidatePr, prListCache, prListKey } from './pr-cache';
;

  interface Props {
    repoId: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId }: Props = $props();

  const PER_PAGE = 50;

  const states: PrState[] = ['open', 'merged', 'declined', 'all'];
  let stateFilter: PrState = $state('open');
  let prs: PrSummary[] = $state([]);
  let page = $state(1);
  let hasMore = $state(false);
  let loadingMore = $state(false);
  /** Client-side filter over the pages already loaded (title/author/branch/#). */
  let query = $state('');
  let loading = $state(true);
  let error = $state('');
  /** A failed "Load more" — shown inline beside the button. It used to write
   *  `error`, which swapped the WHOLE list for the error state and wiped the
   *  rows already on screen (the opposite of what loadMore promises). */
  let moreError = $state('');
  const moreErrId = $props.id();
  /** Cause line of the error state — distinguishes "your token is bad" from
   *  "the provider is down" instead of labelling every failure "unreachable". */
  let errorCause = $state('The provider is unreachable.');
  let createOpen = $state(false);
  // Bumped by the Retry button to re-run the load effect.
  let retryRev = $state(0);
  let loadRevision = 0;
  /** Retry always goes to the network, even over a fresh cache entry. */
  let lastRetryRev = 0;

  $effect(() => {
    const id = repoId;
    const st = stateFilter;
    void retryRev;
    const revision = ++loadRevision;
    let active = true;
    loadingMore = false;
    error = '';
    moreError = '';
    // Stale-while-revalidate: paint the last answer for this repo+state at
    // once; a fresh one (<45 s, not a Retry) needs no request at all.
    const key = prListKey(id, st, 1);
    const hit = prListCache.get(key);
    if (hit) {
      prs = hit.value.items;
      hasMore = hit.value.has_more;
      page = 1;
      loading = false;
      if (hit.fresh && retryRev === lastRetryRev) {
        return () => {
          active = false;
          if (loadRevision === revision) loadRevision++;
        };
      }
    } else {
      loading = true;
    }
    lastRetryRev = retryRev;
    void api
      .get<PrListResp>(`/repos/${id}/prs?state=${st}&page=1&per_page=${PER_PAGE}`)
      .then((r) => {
        prListCache.set(key, r);
        if (!active) return;
        prs = r.items;
        hasMore = r.has_more;
        page = 1;
      })
      .catch((e) => {
        if (!active) return;
        // A failed background revalidation keeps the cached rows on screen.
        if (hit) return;
        prs = [];
        hasMore = false;
        error = e instanceof Error && e.message ? e.message : 'Couldn’t load pull requests.';
        errorCause =
          e instanceof ApiError && (e.status === 401 || e.status === 403)
            ? 'The provider rejected the credentials.'
            : e instanceof ApiError && e.status === 404
              ? 'The repository wasn’t found on the provider.'
              : 'The provider is unreachable.';
      })
      .finally(() => { if (active) loading = false; });
    return () => {
      active = false;
      if (loadRevision === revision) loadRevision++;
    };
  });

  /** Append the next page. Failures toast nothing — the button simply stays
   *  (with an inline note), so a flaky provider never wipes the rows already
   *  on screen. */
  async function loadMore(): Promise<void> {
    if (loadingMore || !hasMore) return;
    const revision = loadRevision;
    loadingMore = true;
    moreError = '';
    try {
      const next = page + 1;
      const r = await api.get<PrListResp>(
        `/repos/${repoId}/prs?state=${stateFilter}&page=${next}&per_page=${PER_PAGE}`,
      );
      if (revision !== loadRevision) return;
      prs = [...prs, ...r.items];
      hasMore = r.has_more;
      page = next;
    } catch (e) {
      if (revision !== loadRevision) return;
      moreError = loadErrorText(e);
    } finally {
      if (revision === loadRevision) loadingMore = false;
    }
  }

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (q === '') return prs;
    return prs.filter((p) =>
      [p.title, p.author, p.source_branch, p.target_branch, String(p.number)].some((v) =>
        v.toLowerCase().includes(q),
      ),
    );
  });

  /** CI chip for a row; `null` when the provider reported no CI at all. */
  function ciChip(state: string | null | undefined): { icon: IconName; cls: string; label: string } | null {
    if (state === 'passing' || state === 'success') return { icon: 'check', cls: 'ok', label: 'CI passing' };
    if (state === 'failing' || state === 'failure') return { icon: 'x', cls: 'bad', label: 'CI failing' };
    if (state === 'pending') return { icon: 'clock', cls: 'warn', label: 'CI running' };
    return null;
  }

  const STATE_LABEL: Record<PrState, string> = {
    open: 'Open',
    merged: 'Merged',
    declined: 'Declined',
    all: 'All',
  };

  const stateColors: Record<string, string> = {
    open: 'ok',
    merged: 'accent',
    declined: 'bad',
  };
;
async () => {

 { svelteHTML.createElement("div", { "class":`prlist`,});
   { svelteHTML.createElement("div", { "class":`pr-toolbar`,});
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Pull request state`,});
         for(let s of __sveltets_2_ensureArray(states)){s;
         { svelteHTML.createElement("button", {      "aria-pressed":stateFilter === s,"onclick":() => (stateFilter = s),});stateFilter === s;
          STATE_LABEL[s];
         }
      }
     }
     { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input pr-search`,"type":`search`,"bind:value":query,"placeholder":`Filter by title, author, branch…`,"aria-label":`Filter pull requests`,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    
    if(loading || error || prs.length > 0){
       { svelteHTML.createElement("button", {   "class":`btn primary small`,"onclick":() => (createOpen = true),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`pr`,"size":12,}});}   
       }
    }
   }

  if(loading){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":4,"height":48,}});}
  } else if (error){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`pull requests`,"error":`${errorCause} ${error}`,"empty":true,"onretry":() => retryRev++,}});}
  } else if (prs.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`pr`,"title":stateFilter === 'all' ? 'No pull requests' : `No ${stateFilter} pull requests`,"body":`Open one from your current branch, or pick another state above.`,"actionLabel":`New pull request`,"actionIcon":`pr`,"onaction":() => (createOpen = true),}});}
  }else{
     { svelteHTML.createElement("div", { "class":`pr-rows`,});
         for(let pr of __sveltets_2_ensureArray(filtered)){pr.number;
        const ci = ciChip(pr.ci_status);
         { svelteHTML.createElement("button", {   "class":`pr-row card`,"onclick":() => router.go(`git/${repoId}/pr/${pr.number}`),});
           { svelteHTML.createElement("div", { "class":`pr-main`,});
             { svelteHTML.createElement("div", { "class":`pr-title`,});
               { svelteHTML.createElement("span", { "class":`pr-num dim`,}); pr.number; }
              pr.title;
             }
             { svelteHTML.createElement("div", { "class":`pr-meta`,});
               { svelteHTML.createElement("span", { "class":`chip ${stateColors[pr.state] ?? ''}`,});STATE_LABEL[pr.state] ?? pr.state; }
              if(ci){
                 { svelteHTML.createElement("span", {       "class":`ci-chip ${ci.cls}`,"title":ci.label,"role":`img`,"aria-label":ci.label,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":ci.icon,"size":12,}});} }
              }
               { svelteHTML.createElement("span", { "class":`dim`,});pr.author; }
               { svelteHTML.createElement("span", {   "class":`mono dim pr-branches`,"title":`${pr.source_branch} → ${pr.target_branch}`,});pr.source_branch;  { svelteHTML.createElement("span", { "class":`dir-arrow`,});  } pr.target_branch; }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("span", {   "class":`dim pr-updated`,"title":new Date(pr.updated_at).toLocaleString(),}); rel(pr.updated_at); }
             }
           }
         }
      }
      if(filtered.length === 0){
         { svelteHTML.createElement("div", { "class":`pr-nomatch dim`,});    query;  }
      }
     }
    if(hasMore){
       { svelteHTML.createElement("div", { "class":`pr-more`,});
         { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":loadingMore,"onclick":loadMore,"aria-describedby":moreError ? moreErrId : undefined,});
          loadingMore ? 'Loading more pull requests…' : moreError ? 'Retry' : 'Load more';
         }
        
         { svelteHTML.createElement("span", {     "id":moreErrId,"class":`pr-more-err`,"role":`status`,});moreError ? `Couldn’t load more: ${moreError}` : ''; }
       }
    }
  }
 }

if(createOpen){
   { const $$_rPetaerC0C = __sveltets_2_ensureComponent(CreatePr); new $$_rPetaerC0C({ target: __sveltets_2_any(), props: {      repoId,"onclose":() => (createOpen = false),"oncreated":(pr) => {
      createOpen = false;
      invalidatePr(repoId);
      router.go(`git/${repoId}/pr/${pr.number}`);
    },}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PrList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PrList__SvelteComponent_ = ReturnType<typeof PrList__SvelteComponent_>;
/*Ωignore_endΩ*/export default PrList__SvelteComponent_;