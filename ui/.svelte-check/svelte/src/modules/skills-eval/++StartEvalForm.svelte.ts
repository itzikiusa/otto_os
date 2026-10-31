///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
// The "new evaluation" form: pick a skill (library / provider / a path or
// archive), describe the task, choose the implementation CLI + iterations,
// add validation dimensions (each fanned across one or more agent CLIs), and
// pick the improver agent. Prefilled from the saved defaults (/settings/skill-eval).
import { auth } from '../../lib/stores/auth.svelte';
import { loadErrorText } from '../../lib/loadError';
import { ws } from '../../lib/stores/workspace.svelte';
import { skillsEvalApi } from '../../lib/api/skillsEval';
import type {
    SkillEvalValidationCfg,
    SkillSourceInfo,
    StartSkillEvalReq,
  } from '../../lib/api/types';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
;

  interface Props {
    starting: boolean;
    onstart: (req: StartSkillEvalReq) => void;
    /** Pre-select this skill ("Evaluate <skill>" from the Skills tab). `source`
     *  is "library" | "bundled" | a provider name. */
    initialSkill?: { name: string; source: string } | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { starting, onstart, initialSkill = null }: Props = $props();

  // The matching source row: the same copy when listed (a provider copy, or
  // the library), else any copy with that name. Bundled skills aren't eval
  // sources, so a bundled hand-off falls back to the library copy if present.
  function matchSource(list: SkillSourceInfo[], want: { name: string; source: string }): number {
    const isProvider = want.source !== 'library' && want.source !== 'bundled';
    let i = list.findIndex((s) =>
      s.name === want.name && (isProvider ? s.kind === 'provider' && s.provider === want.source : s.kind === 'library'),
    );
    if (i < 0) i = list.findIndex((s) => s.name === want.name);
    return i;
  }

  // Agent CLIs available for implementation / validation / improvement.
  const providerOpts = $derived(agentProviders());

  let sources: SkillSourceInfo[] = $state([]);
  // 'custom' = the path/archive branch; otherwise an index into `sources`.
  let sourceSel = $state<'custom' | number>('custom');
  let customPath = $state('');
  let showPicker = $state(false);

  let task = $state('');
  let implCli = $state(defaultAgentProvider());
  let iterations = $state(2);
  let validatorPasses = $state(1);
  let improverProvider = $state(defaultAgentProvider());
  let baseRef = $state('');
  // Repo-specific commands → scored as tests/lint signals + proof-pack evidence.
  let testCmd = $state('');
  let lintCmd = $state('');
  // Saved defaults (Settings → Skill eval). A blank field falls back to these
  // on the daemon, so the placeholder shows what will actually run.
  let testDefault = $state('');
  let lintDefault = $state('');
  let validations: SkillEvalValidationCfg[] = $state([]);
  // Advanced options (commands, passes, improver, base ref) sit behind a
  // disclosure: most runs only need the skill, the task and validations.
  let advancedOpen = $state(false);
  const advancedSummary = $derived.by(() => {
    const parts: string[] = [];
    const t = testCmd.trim() || testDefault;
    parts.push(t ? `Tests: ${t}` : 'No test command');
    parts.push(`${Math.max(1, Math.floor(validatorPasses))} validation pass${Math.floor(validatorPasses) === 1 ? '' : 'es'}`);
    parts.push(`improver ${improverProvider}`);
    if (baseRef.trim()) parts.push(`from ${baseRef.trim()}`);
    return parts.join(' · ');
  });

  let loaded = $state(false);
  // A failed defaults/sources load is shown inline with Retry: the form would
  // otherwise look ready with no skills and no validations.
  let loadError = $state<string | null>(null);
  // The improver model saved in Settings → Skills evaluator (sent with the run
  // so the setting isn't silently dropped).
  let improverModel = $state('');

  $effect(() => {
    if (!loaded) void load();
  });

  async function load(): Promise<void> {
    loadError = null;
    try {
      const wsId = ws.currentId;
      const [cfg, src] = await Promise.all([
        skillsEvalApi.getConfig(),
        wsId ? skillsEvalApi.listSources(wsId) : Promise.resolve({ sources: [] }),
      ]);
      validations = cfg.validations.map((v) => ({ ...v, providers: [...v.providers] }));
      iterations = cfg.iterations ?? 2;
      validatorPasses = cfg.validator_passes ?? 1;
      testDefault = cfg.default_test_cmd ?? '';
      lintDefault = cfg.default_lint_cmd ?? '';
      sources = src.sources;
      implCli = defaultAgentProvider();
      improverProvider = cfg.improver?.provider || implCli;
      improverModel = cfg.improver?.model ?? '';
      const want = initialSkill ? matchSource(sources, initialSkill) : -1;
      if (want >= 0) sourceSel = want;
      else if (sources.length > 0) sourceSel = 0;
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loaded = true;
    }
  }

  function addValidation(): void {
    validations = [
      ...validations,
      { name: '', criteria: '', providers: [implCli], model: '' },
    ];
  }
  function removeValidation(i: number): void {
    validations = validations.filter((_, idx) => idx !== i);
  }
  function toggleValProvider(i: number, p: string): void {
    const v = validations[i];
    const has = v.providers.includes(p);
    const next = has ? v.providers.filter((x) => x !== p) : [...v.providers, p];
    validations = validations.map((vv, idx) => (idx === i ? { ...vv, providers: next } : vv));
  }

  const canStart = $derived(
    !starting &&
      loaded &&
      task.trim().length > 0 &&
      implCli.length > 0 &&
      (sourceSel === 'custom' ? customPath.trim().length > 0 : sources.length > 0) &&
      validations.length > 0 &&
      validations.every((v) => v.name.trim() && v.criteria.trim()),
  );

  // Why Start is disabled, for its tooltip (a greyed button with no reason
  // leaves the user hunting through the form).
  const blockReason = $derived.by(() => {
    if (starting) return 'Starting…';
    if (!loaded) return 'Loading the evaluator defaults…';
    if (sourceSel === 'custom' ? !customPath.trim() : sources.length === 0) return 'Choose the skill under test';
    if (!task.trim()) return 'Describe the task to implement';
    if (validations.length === 0) return 'Add at least one validation';
    if (!validations.every((v) => v.name.trim() && v.criteria.trim())) return 'Every validation needs a name and criteria';
    return '';
  });

  // Rough agent-session count so the user sees the scope before launching.
  // Per iteration: 1 implementation + Σ(validation providers) × passes; plus
  // one improver between iterations (worst case: no early perfect-score exit).
  const estAgents = $derived.by(() => {
    const iters = Math.max(1, Math.floor(iterations));
    const passes = Math.max(1, Math.floor(validatorPasses));
    const valsPerIter = validations.reduce(
      (sum, v) => sum + (v.providers.length > 0 ? v.providers.length : 1) * passes,
      0,
    );
    return iters * (1 + valsPerIter) + Math.max(0, iters - 1);
  });

  function submit(): void {
    let source: StartSkillEvalReq['source'];
    if (sourceSel === 'custom') {
      source = { kind: 'path', reference: customPath.trim() };
    } else {
      const s = sources[sourceSel as number];
      source = { kind: s.kind, reference: s.name, provider: s.provider ?? null };
    }
    const req: StartSkillEvalReq = {
      source,
      task: task.trim(),
      impl_cli: implCli,
      iterations: Math.max(1, Math.floor(iterations)),
      validator_passes: Math.max(1, Math.min(3, Math.floor(validatorPasses))),
      validations: validations.map((v) => ({
        name: v.name.trim(),
        criteria: v.criteria.trim(),
        providers: v.providers.length > 0 ? v.providers : [implCli],
        model: v.model ?? '',
      })),
      improver: { provider: improverProvider, model: improverProvider === improverDefaultProvider ? improverModel : '' },
      base_ref: baseRef.trim() || null,
      test_cmd: testCmd.trim() || null,
      lint_cmd: lintCmd.trim() || null,
    };
    onstart(req);
  }

  // The saved improver model belongs to the saved improver provider; picking a
  // different agent here falls back to that agent's default model.
  let improverDefaultProvider = $state('');
  $effect(() => {
    if (loaded && !improverDefaultProvider) improverDefaultProvider = improverProvider;
  });

  function sourceLabel(s: SkillSourceInfo): string {
    const origin = s.kind === 'provider' ? s.provider : 'library';
    return `${s.name}  ·  ${origin}`;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`form-wrap`,});
   { svelteHTML.createElement("h2", {});   }
   { svelteHTML.createElement("p", { "class":`lede`,});
                    
                    
      
   }

  if(loadError){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`the evaluator defaults`,"variant":`compact`,"error":`${loadError} The skill list and saved validations stay missing until they load.`,"empty":true,"onretry":() => void load(),}});}
  }

  
   { svelteHTML.createElement("section", { "class":`card block`,});
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-source`,});   }
      if(!loaded && !loadError){
         { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`skills`,}});}
      }else{
         { svelteHTML.createElement("select", {       "id":`se-source`,"class":`input`,"bind:value":sourceSel,"disabled":!loaded,});/*Ωignore_startΩ*/() => sourceSel = __sveltets_2_any(null);/*Ωignore_endΩ*/
              for(let s of __sveltets_2_ensureArray(sources)){let i = 1;s.kind + s.name + (s.provider ?? '');
             { svelteHTML.createElement("option", { "value":i,});sourceLabel(s); }
          }
           { svelteHTML.createElement("option", { "value":`custom`,});         }
         }
      }

      if(sourceSel === 'custom'){
         { svelteHTML.createElement("div", { "class":`row`,});
           { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input grow`,"aria-label":`Skill path or archive`,"placeholder":`/path/to/skill-folder · SKILL.md · skill.zip`,"bind:value":customPath,});/*Ωignore_startΩ*/() => customPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => (showPicker = true),"type":`button`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});} 
           }
         }
      } else if (sources[sourceSel as number]?.description){
         { svelteHTML.createElement("p", {   "class":`hint clamp`,"title":sources[sourceSel as number].description,});sources[sourceSel as number].description; }
      }
     }

     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-task`,});   }
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`se-task`,"class":`input`,"rows":3,"placeholder":`e.g. Add a new endpoint that returns a player’s bonus balance history`,"bind:value":task,});/*Ωignore_startΩ*/() => task = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }

     { svelteHTML.createElement("div", { "class":`grid2`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-cli`,});  }
         { svelteHTML.createElement("select", {     "id":`se-cli`,"class":`input`,"bind:value":implCli,});/*Ωignore_startΩ*/() => implCli = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(providerOpts)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-iter`,});  }
         { svelteHTML.createElement("input", {            "id":`se-iter`,"class":`input`,"type":`number`,"min":`1`,"max":`10`,"bind:value":iterations,});/*Ωignore_startΩ*/() => iterations = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
     }
   }

  
   { svelteHTML.createElement("section", { "class":`card block`,});
     { svelteHTML.createElement("div", { "class":`block-head`,});
       { svelteHTML.createElement("span", { "class":`field-label`,});  }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":addValidation,"type":`button`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
       }
     }
    if(validations.length === 0){
       { svelteHTML.createElement("p", { "class":`hint`,});               }
    }
        for(let v of __sveltets_2_ensureArray(validations)){let i = 1;i;
       { svelteHTML.createElement("div", { "class":`val`,});
         { svelteHTML.createElement("div", { "class":`row`,});
           { svelteHTML.createElement("input", {          "dir":`auto`,"class":`input grow`,"placeholder":`logging`,"aria-label":`Validation ${i + 1} name`,"bind:value":v.name,});/*Ωignore_startΩ*/() => v.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => removeValidation(i),"type":`button`,"title":`Remove validation`,"aria-label":`Remove validation ${v.name.trim() || i + 1}`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
           }
         }
         { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`input`,"rows":2,"aria-label":`Validation ${i + 1} criteria`,"placeholder":`What to check and how to judge it (passed to the agent)`,"bind:value":v.criteria,});/*Ωignore_startΩ*/() => v.criteria = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("div", {     "class":`provider-chips`,"role":`group`,"aria-label":`Agents that run validation ${i + 1}`,});
             for(let p of __sveltets_2_ensureArray(providerOpts)){p;
             { svelteHTML.createElement("label", {  "class":`chip-toggle`,});v.providers.includes(p);
               { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":v.providers.includes(p),"onchange":() => toggleValProvider(i, p),});}
               { svelteHTML.createElement("span", { "class":`mono`,});p; }
             }
          }
         }
       }
    }
   }

  
   { svelteHTML.createElement("section", { "class":`card adv`,});
     { svelteHTML.createElement("button", {         "class":`adv-toggle`,"type":`button`,"aria-expanded":advancedOpen,"aria-controls":`se-advanced`,"onclick":() => (advancedOpen = !advancedOpen),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":advancedOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("span", { "class":`adv-sum dim`,});advancedSummary; }
     }
    if(advancedOpen){
       { svelteHTML.createElement("div", {   "class":`block adv-body`,"id":`se-advanced`,});
         { svelteHTML.createElement("div", { "class":`grid2`,});
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-test`,});   { svelteHTML.createElement("span", { "class":`hint-inline`,});       } }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`se-test`,"class":`input`,"bind:value":testCmd,"placeholder":testDefault ? `Default: ${testDefault}` : 'e.g. cargo test  /  npm test',...__sveltets_2_empty({"data-testid":`eval-test-cmd`}),});/*Ωignore_startΩ*/() => testCmd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-lint`,});   { svelteHTML.createElement("span", { "class":`hint-inline`,});  } }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`se-lint`,"class":`input`,"bind:value":lintCmd,"placeholder":lintDefault ? `Default: ${lintDefault}` : 'e.g. cargo clippy  /  npm run check',...__sveltets_2_empty({"data-testid":`eval-lint-cmd`}),});/*Ωignore_startΩ*/() => lintCmd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-passes`,});  }
             { svelteHTML.createElement("input", {            "id":`se-passes`,"class":`input`,"type":`number`,"min":`1`,"max":`3`,"bind:value":validatorPasses,});/*Ωignore_startΩ*/() => validatorPasses = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-imp`,});  }
             { svelteHTML.createElement("select", {     "id":`se-imp`,"class":`input`,"bind:value":improverProvider,});/*Ωignore_startΩ*/() => improverProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
                 for(let p of __sveltets_2_ensureArray(providerOpts)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
             }
           }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`se-base`,});   }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`se-base`,"class":`input`,"placeholder":`HEAD`,"bind:value":baseRef,});/*Ωignore_startΩ*/() => baseRef = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("p", { "class":`hint`,});
                          
                         { svelteHTML.createElement("span", { "class":`mono`,});  }
             
           }
         }
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`actions`,});
     { svelteHTML.createElement("span", {   "class":`cost`,"title":`Approximate — improver runs are skipped on a perfect score`,});
       plural(estAgents, 'agent session');
     }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":!canStart,"onclick":submit,"title":canStart ? undefined : blockReason,});
      starting ? 'Starting…' : 'Start evaluation';
     }
   }
 }

if(showPicker){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {           "title":`Choose a skill folder, SKILL.md, or archive`,"files":true,"start":customPath,"onpick":(p) => {
      customPath = p;
      showPicker = false;
    },"onclose":() => (showPicker = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StartEvalForm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StartEvalForm__SvelteComponent_ = ReturnType<typeof StartEvalForm__SvelteComponent_>;
/*Ωignore_endΩ*/export default StartEvalForm__SvelteComponent_;