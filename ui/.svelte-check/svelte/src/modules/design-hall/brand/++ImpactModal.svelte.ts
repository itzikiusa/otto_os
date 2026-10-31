///<reference types="svelte" />
;// The impact preview (proposal §3.3): before a token change is saved, show
// what changes, which designs see it and when they move (follow approved →
// on approve; follow latest → on save; pinned → never until updated), plus
// contrast warnings for the proposed colours. Nothing is saved from here
// unless the person presses "Save as vN".

import type { BrandConsumer, BrandImpactResp, BrandTokenChange } from '../../../lib/api/types';
import Badge from '../../../lib/components/Badge.svelte';
import type { BadgeTone } from '../../../lib/status';
import Modal from '../../../lib/components/Modal.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import StudioBadge from '../StudioBadge.svelte';
import { openArtifact } from '../nav';
import { studioInfo } from '../model';
import { normalizeHex } from './tokens';
;

  interface Props {
    impact: BrandImpactResp | null;
    loading: boolean;
    error: string | null;
    nextSeq: number;
    approvedSeq: number | null;
    canSave: boolean;
    saving: boolean;
    onclose: () => void;
    onsave: () => void;
    onretry: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { impact, loading, error, nextSeq, approvedSeq, canSave, saving, onclose, onsave, onretry }: Props = $props();

  const affected = $derived(impact?.consumers.filter((c) => c.affected.length > 0) ?? []);
  const untouched = $derived(impact ? impact.artifact_count - affected.length : 0);

  function when(c: BrandConsumer): { text: string; tone: BadgeTone } {
    if (c.policy === 'pinned') return { text: 'pinned · stays until updated', tone: 'warn' };
    if (c.policy === 'follow_latest') return { text: 'follows latest · changes on save', tone: 'info' };
    return { text: `follows approved · changes when you approve v${nextSeq}`, tone: 'accent' };
  }
  function isColor(ch: BrandTokenChange): boolean {
    return ch.token.startsWith('color.');
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Impact of your changes`,"width":640,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":onsave,"disabled":!canSave || saving,...__sveltets_2_empty({"data-testid":`brand-impact-save`}),});
      saving ? 'Saving…' : `Save as v${nextSeq}`;
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`imp`,...__sveltets_2_empty({"data-testid":`brand-impact-modal`}),});
    if((loading || error) && !impact){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         "what":`the impact preview`,"rows":4,loading,error,"empty":true,onretry,}});}
    } else if (impact){
       { svelteHTML.createElement("p", { "class":`lead`,});
        if(impact.changes.length === 0){     impact.base === 'approved' ? `the approved v${impact.base_seq}` : impact.base === 'head' ? `v${impact.base_seq}` : 'the empty kit'; }else{
          impact.changes.length;  impact.changes.length === 1 ? 'change' : 'changes';  
          impact.base === 'approved' ? `the approved v${impact.base_seq}` : impact.base === 'head' ? `v${impact.base_seq} (nothing approved yet)` : 'the empty kit';
          impact.changes.length === 1 ? 'reaches' : 'reach';  { svelteHTML.createElement("b", {});impact.affected_count; impact.affected_count === 1 ? 'design' : 'designs'; } 
           { svelteHTML.createElement("b", {});impact.affected_studio_count; impact.affected_studio_count === 1 ? 'studio' : 'studios'; } }
       }

      if(impact.changes.length){
         { svelteHTML.createElement("ul", {   "class":`changes`,"aria-label":`Token changes`,});
             for(let ch of __sveltets_2_ensureArray(impact.changes)){ch.token;
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("span", { "class":`mono tok`,});ch.token; }
               { svelteHTML.createElement("span", { "class":`kind ${ch.change}`,});ch.change; }
               { svelteHTML.createElement("span", { "class":`vals mono`,});
                if(isColor(ch) && ch.before && normalizeHex(ch.before)){ { svelteHTML.createElement("i", {  "class":`sw`,});__sveltets_2_ensureType(String, Number, ch.before); }}
                 { svelteHTML.createElement("span", { });!ch.before;ch.before ?? '—'; }
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
                if(isColor(ch) && ch.after && normalizeHex(ch.after)){ { svelteHTML.createElement("i", {  "class":`sw`,});__sveltets_2_ensureType(String, Number, ch.after); }}
                 { svelteHTML.createElement("span", { });!ch.after;ch.after ?? 'removed'; }
               }
             }
          }
         }
      }

      if(impact.affected_by_studio.length){
         { svelteHTML.createElement("div", { "class":`studios`,});
             for(let s of __sveltets_2_ensureArray(impact.affected_by_studio)){s.studio;
             { svelteHTML.createElement("span", { "class":`chip`,}); { const $$_egdaBoidutS4C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS4C({ target: __sveltets_2_any(), props: {  "studio":s.studio,}});} studioInfo(s.studio).name;  s.count; }
          }
         }
      }

      if(affected.length){
         { svelteHTML.createElement("ul", {   "class":`rows`,"aria-label":`Designs that change`,});
             for(let c of __sveltets_2_ensureArray(affected)){c.artifact.id;
            const w = when(c);
             { svelteHTML.createElement("li", {});
               { const $$_egdaBoidutS4C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS4C({ target: __sveltets_2_any(), props: {  "studio":c.artifact.studio,}});}
               { svelteHTML.createElement("button", {     "class":`linkish t`,"onclick":() => openArtifact(c.artifact.id),"title":`Open ${c.artifact.title}`,});c.artifact.title; }
               { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":w.tone,"label":w.text,}});}
               { svelteHTML.createElement("span", {   "class":`mono dim aff`,"title":c.affected.join(', '),});c.whole_kit ? 'whole kit' : c.affected.join(', '); }
             }
          }
         }
      }
      if(untouched > 0){
         { svelteHTML.createElement("p", { "class":`dim`,});untouched;  untouched === 1 ? 'design uses' : 'designs use';         }
      }
      if(impact.hidden_count > 0){
         { svelteHTML.createElement("p", { "class":`dim`,});impact.hidden_count;       }
      }

      if(impact.warnings.length){
         { svelteHTML.createElement("div", {   "class":`warns`,"role":`note`,});
              for(let w of __sveltets_2_ensureArray(impact.warnings)){let i = 1;i;
             { svelteHTML.createElement("p", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});} w; }
          }
         }
      }

       { svelteHTML.createElement("p", { "class":`note`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":13,}});}
          nextSeq;        approvedSeq ? `v${approvedSeq}` : 'the current tokens';    nextSeq;
       }
    }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ImpactModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ImpactModal__SvelteComponent_ = ReturnType<typeof ImpactModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default ImpactModal__SvelteComponent_;