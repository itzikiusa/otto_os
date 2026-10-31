///<reference types="svelte" />
;
import { plural } from '../../../lib/plural';
// Evidence for one learned rule / proposal: a vertical timeline of the design
// signals it was drawn from (newest first) — when, what kind, which design,
// a one-line summary and a way back to the design. Evidence ids that aren't
// in the loaded signal window are counted, never invented.
import Icon from '../../../lib/components/Icon.svelte';
import { rel } from '../../../lib/stores/now.svelte';
import type { DesignSignal } from '../../../lib/api/types';
import ArtifactThumb from '../ArtifactThumb.svelte';
import { signalSummary, signalTone } from '../model';
import { library } from '../library.svelte';
import { assistSignalSummary, signalKindLabel } from './model';
;

  interface Props {
    title: string;
    rationale: string;
    evidence: string[];
    signals: Record<string, DesignSignal>;
    onclose?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { title, rationale, evidence, signals, onclose }: Props = $props();

  const items = $derived(
    evidence
      .map((id) => signals[id])
      .filter((s): s is DesignSignal => !!s)
      .sort((a, b) => b.created_at.localeCompare(a.created_at)),
  );
  const missing = $derived(evidence.length - items.length);
  const summary = (s: DesignSignal) => assistSignalSummary(s.kind, s.payload) ?? signalSummary(s);
  const day = (iso: string) => new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
;
async () => {

 { svelteHTML.createElement("aside", {     "class":`evidence`,"aria-label":`Evidence`,...__sveltets_2_empty({"data-testid":`design-learned-evidence`}),});
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("span", { "class":`k`,});  }
    if(onclose){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onclose,"aria-label":`Close evidence`,"title":`Close`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
    }
   }
   { svelteHTML.createElement("h3", {});title; }
  if(rationale){ { svelteHTML.createElement("p", { "class":`why`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} rationale; }}
  if(items.length === 0){
     { svelteHTML.createElement("p", { "class":`dim`,});evidence.length ? 'These signals are older than the loaded log.' : 'No evidence was recorded with this rule.'; }
  }else{
     { svelteHTML.createElement("ol", { "class":`timeline`,});
         for(let s of __sveltets_2_ensureArray(items)){s.id;
        const hit = library.hitOf(s.artifact_id);
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("span", {   "class":`dot`,"aria-hidden":`true`,}); }
           { svelteHTML.createElement("div", { "class":`ev card`,});
             { svelteHTML.createElement("div", { "class":`row`,});
               { svelteHTML.createElement("strong", { "title":new Date(s.created_at).toLocaleString(),});day(s.created_at); }
               { svelteHTML.createElement("span", { "class":`kind tone-${signalTone(s.kind)}`,});signalKindLabel(s.kind); }
               { svelteHTML.createElement("span", { "class":`dim when`,});rel(s.created_at); }
             }
             { svelteHTML.createElement("div", { "class":`art`,});hit?.artifact.title ?? 'A design'; }
            if(hit){
               { svelteHTML.createElement("div", { "class":`pic`,});  { const $$_bmuhTtcafitrA5C = __sveltets_2_ensureComponent(ArtifactThumb); new $$_bmuhTtcafitrA5C({ target: __sveltets_2_any(), props: {    "artifact":hit.artifact,"versionId":s.version_id,"live":true,"compact":true,}});} }
            }
             { svelteHTML.createElement("p", { "class":`sum`,});summary(s); }
             { svelteHTML.createElement("a", { "href":`#/design/a/${encodeURIComponent(s.artifact_id)}`,});   }
           }
         }
      }
     }
  }
  if(missing > 0 && items.length > 0){
     { svelteHTML.createElement("p", { "class":`dim`,});plural(missing, 'more signal');     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const EvidencePane__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type EvidencePane__SvelteComponent_ = ReturnType<typeof EvidencePane__SvelteComponent_>;
/*Ωignore_endΩ*/export default EvidencePane__SvelteComponent_;