///<reference types="svelte" />
;// Skills Lab — the umbrella page. Three sections (routes):
//   • Skills    `#/skills-eval`           — every skill grouped by name across
//                                            Library / Claude / Codex / Bundled,
//                                            with Overview · Edit · Evals · Usage
//   • Review    `#/skills-eval/review`    — multi-agent skill review
//   • Evaluator `#/skills-eval/evaluator` — the Skills Evaluator (Runs /
//                                            Golden Tasks / Matrix), unchanged

import { viewport } from '../../lib/stores/viewport.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { ws } from '../../lib/stores/workspace.svelte';
import { router } from '../../lib/router.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import SkillsBrowser from './SkillsBrowser.svelte';
import SkillReviewPanel from './SkillReviewPanel.svelte';
import SkillsEvalPage from '../skills-eval/SkillsEvalPage.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  type Tab = 'skills' | 'review' | 'evaluator';
  const tab = $derived<Tab>(router.parts[1] === 'review' ? 'review' : router.parts[1] === 'evaluator' ? 'evaluator' : 'skills');
  function go(t: Tab): void {
    if (t === tab) return;
    // Leaving Skills unmounts an open editor with unsaved edits: the editor's
    // router leave-guard (guardUnsaved) asks once — no second local prompt.
    router.go(t === 'skills' ? 'skills-eval' : `skills-eval/${t}`);
  }
  const TABS: { id: Tab; label: string }[] = [
    { id: 'skills', label: 'Skills' },
    { id: 'review', label: 'Review' },
    { id: 'evaluator', label: 'Evaluator' },
  ];
  // The Evaluator's own views ride in the SAME header tab row (after the
  // section tabs) instead of a second strip inside the body. Routed as
  // `#/skills-eval/evaluator[/golden|/matrix]`; SkillsEvalPage reads it.
  type EvalView = 'runs' | 'golden' | 'matrix';
  const EVAL_VIEWS: { id: EvalView; label: string; icon: 'zap' | 'target' | 'grid' }[] = [
    { id: 'runs', label: 'Runs', icon: 'zap' },
    { id: 'golden', label: 'Golden tasks', icon: 'target' },
    { id: 'matrix', label: 'Matrix', icon: 'grid' },
  ];
  const evalView = $derived<EvalView>(router.parts[2] === 'golden' ? 'golden' : router.parts[2] === 'matrix' ? 'matrix' : 'runs');
  function goEval(v: EvalView): void {
    if (v === evalView) return;
    router.go(v === 'runs' ? 'skills-eval/evaluator' : `skills-eval/evaluator/${v}`);
  }

  // Cross-tab intent: "Review this skill" from the Skills tab pre-fills the
  // Review form and switches tabs.
  let reviewTarget = $state<{ name: string; source: string } | null>(null);
  function reviewSkill(name: string, source: string): void {
    reviewTarget = { name, source };
    reviewOpen = null;
    go('review');
  }
  // "Reviews" on a skill's Usage tab: open that skill's latest review.
  let reviewOpen = $state<string | null>(null);
  function openReview(id: string): void {
    reviewTarget = null;
    reviewOpen = id;
    go('review');
  }
  // "Evaluate <skill>" opens the Evaluator's start form with that skill
  // pre-selected (it used to land on the form with whatever came first).
  let evalTarget = $state<{ name: string; source: string } | null>(null);
  function evaluateSkill(name: string, source: string): void {
    evalTarget = { name, source };
    go('evaluator');
  }

  // "Open run" from a skill's Evals tab: the Evaluator opens on that run.
  let runTarget = $state<string | null>(null);
  function openRun(id: string): void {
    runTarget = id;
    go('evaluator');
  }

  const wsId = $derived(ws.currentId ?? '');
  let browser = $state<ReturnType<typeof SkillsBrowser> | null>(null);
  let reviewPanel = $state<ReturnType<typeof SkillReviewPanel> | null>(null);
  let evalPage = $state<ReturnType<typeof SkillsEvalPage> | null>(null);
  let canCreate = $state(true);
  let skillsPhoneDetail = $state(false);
  let reviewPhoneDetail = $state(false);
  let evalPhoneDetail = $state(false);
  let listCollapsed = $state(false);
  // The Evaluator has a collapsible list on Runs and Matrix views.
  let evalListable = $state(true);

  const phoneBackLabel = $derived(
    tab === 'skills'
      ? 'Back to skills'
      : tab === 'review'
        ? 'Back to reviews'
        : evalView === 'matrix'
          ? 'Back to matrices'
          : 'Back to evaluations',
  );
  const showPhoneBack = $derived(
    viewport.isPhone &&
      ((tab === 'skills' && skillsPhoneDetail) ||
        (tab === 'review' && reviewPhoneDetail) ||
        (tab === 'evaluator' && evalPhoneDetail)),
  );
  function onPhoneBack(): void {
    if (tab === 'skills') void browser?.back();
    else if (tab === 'review') reviewPanel?.back();
    else evalPage?.back();
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`skills-lab`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":`Skills Lab`,"subtitle":tab === 'skills' ? 'Every skill your agents can load, in one place' : undefined,"tabsPlacement":tab === 'evaluator' ? 'below' : 'inline',leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if((tab === 'skills' || tab === 'review' || (tab === 'evaluator' && evalListable)) && !viewport.isPhone){
        const what = tab === 'skills' ? 'skills' : tab === 'review' ? 'reviews' : evalView === 'matrix' ? 'matrices' : 'evaluations';
        const targetId = tab === 'skills' ? 'skills-list-pane' : tab === 'review' ? 'reviews-list' : evalView === 'matrix' ? 'matrices-list' : 'evaluations-list';
         { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => (listCollapsed = !listCollapsed),"aria-label":listCollapsed ? `Show ${what} list` : `Hide ${what} list`,"title":listCollapsed ? `Show ${what} list` : `Hide ${what} list`,"aria-expanded":!listCollapsed,"aria-controls":targetId,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`sidebar`,"size":16,}});} }
      }
      if(showPhoneBack){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onPhoneBack,"aria-label":phoneBackLabel,"title":phoneBackLabel,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});} }
      }
    };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`lab-tabs`,});
         { svelteHTML.createElement("div", {           "class":`segmented`,"role":`tablist`,"aria-label":`Skills Lab section`,...__sveltets_2_empty({"data-testid":`lab-tabs`}),"tabindex":-1,"onkeydown":onTabKey,});
             for(let t of __sveltets_2_ensureArray(TABS)){t.id;
             { svelteHTML.createElement("button", {           "role":`tab`,"aria-selected":tab === t.id,"tabindex":tab === t.id ? 0 : -1,"onclick":() => go(t.id),...__sveltets_2_empty({"data-testid":`tab-${t.id}`}),});tab === t.id;t.label; }
          }
         }
        if(tab === 'evaluator'){
           { svelteHTML.createElement("span", {   "class":`lab-tabs-sep`,"aria-hidden":`true`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});} }
           { svelteHTML.createElement("div", {           "class":`segmented`,"role":`tablist`,"aria-label":`Evaluator view`,...__sveltets_2_empty({"data-testid":`eval-tabs`}),"tabindex":-1,"onkeydown":onTabKey,});
               for(let v of __sveltets_2_ensureArray(EVAL_VIEWS)){v.id;
               { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":evalView === v.id,"aria-controls":`eval-panel`,"tabindex":evalView === v.id ? 0 : -1,"onclick":() => goEval(v.id),...__sveltets_2_empty({"data-testid":`tab-${v.id}`}),});evalView === v.id;
                 { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":v.icon,"size":12,}});} v.label;
               }
            }
           }
        }
       }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(auth.can('settings', 'admin')){
        
         { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`gear`}),...__sveltets_2_empty({"data-label":tab === 'evaluator' ? 'Evaluator defaults' : 'Skill library settings'}),"onclick":() => router.go(tab === 'evaluator' ? 'settings/skill-eval' : 'settings/skills'),"aria-label":tab === 'evaluator' ? 'Evaluator defaults' : 'Skill library settings',"title":tab === 'evaluator' ? 'Evaluator defaults (Settings → Skills evaluator)' : 'Installed skills (Settings → Skills)',}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":14,}});} }
      }
      if(tab === 'skills'){
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`download`}),...__sveltets_2_empty({"data-label":`Import a skill (.zip)…`}),"onclick":() => browser?.openImport(),"title":`Import a skill package (.zip)`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
        if(canCreate){
           { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => browser?.openNew(),...__sveltets_2_empty({"data-testid":`new-skill`}),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
        }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
    
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
   { svelteHTML.createElement("div", { "class":`lab-body`,});
    if(tab === 'skills'){
       { const $$_resworBsllikS3C = __sveltets_2_ensureComponent(SkillsBrowser); const $$_resworBsllikS3 = new $$_resworBsllikS3C({ target: __sveltets_2_any(), props: {              listCollapsed,"onreview":reviewSkill,"onevaluate":evaluateSkill,"onopenrun":openRun,"onempty":(empty) => (canCreate = !empty),"onphonedetail":(o) => (skillsPhoneDetail = o),"onopenreview":openReview,}});browser = $$_resworBsllikS3;}
    } else if (tab === 'review'){
       { const $$_lenaPweiveRllikS3C = __sveltets_2_ensureComponent(SkillReviewPanel); const $$_lenaPweiveRllikS3 = new $$_lenaPweiveRllikS3C({ target: __sveltets_2_any(), props: {              wsId,listCollapsed,"initialTarget":reviewTarget,"onconsumed":() => (reviewTarget = null),"initialReview":reviewOpen,"onreviewconsumed":() => (reviewOpen = null),"onphonedetail":(o) => (reviewPhoneDetail = o),}});reviewPanel = $$_lenaPweiveRllikS3;}
    }else{
       { const $$_egaPlavEsllikS3C = __sveltets_2_ensureComponent(SkillsEvalPage); const $$_egaPlavEsllikS3 = new $$_egaPlavEsllikS3C({ target: __sveltets_2_any(), props: {               "initialSkill":evalTarget,"onconsumed":() => (evalTarget = null),"initialRun":runTarget,"onrunconsumed":() => (runTarget = null),listCollapsed,"onlistable":(v) => (evalListable = v),"onphonedetail":(o) => (evalPhoneDetail = o),}});evalPage = $$_egaPlavEsllikS3;}
    }
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillsLabPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillsLabPage__SvelteComponent_ = ReturnType<typeof SkillsLabPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillsLabPage__SvelteComponent_;