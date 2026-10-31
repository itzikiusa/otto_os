///<reference types="svelte" />
;
import PageHeader from '../../lib/components/PageHeader.svelte';
import { toastError } from '../../lib/toastError';
import { sectionLabel } from './sections';
import { guardUnsaved } from '../../lib/leaveGuard';
import SectionIntro from './SectionIntro.svelte';
import { router } from '../../lib/router.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Root-only defaults for the Skills Evaluator: the validations, improver
// agent, iterations, and validation passes pre-filled into the start form.
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import { skillsEvalApi } from '../../lib/api/skillsEval';
import type { SkillEvalConfig, SkillEvalValidationCfg } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const providerOpts = $derived(agentProviders());

  let cfg: SkillEvalConfig | null = $state(null);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  // Last loaded/saved form, so Save stays disabled until something changed.
  let savedKey = $state('');
  function formKey(c: SkillEvalConfig): string {
    return JSON.stringify([c.iterations, c.validator_passes, c.improver.provider, c.validations]);
  }
  const dirty = $derived(!!cfg && formKey(cfg) !== savedKey);
  // Leaving Settings (or this page) with unsaved edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: 'the evaluator defaults' }));
  // Inline validation (not a toast): every validation needs a name, criteria
  // and at least one CLI, and the numbers must be in range.
  const formError = $derived.by(() => {
    if (!cfg) return '';
    const whole = (n: unknown, lo: number, hi: number) => typeof n === 'number' && Number.isInteger(n) && n >= lo && n <= hi;
    if (!whole(cfg.iterations, 1, 10)) return 'Iterations must be a whole number from 1 to 10.';
    if (!whole(cfg.validator_passes, 1, 3)) return 'Validation passes must be a whole number from 1 to 3.';
    const i = cfg.validations.findIndex((v) => !v.name.trim() || !v.criteria.trim());
    if (i >= 0) return `Validation ${i + 1} needs a name and criteria.`;
    const j = cfg.validations.findIndex((v) => v.providers.length === 0);
    if (j >= 0) return `Validation “${cfg.validations[j].name || j + 1}” needs at least one agent CLI.`;
    return '';
  });

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      cfg = await skillsEvalApi.getConfig();
      savedKey = formKey(cfg);
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function addValidation(): void {
    if (!cfg) return;
    cfg.validations = [
      ...cfg.validations,
      { name: '', criteria: '', providers: [defaultAgentProvider()], model: '' },
    ];
  }
  function removeValidation(i: number): void {
    if (!cfg) return;
    cfg.validations = cfg.validations.filter((_, idx) => idx !== i);
  }
  function toggleProvider(i: number, p: string): void {
    if (!cfg) return;
    const v = cfg.validations[i];
    const next = v.providers.includes(p)
      ? v.providers.filter((x) => x !== p)
      : [...v.providers, p];
    cfg.validations = cfg.validations.map((vv, idx) => (idx === i ? { ...vv, providers: next } : vv));
  }

  async function save(): Promise<void> {
    if (!cfg || saving || formError || !dirty) return;
    saving = true;
    try {
      // Spread the loaded config first: PUT stores the body as-is and serde
      // fills anything missing with its default, so sending only the fields
      // on this page silently reset weights, promote_min_score,
      // require_proof_pass and the default test/lint commands on every save.
      const body: SkillEvalConfig = {
        ...$state.snapshot(cfg),
        validations: cfg.validations.map((v: SkillEvalValidationCfg) => ({
          name: v.name.trim(),
          criteria: v.criteria.trim(),
          providers: v.providers,
          model: v.model ?? '',
        })),
        improver: { provider: cfg.improver.provider, model: cfg.improver.model ?? '' },
        iterations: Math.max(1, Math.min(10, Math.floor(cfg.iterations))),
        validator_passes: Math.max(1, Math.min(3, Math.floor(cfg.validator_passes))),
      };
      cfg = await skillsEvalApi.putConfig(body);
      savedKey = formKey(cfg);
      toasts.success('Skills evaluator defaults saved', 'New runs start from these.');
    } catch (e) {
      toastError('Couldn’t save the evaluator defaults', e);
    } finally {
      saving = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":sectionLabel('skill-eval'),"subtitle":`Defaults pre-filled into the start form`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(cfg){
         { svelteHTML.createElement("button", {         "class":`btn small primary`,"disabled":saving || !dirty || !!formError,"title":formError || (dirty ? 'Save these defaults' : 'No unsaved changes'),"onclick":save,});
          saving ? 'Saving…' : 'Save';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                       { svelteHTML.createElement("button", {     "type":`button`,"class":`intro-link`,"onclick":() => router.go('skills-eval/evaluator'),});    }            SectionIntro}
   { svelteHTML.createElement("div", { "class":`eval-body`,});

  if(!cfg){
     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {         "what":`Skills evaluator defaults`,loading,"error":loadError,"empty":true,"onretry":() => void load(),"rows":3,}});}
  }else{
    if(formError){
       { svelteHTML.createElement("p", {   "class":`form-note error`,"role":`alert`,});formError; }
    } else if (dirty){
       { svelteHTML.createElement("p", { "class":`form-note unsaved`,});       }
    }
     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { svelteHTML.createElement("section", { "class":`card row3`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`sv-iter`,});  }
         { svelteHTML.createElement("input", {            "id":`sv-iter`,"class":`input`,"type":`number`,"min":`1`,"max":`10`,"bind:value":cfg.iterations,});/*Ωignore_startΩ*/() => cfg.iterations = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`hint`,});  }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`sv-passes`,});  }
         { svelteHTML.createElement("input", {            "id":`sv-passes`,"class":`input`,"type":`number`,"min":`1`,"max":`3`,"bind:value":cfg.validator_passes,});/*Ωignore_startΩ*/() => cfg.validator_passes = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`hint`,});  }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`sv-imp`,});  }
         { svelteHTML.createElement("select", {     "id":`sv-imp`,"class":`input`,"bind:value":cfg.improver.provider,});/*Ωignore_startΩ*/() => cfg.improver.provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(providerOpts)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
         }
         { svelteHTML.createElement("span", { "class":`hint`,});      }
       }
     }

     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("div", { "class":`block-head`,});
         { svelteHTML.createElement("h2", { "class":`section-title`,});   { svelteHTML.createElement("span", { "class":`count`,});cfg.validations.length; } }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {     "class":`btn small`,...__sveltets_2_empty({"data-icon":`plus`}),"onclick":addValidation,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
       }
          for(let v of __sveltets_2_ensureArray(cfg.validations)){let i = 1;i;
         { svelteHTML.createElement("div", { "class":`val card`,});
           { svelteHTML.createElement("div", { "class":`val-row`,});
             { svelteHTML.createElement("div", { "class":`field grow`,});
               { svelteHTML.createElement("label", { "for":`sv-name-${i}`,});  }
               { svelteHTML.createElement("input", {              "dir":`auto`,"id":`sv-name-${i}`,"class":`input`,"placeholder":`logging`,"spellcheck":`false`,"autocomplete":`off`,"bind:value":v.name,});/*Ωignore_startΩ*/() => v.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
             { svelteHTML.createElement("button", {         "class":`icon-btn danger-icon val-remove`,"onclick":() => removeValidation(i),"aria-label":`Remove validation ${v.name || i + 1}`,"title":`Remove validation ${v.name || i + 1}`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
             }
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", { "for":`sv-crit-${i}`,});  }
             { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`sv-crit-${i}`,"class":`input val-textarea`,"rows":2,"placeholder":`Logs use the skill’s conventions and never leak secrets.`,"bind:value":v.criteria,});/*Ωignore_startΩ*/() => v.criteria = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           }
           { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("span", {   "class":`lbl`,"id":`sv-clis-${i}`,});  }
           { svelteHTML.createElement("div", {     "class":`chips`,"role":`group`,"aria-labelledby":`sv-clis-${i}`,});
               for(let p of __sveltets_2_ensureArray(providerOpts)){p;
               { svelteHTML.createElement("button", {          "type":`button`,"class":`pill-toggle`,"aria-pressed":v.providers.includes(p),"onclick":() => toggleProvider(i, p),});v.providers.includes(p);
                if(v.providers.includes(p)){ { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} { svelteHTML.createElement("span", { "class":`mono`,});p; }
               }
            }
           }
           }
         }
      }
         { svelteHTML.createElement("div", { "class":`empty`,});
                        
         }
      
     }
  }
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillEvalSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillEvalSettings__SvelteComponent_ = ReturnType<typeof SkillEvalSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillEvalSettings__SvelteComponent_;