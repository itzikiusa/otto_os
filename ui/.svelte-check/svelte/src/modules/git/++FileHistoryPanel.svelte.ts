///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// One file's history, opened from a diff header's ⋯ menu (via `gitBridge`).
// `follow=true` walks THROUGH renames — a history that stops at the rename is
// the reason the flag exists. Clicking a commit selects it in the graph.
import type { CommitInfo } from '../../lib/api/types';
import { api } from '../../lib/api/client';
import { gitBridge } from './gitBridge.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Skeleton from '../../lib/components/Skeleton.svelte';
;

  interface Props {
    repoId: string;
    path: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { repoId, path, onclose }: Props = $props();

  let commits = $state.raw<CommitInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let retryRev = $state(0);
  /** One page of the log. A full page means there may be more — say so and
   *  offer "Load more" instead of silently stopping at the page size. */
  const PAGE = 200;
  let hasMore = $state(false);
  let moreBusy = $state(false);
  let moreError = $state<string | null>(null);
  const logUrl = (id: string, p: string, skip: number): string =>
    `/repos/${id}/log?all=true&limit=${PAGE}&skip=${skip}&follow=true&path=${encodeURIComponent(p)}`;

  $effect(() => {
    void retryRev;
    const id = repoId;
    const p = path;
    loading = true;
    error = null;
    hasMore = false;
    moreError = null;
    // Switching files aborts the previous request; a late response is dropped.
    const ctl = new AbortController();
    api
      .get<CommitInfo[]>(logUrl(id, p, 0), ctl.signal)
      .then((rows) => {
        if (ctl.signal.aborted) return;
        commits = rows;
        hasMore = rows.length >= PAGE;
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted) return;
        commits = [];
        error = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) loading = false;
      });
    return () => ctl.abort();
  });

  async function loadMore(): Promise<void> {
    if (moreBusy) return;
    const id = repoId;
    const p = path;
    moreBusy = true;
    moreError = null;
    try {
      const rows = await api.get<CommitInfo[]>(logUrl(id, p, commits.length));
      if (id !== repoId || p !== path) return;
      const seen = new Set(commits.map((c) => c.sha));
      commits = [...commits, ...rows.filter((c) => !seen.has(c.sha))];
      hasMore = rows.length >= PAGE;
    } catch (e) {
      if (id === repoId && p === path) moreError = loadErrorText(e);
    } finally {
      moreBusy = false;
    }
  }

  function when(date: string): string {
    const d = new Date(date);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString();
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`fh`,"aria-label":`File history`,});
   { svelteHTML.createElement("header", { "class":`fh-head`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":13,}});}
     { svelteHTML.createElement("span", {   "class":`fh-title mono`,"title":path,});path; }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onclose,"aria-label":`Close file history`,"title":`Close file history`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
     }
   }

   { svelteHTML.createElement("div", { "class":`fh-body`,});
    if(loading){
       { svelteHTML.createElement("div", { "class":`fh-pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"height":30,}});} }
    } else if (error){
       { svelteHTML.createElement("div", { "class":`fh-pad`,}); { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {       "what":`this file’s history`,error,"empty":true,"onretry":() => retryRev++,"variant":`compact`,}});} }
    } else if (commits.length === 0){
       { svelteHTML.createElement("p", { "class":`fh-msg`,});     }
    }else{
       { svelteHTML.createElement("ul", { "class":`fh-list`,});
           for(let c of __sveltets_2_ensureArray(commits)){c.sha;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {   "class":`fh-row`,"onclick":() => gitBridge.focusCommit(repoId, c.sha),});
               { svelteHTML.createElement("span", {   "class":`fh-subject`,"title":c.subject,});c.subject; }
               { svelteHTML.createElement("span", { "class":`fh-meta`,});
                 { svelteHTML.createElement("span", { "class":`mono`,});c.short_sha; }
                 c.author;  when(c.date);
               }
             }
           }
        }
       }
      if(hasMore){
         { svelteHTML.createElement("div", { "class":`fh-pad fh-more`,});
           { svelteHTML.createElement("span", { "class":`dim`,});   commits.length;  }
           { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":moreBusy,"onclick":() => void loadMore(),});
            moreBusy ? 'Loading older commits…' : 'Load more';
           }
          if(moreError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('span'),({ text: `Couldn’t load more: ${moreError}`, assertive: true })));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {   "class":`fh-more-err`,});   moreError; }}}
         }
      }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FileHistoryPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FileHistoryPanel__SvelteComponent_ = ReturnType<typeof FileHistoryPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default FileHistoryPanel__SvelteComponent_;