///<reference types="svelte" />
;
import { onMount } from 'svelte';
import { okfMetadata } from './okfMetadata';
import { rel } from '../../lib/stores/now.svelte';

;type $$ComponentProps =  { frontmatter: unknown };function $$render() {

  
  
  
  /** Status values are raw frontmatter words ("stable"): sentence-case them. */
  const cap = (v: string): string => (v ? v[0].toUpperCase() + v.slice(1) : v);
  let { frontmatter }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let now = $state(Date.now());
  onMount(() => { const timer = setInterval(() => now = Date.now(), 30_000); return () => clearInterval(timer); });
  const meta = $derived(okfMetadata(frontmatter, now));
;
async () => {
 { svelteHTML.createElement("div", {   "class":`knowledge`,"aria-label":`Knowledge provenance`,});
   { svelteHTML.createElement("div", { "class":`badges`,}); { svelteHTML.createElement("span", {});meta.tier; } { svelteHTML.createElement("span", {});cap(meta.status); }if(meta.stale){ { svelteHTML.createElement("span", { "class":`warning`,});  }} }
  if(meta.generatedAt){ { svelteHTML.createElement("p", {});   { svelteHTML.createElement("time", {   "datetime":meta.generatedAt,"title":meta.generatedAt,});rel(meta.generatedAt) || meta.generatedAt; }if(meta.generatedBy){' by ';meta.generatedBy;} }}
  if(meta.staleAt){ { svelteHTML.createElement("p", {   "title":meta.staleAt,});meta.stale || meta.invalidDeadline;if(meta.invalidDeadline){  meta.staleAt;    }else{meta.stale ? 'Went stale' : 'Goes stale'; rel(meta.staleAt);} }}
  if(meta.changedSinceVerification){ { svelteHTML.createElement("p", { "class":`warning`,});       }}
      for(let event of __sveltets_2_ensureArray(meta.verified)){let i = 1;i; { svelteHTML.createElement("p", {});  event.by;   { svelteHTML.createElement("time", {   "datetime":event.at,"title":event.at,});rel(event.at); } }}
  if(meta.sources.length){
     { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {}); meta.sources.length;  }
          for(let source of __sveltets_2_ensureArray(meta.sources)){let i = 1;i;
         { svelteHTML.createElement("div", { "class":`source`,}); { svelteHTML.createElement("strong", {});source.title || source.id || 'Source'; } { svelteHTML.createElement("div", {});source.resource; }
          if(source.author || source.modified){ { svelteHTML.createElement("small", {});source.author; source.modified; }}
         }
      }
     }
  }
   { svelteHTML.createElement("p", { "class":`hint`,});            }
  if(meta.attestedComputation){ { svelteHTML.createElement("p", { "class":`hint`,});           }}
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const KnowledgeMetadata__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type KnowledgeMetadata__SvelteComponent_ = ReturnType<typeof KnowledgeMetadata__SvelteComponent_>;
/*Ωignore_endΩ*/export default KnowledgeMetadata__SvelteComponent_;