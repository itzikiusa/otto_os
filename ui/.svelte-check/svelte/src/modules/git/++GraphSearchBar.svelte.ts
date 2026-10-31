///<reference types="svelte" />
;// Commit search above the graph. ⌘/Ctrl+F focuses the box; typing runs
// `log --all --grep=<q>` (or `--author=<q>`) on the daemon after a 250 ms
// debounce, and clicking a hit asks the graph to select that commit through
// `gitBridge` (no prop path from here into GraphView).
//
// The search is SERVER-backed rather than a filter of the rows the graph
// happens to have loaded: the daemon is local, `--fixed-strings` matching is
// cheap, and it finds commits outside the loaded page — "search all history"
// with no second mode to toggle.

import type { CommitInfo } from '../../lib/api/types';
import { toastError } from '../../lib/toastError';
import { api, isAbortError } from '../../lib/api/client';
import { gitBridge } from './gitBridge.svelte';
import Icon from '../../lib/components/Icon.svelte';
;

  interface Props {
    repoId: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId }: Props = $props();

  let q = $state('');
  /** Search the AUTHOR instead of the subject. */
  let byAuthor = $state(false);
  let results = $state<CommitInfo[]>([]);
  let searching = $state(false);
  /** The query `results` belong to ('' = no active search). */
  let ran = $state('');
  let listOpen = $state(false);
  let rootEl = $state<HTMLDivElement | null>(null);
  /** Result-list height cap: the room under the bar in the window (not a fixed
   *  50vh), re-measured each time the list opens. */
  let listMaxH = $state(320);
  $effect(() => {
    if (!listOpen || !rootEl) return;
    listMaxH = Math.max(160, Math.min(480, window.innerHeight - rootEl.getBoundingClientRect().bottom - 12));
  });
  let inputEl: HTMLInputElement | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inflight: AbortController | null = null;

  // Re-running a repo switch must not leave another repo's hits on screen.
  $effect(() => {
    void repoId;
    return () => clear();
  });

  function onWindowKeydown(e: KeyboardEvent): void {
    // Only while this bar is mounted — i.e. the graph is the visible tab — so
    // ⌘F keeps its browser meaning everywhere else.
    if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      inputEl?.focus();
      inputEl?.select();
    }
  }

  function clear(): void {
    if (timer) clearTimeout(timer);
    timer = null;
    inflight?.abort();
    inflight = null;
    q = '';
    ran = '';
    results = [];
    searching = false;
    listOpen = false;
  }

  function schedule(): void {
    if (timer) clearTimeout(timer);
    const term = q.trim();
    if (!term) {
      inflight?.abort();
      inflight = null;
      results = [];
      ran = '';
      searching = false;
      listOpen = false;
      return;
    }
    searching = true;
    listOpen = true;
    timer = setTimeout(() => void run(term), 250);
  }

  async function run(term: string): Promise<void> {
    inflight?.abort();
    const ac = new AbortController();
    inflight = ac;
    const field = byAuthor ? 'author' : 'grep';
    try {
      const rows = await api.get<CommitInfo[]>(
        `/repos/${repoId}/log?all=true&limit=200&${field}=${encodeURIComponent(term)}`,
        ac.signal,
      );
      if (ac !== inflight) return; // a newer query already won
      results = rows;
      ran = term;
    } catch (e) {
      if (isAbortError(e)) return;
      results = [];
      ran = term;
      toastError('Couldn’t search the graph', e);
    } finally {
      if (ac === inflight) {
        searching = false;
        inflight = null;
      }
    }
  }

  function toggleAuthor(): void {
    byAuthor = !byAuthor;
    if (q.trim()) schedule();
  }

  function pick(c: CommitInfo): void {
    listOpen = false;
    gitBridge.focusCommit(repoId, c.sha);
  }

  function onInputKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (listOpen && results.length > 0) listOpen = false;
      else clear();
    } else if (e.key === 'Enter' && results.length > 0) {
      pick(results[0]);
    }
  }

  function short(date: string): string {
    const d = new Date(date);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString();
  }
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onWindowKeydown,});}

 { const $$_div0 = svelteHTML.createElement("div", {  "class":`gsb`,});rootEl = $$_div0;
   { svelteHTML.createElement("div", { "class":`gsb-row`,});
     { svelteHTML.createElement("div", { "class":`gsb-box`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { const $$_input3 = svelteHTML.createElement("input", {                   "dir":`auto`,"class":`gsb-input`,"bind:value":q,"oninput":schedule,"onkeydown":onInputKeydown,"onfocus":() => (listOpen = results.length > 0),"placeholder":byAuthor ? 'Search commits by author — ⌘F' : 'Search commit messages — ⌘F',"aria-label":byAuthor ? 'Search commits by author' : 'Search commit messages',"spellcheck":`false`,});inputEl = $$_input3;/*Ωignore_startΩ*/() => q = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {          "class":`gsb-chip`,"aria-pressed":byAuthor,"onclick":toggleAuthor,"title":byAuthor ? 'Matching the author — click to search commit messages instead' : 'Match the author instead of the commit message',});byAuthor;
        
       }
      if(q){
         { svelteHTML.createElement("button", {       "class":`gsb-x`,"onclick":clear,"aria-label":`Clear search`,"title":`Clear search`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
      }
     }

    if(ran){
       { svelteHTML.createElement("div", { "class":`gsb-banner`,});
        if(searching){ }else{
          
          results.length >= 200 ? '200+ matches' : `${results.length} match${results.length === 1 ? '' : 'es'}`;
           ran; }
         { svelteHTML.createElement("button", {   "class":`gsb-link`,"onclick":clear,});  }
       }
    }
   }

  if(listOpen && results.length > 0){
    
     { svelteHTML.createElement("ul", {   "class":`gsb-results`,"style":`max-height:${listMaxH}px`,});
         for(let c of __sveltets_2_ensureArray(results)){c.sha;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {   "class":`gsb-result`,"onclick":() => pick(c),});
             { svelteHTML.createElement("span", { "class":`mono gsb-sha`,});c.short_sha; }
             { svelteHTML.createElement("span", {   "class":`gsb-subject`,"title":c.subject,});c.subject; }
             { svelteHTML.createElement("span", { "class":`gsb-author`,});c.author; }
             { svelteHTML.createElement("span", { "class":`gsb-date`,});short(c.date); }
           }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GraphSearchBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GraphSearchBar__SvelteComponent_ = ReturnType<typeof GraphSearchBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default GraphSearchBar__SvelteComponent_;