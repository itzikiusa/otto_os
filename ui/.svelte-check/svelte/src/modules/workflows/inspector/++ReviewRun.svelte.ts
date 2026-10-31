///<reference types="svelte" />
;// review_run: repository / base (inherited by default), threshold, execution
// mode, and the rich reviewer config — per-lens reviewers (each its own
// providers + instructions), a summarizer and a per-severity scoring guideline.

import type { WorkflowNode } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
import RepoPicker from './RepoPicker.svelte';
import type { InspectorOptions } from './options.svelte';
import {
    linesValue, paramBool, paramLines, paramNum, paramStr, reviewers as reviewersOf, scoringField as scoringOf,
    summarizerField as summarizerOf, toggled, withScoring, withSummarizer, type ReviewerRow, type SetParam,
  } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  
  
  
  

  let { node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  // Reviewer providers come from the live registry (built-ins + custom).
  const REVIEW_PROVIDERS = $derived(agentProviders());
  function reviewers(): ReviewerRow[] {
    return reviewersOf(node);
  }
  function setReviewers(rows: ReviewerRow[]): void {
    set('reviewers', rows.length ? rows : undefined);
  }
  function addReviewer(): void {
    setReviewers([...reviewers(), { lens: '', providers: [defaultAgentProvider()] }]);
  }
  function removeReviewer(i: number): void {
    setReviewers(reviewers().filter((_, idx) => idx !== i));
  }
  function updateReviewer(i: number, patch: Partial<ReviewerRow>): void {
    setReviewers(reviewers().map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  }
  function toggleReviewerProvider(i: number, prov: string): void {
    const next = toggled(reviewers()[i]?.providers ?? [], prov);
    updateReviewer(i, { providers: next.length ? next : [defaultAgentProvider()] });
  }
  const summarizerField = (field: 'provider' | 'instructions'): string => summarizerOf(node, field);
  const updateSummarizer = (field: string, value: string): void => set('summarizer', withSummarizer(node, field, value));
  const scoringField = (sev: 'bug' | 'warn' | 'info', def: number): number => scoringOf(node, sev, def);
  const updateScoring = (sev: string, value: number): void => set('scoring', withScoring(node, sev, value));
;
async () => {

 { svelteHTML.createElement("p", { "class":`insp-note`,});
              
            
 }
 { svelteHTML.createElement("label", { "for":`np-repo`,});       }
 { const $$_rekciPopeR0C = __sveltets_2_ensureComponent(RepoPicker); new $$_rekciPopeR0C({ target: __sveltets_2_any(), props: {        "id":`np-repo`,"emptyLabel":`Inherit from the working folder`,node,set,wsId,pickers,}});}
 { svelteHTML.createElement("label", { "for":`np-base`,});       }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-base`,"type":`text`,"placeholder":`inherits from the run, else main`,"value":paramStr(node, 'base'),"oninput":(e) => set('base', e.currentTarget.value),});}
 { svelteHTML.createElement("label", { "for":`np-threshold`,});   }
 { svelteHTML.createElement("input", {             "id":`np-threshold`,"type":`number`,"min":`0`,"max":`100`,"value":paramNum(node, 'threshold', 80),"oninput":(e) => set('threshold', Number(e.currentTarget.value)),});}

 { svelteHTML.createElement("label", { "for":`np-review-mode`,});  }
  { svelteHTML.createElement("select", {        "id":`np-review-mode`,...__sveltets_2_empty({"data-testid":`review-mode-select`}),"value":paramStr(node, 'mode'),"onchange":(e) => set('mode', e.currentTarget.value || undefined),});
   { svelteHTML.createElement("option", {"value":"",});       }
   { svelteHTML.createElement("option", { "value":`fan_out`,});         }
   { svelteHTML.createElement("option", { "value":`orchestrator`,});             }
 }
 { svelteHTML.createElement("p", { "class":`insp-note`,});
             
               
              
   
 }
 { svelteHTML.createElement("div", { "class":`rv-h np-sec`,});
   { svelteHTML.createElement("span", { "class":`np-label`,});            }
   { svelteHTML.createElement("button", {     "class":`btn small ghost`,"type":`button`,"onclick":addReviewer,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} 
   }
 }
if(reviewers().length === 0){
   { svelteHTML.createElement("p", { "class":`insp-note`,});
                 
      { svelteHTML.createElement("code", {});  }    
   }
}
 { svelteHTML.createElement("div", { "class":`rv-list`,});
    for(let r of __sveltets_2_ensureArray(reviewers())){let i = 1;i;
   { svelteHTML.createElement("div", { "class":`rv-row`,});
     { svelteHTML.createElement("div", { "class":`rv-top`,});
       { svelteHTML.createElement("input", {              "dir":`auto`,"aria-label":`Reviewer lens / skill`,"class":`rv-lens`,"type":`text`,"placeholder":`lens / skill (e.g. correctness-review)`,"value":r.lens ?? '',"oninput":(e) => updateReviewer(i, { lens: e.currentTarget.value }),});}
       { svelteHTML.createElement("button", {         "class":`rv-del`,"type":`button`,"title":`Remove reviewer`,"aria-label":`Remove reviewer`,"onclick":() => removeReviewer(i),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
       }
     }
     { svelteHTML.createElement("div", { "class":`rv-provs`,});
         for(let prov of __sveltets_2_ensureArray(REVIEW_PROVIDERS)){prov;
         { svelteHTML.createElement("label", {  "class":`rv-chip`,});(r.providers ?? []).includes(prov);
           { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":(r.providers ?? []).includes(prov),"onchange":() => toggleReviewerProvider(i, prov),});}
          prov;
         }
      }
     }
     { svelteHTML.createElement("textarea", {              "dir":`auto`,"aria-label":`Reviewer instructions`,"class":`rv-instr np-prompt`,"rows":5,"placeholder":`custom instructions for this reviewer (optional)`,"value":r.instructions ?? '',"oninput":(e) => updateReviewer(i, { instructions: e.currentTarget.value }),}); }
   }
}
 }

 { svelteHTML.createElement("label", {   "class":`np-sec`,"for":`np-sum-prov`,});    }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-sum-prov`,"type":`text`,"placeholder":`provider (e.g. claude)`,"value":summarizerField('provider'),"oninput":(e) => updateSummarizer('provider', e.currentTarget.value),});}
  { svelteHTML.createElement("textarea", {             "dir":`auto`,"aria-label":`Summarizer instructions`,"class":`np-prompt np-prompt-lg`,"rows":14,"placeholder":`summarizer instructions (optional)`,"value":summarizerField('instructions'),"oninput":(e) => updateSummarizer('instructions', e.currentTarget.value),}); }

 { svelteHTML.createElement("span", { "class":`np-label np-sec`,});        }
 { svelteHTML.createElement("div", { "class":`rv-score`,});
   { svelteHTML.createElement("label", { "class":`rv-sc`,});
    
     { svelteHTML.createElement("input", {          "type":`number`,"min":`0`,"max":`100`,"value":scoringField('bug', 20),"oninput":(e) => updateScoring('bug', Number(e.currentTarget.value)),});}
   }
   { svelteHTML.createElement("label", { "class":`rv-sc`,});
    
     { svelteHTML.createElement("input", {          "type":`number`,"min":`0`,"max":`100`,"value":scoringField('warn', 5),"oninput":(e) => updateScoring('warn', Number(e.currentTarget.value)),});}
   }
   { svelteHTML.createElement("label", { "class":`rv-sc`,});
    
     { svelteHTML.createElement("input", {          "type":`number`,"min":`0`,"max":`100`,"value":scoringField('info', 5),"oninput":(e) => updateScoring('info', Number(e.currentTarget.value)),});}
   }
 }
 { svelteHTML.createElement("label", {   "class":`np-sec`,"for":`np-goals`,});     }
  { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`np-goals`,"class":`np-prompt`,"rows":7,"placeholder":'No N+1 queries\nAll inputs validated',"value":paramLines(node, 'goals'),"oninput":(e) => set('goals', linesValue(e.currentTarget.value)),}); }
 { svelteHTML.createElement("label", {   "class":`np-sec`,"for":`np-checks`,});          }
  { svelteHTML.createElement("textarea", {             "dir":`ltr`,"id":`np-checks`,"class":`np-prompt`,"rows":7,"placeholder":'go test -tags=component ./...\ngo test -tags=integration ./...',"value":paramLines(node, 'checks'),"oninput":(e) => set('checks', linesValue(e.currentTarget.value)),}); }
 { svelteHTML.createElement("p", { "class":`insp-note`,});
                
              
 }
 { svelteHTML.createElement("label", { "class":`np-chk`,});
   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'await', true),"onchange":(e) => set('await', e.currentTarget.checked),});}      
 }
 { svelteHTML.createElement("label", { "class":`np-chk`,});
   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'require_pass'),"onchange":(e) => set('require_pass', e.currentTarget.checked),});}        
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReviewRun__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReviewRun__SvelteComponent_ = ReturnType<typeof ReviewRun__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReviewRun__SvelteComponent_;