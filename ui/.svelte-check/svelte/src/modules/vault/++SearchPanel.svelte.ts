///<reference types="svelte" />
;// Full-text search panel (left sidebar mode). FTS5 with `tag:` / `path:` /
// `type:` operators; snippets come highlighted with ‹› markers from bm25.

import { vault } from './vault.svelte';
import { escapeHtml } from '../../lib/html';
import LoadState from '../../lib/components/LoadState.svelte';
function $$render() {

  
  
  
  
  

  let input = $state<HTMLInputElement | undefined>();

  $effect(() => {
    if (vault.leftMode === 'search') input?.focus();
  });

  function renderSnippet(s: string): string {
    // Server marks matches with ‹ › — escape everything else. Only a
    // balanced ‹…› pair (no nested ‹) becomes a <mark>: a stray literal ‹ or
    // › in the note stays text instead of opening/closing tags (S18-26).
    return escapeHtml(s).replace(/‹([^‹›]*)›/g, '<mark>$1</mark>');
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`search`,});
   { const $$_input1 = svelteHTML.createElement("input", {                 "dir":`auto`,"bind:value":vault.searchQuery,"type":`search`,"placeholder":`Search… (tag:x path:y type:z)`,"aria-label":`Search this vault`,"title":`Full-text search. Narrow with tag:, path: or type:. Press Enter to search.`,"oninput":() => {
      if (!vault.searchQuery.trim()) void vault.runSearch();
    },"onkeydown":(e) => e.key === 'Enter' && void vault.runSearch(),});input = $$_input1;/*Ωignore_startΩ*/() => vault.searchQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  if(vault.searching){
     { svelteHTML.createElement("div", { "class":`dim`,});  }
  } else if (vault.searchError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`search results`,"error":vault.searchError,"empty":true,"onretry":() => void vault.runSearch(),}});}
  } else if (vault.searchQuery.trim() && vault.searchQuery.trim() !== vault.searchedQuery){
    
     { svelteHTML.createElement("div", { "class":`dim`,});    }
  } else if (vault.searchQuery.trim() && vault.searchHits.length === 0){
     { svelteHTML.createElement("div", { "class":`dim`,});   vault.searchQuery.trim();  }
  }
  if(!vault.searching && !vault.searchError && vault.searchedQuery && vault.searchQuery.trim() === vault.searchedQuery){
     { svelteHTML.createElement("div", { "class":`hits`,});
         for(let h of __sveltets_2_ensureArray(vault.searchHits)){h.path;
         { svelteHTML.createElement("button", {    "class":`hit`,"onclick":() => void vault.open(h.path),});h.reserved;
           { svelteHTML.createElement("div", { "class":`t`,});h.title; }
           { svelteHTML.createElement("div", {   "class":`p`,"title":h.path,});h.path; }
          if(h.snippet){
            
             { svelteHTML.createElement("div", { "class":`s`,}); renderSnippet(h.snippet); }
          }
         }
      }
      if(vault.searchHits.length >= 50){
         { svelteHTML.createElement("div", { "class":`dim cap-note`,});           }
      }
     }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SearchPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SearchPanel__SvelteComponent_ = ReturnType<typeof SearchPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default SearchPanel__SvelteComponent_;