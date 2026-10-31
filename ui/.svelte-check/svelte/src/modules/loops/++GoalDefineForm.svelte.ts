///<reference types="svelte" />
;
import { onDestroy, untrack } from 'svelte';
import { router } from '../../lib/router.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { toastError } from '../../lib/toastError';
import { ws } from '../../lib/stores/workspace.svelte';
import { loops } from '../../lib/stores/loops.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
import type { AcceptanceCriterion, GoalLoopDraft } from '../../lib/api/types';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import { confirmer } from '../../lib/confirm.svelte';

;type $$ComponentProps =  { oncancel: () => void; oncreated: (id: string) => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { oncancel, oncreated }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let seed = $state('');
  let mode = $state<'build' | 'research'>('build');
  let allowCommits = $state(false);
  let requireReview = $state(false);
  let sourceLinks = $state('');
  let selectedSkills = $state('');
  let repoPath = $state('');
  let picking = $state(false);
  let feedback = $state('');
  let defining = $state(false);
  let launching = $state(false);

  let draft = $state<GoalLoopDraft | null>(null);
  let name = $state('');
  // Budget (edited as friendlier units).
  let maxIterations = $state(5);
  let maxMinutes = $state(30);
  let perPhaseMinutes = $state(10);
  let executorCount = $state(1);
  // Executor agent: provider from /meta (shell can't take an agent prompt);
  // model is a free-text alias ("" = provider default).
  let execProvider = $state(defaultAgentProvider());
  let execModel = $state('');
  // The agent that DRAFTS the goal (the define call) — its own pick, so
  // changing it doesn't silently change who executes the loop.
  let defProvider = $state(defaultAgentProvider());
  let defModel = $state('');
  let showAdvanced = $state(false);
  const providers = $derived(
    agentProviders(),
  );

  let alive = true;
  let formGeneration = 0;
  let initialSnapshot = untrack(formState);
  onDestroy(() => { alive = false; formGeneration++; });
  $effect(() => router.guard(() => canLeaveForm()));

  function formState(): string {
    return JSON.stringify([seed, mode, allowCommits, requireReview, sourceLinks, selectedSkills, repoPath,
      feedback, draft, name, maxIterations, maxMinutes, perPhaseMinutes, executorCount,
      execProvider, execModel, defProvider, defModel]);
  }

  async function define(): Promise<void> {
    const wsId = ws.currentId;
    if (!alive || !wsId || !seed.trim() || (mode === 'build' && !repoPath.trim())) return;
    const generation = formGeneration;
    const ownsForm = () => alive && generation === formGeneration && ws.currentId === wsId;
    defining = true;
    try {
      const d = await loops.define(wsId, {
        seed, provider: defProvider, model: defModel, mode,
        repo_path: repoPath.trim(),
        context: draft ? JSON.stringify(draft.definition) : undefined,
        feedback: feedback.trim() || undefined,
      });
      if (!ownsForm()) return;
      draft = d;
      name = d.definition.title;
      maxIterations = d.suggested_limits.max_iterations;
      maxMinutes = Math.round(d.suggested_limits.max_runtime_secs / 60);
      perPhaseMinutes = Math.max(1, Math.round(d.suggested_limits.per_phase_timeout_secs / 60));
      feedback = '';
    } catch (e) {
      if (ownsForm()) toastError('Couldn’t draft the goal', e);
    } finally {
      if (ownsForm()) defining = false;
    }
  }

  function addCriterion(): void {
    if (!draft) return;
    // Generate a unique id (ids are the {#each} key; collisions after a remove
    // would mis-map bound inputs).
    const ids = new Set(draft.definition.acceptance_criteria.map((c) => c.id));
    let n = draft.definition.acceptance_criteria.length + 1;
    while (ids.has(`c${n}`)) n++;
    draft.definition.acceptance_criteria.push({
      id: `c${n}`,
      text: '',
      verify: '',
      verify_kind: 'agent',
      verify_cmd: null,
    });
  }
  function removeCriterion(i: number): void {
    if (!draft) return;
    draft.definition.acceptance_criteria.splice(i, 1);
  }

  function canLaunch(): boolean {
    if (!alive || !ws.currentId || !draft || !name.trim()) return false;
    const cs = draft.definition.acceptance_criteria;
    return (
      cs.length > 0 &&
      cs.every(
        (c) =>
          c.text.trim() !== '' &&
          c.verify.trim() !== '' &&
          (c.verify_kind !== 'command' || (c.verify_cmd ?? '').trim() !== ''),
      )
    );
  }

  async function launch(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !draft || !canLaunch()) return;
    if (launching) return;
    const generation = formGeneration;
    const ownsForm = () => alive && generation === formGeneration && ws.currentId === wsId;
    launching = true;
    try {
      const base = draft.suggested_config.executors[0] ?? {
        name: 'Executor',
        provider: defaultAgentProvider(),
        model: '',
        prompt_extra: '',
      };
      const executors = Array.from({ length: Math.max(1, executorCount) }, (_, i) => ({
        ...base,
        provider: execProvider,
        model: execModel.trim(),
        name: executorCount > 1 ? `${base.name} ${i + 1}` : base.name,
      }));
      const loop = await loops.create(wsId, {
        name: name.trim(),
        repo_path: repoPath.trim(),
        definition: draft.definition,
        limits: {
          max_iterations: maxIterations,
          max_runtime_secs: maxMinutes * 60,
          per_phase_timeout_secs: perPhaseMinutes * 60,
          max_cost_usd: null,
          max_attempts_per_executor: 3,
        },
        config: { ...draft.suggested_config, executors, mode, allow_commits: mode === 'build' && allowCommits,
          require_review: requireReview, source_links: sourceLinks.split('\n').map(s => s.trim()).filter(Boolean),
          skills: selectedSkills.split(',').map(s => s.trim()).filter(Boolean) },
        autostart: true,
      });
      if (ownsForm()) {
        initialSnapshot = formState();
        oncreated(loop.id);
      }
    } catch (e) {
      if (ownsForm()) toastError('Couldn’t launch the goal loop', e);
    } finally {
      if (ownsForm()) launching = false;
    }
  }

  /** Route/workspace navigation and local Cancel use the same complete draft. */
  async function canLeaveForm(): Promise<boolean> {
    if (!alive) return true;
    if (formState() !== initialSnapshot) {
      const ok = await confirmer.ask('Discard this goal loop? The draft and your edits are lost.', {
        title: 'Discard goal loop',
        danger: true, confirmLabel: 'Discard',
      });
      if (!ok) return false;
    }
    // A workspace switch keeps LoopsPage mounted. Settle its creation state,
    // rather than marking an A draft clean and leaving it launchable under B.
    // Invalidate ownership before the callback; repeated leave/Cancel callbacks
    // during teardown must neither close twice nor publish a pending launch.
    if (!alive) return true;
    alive = false;
    formGeneration++;
    defining = false;
    launching = false;
    oncancel();
    return true;
  }

  async function cancel(): Promise<void> {
    await canLeaveForm();
  }

  function setKind(c: AcceptanceCriterion, kind: AcceptanceCriterion['verify_kind']): void {
    c.verify_kind = kind;
    if (kind === 'command' && c.verify_cmd == null) c.verify_cmd = '';
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`form-page`,});
 { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { "title":`New goal loop`,leading:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Back to Goal Loops`,"aria-label":`Back to Goal Loops`,"onclick":cancel,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
 { svelteHTML.createElement("div", { "class":`form`,});
  if(!ws.currentId){
    
     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("p", {});             }
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (ui.newWorkspaceOpen = true),});  }
     }
  }

   { svelteHTML.createElement("section", { "class":`block`,});
     { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-seed`,});  { svelteHTML.createElement("span", { "class":`hint`,});    } }
     { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`gl-seed`,"class":`input in area`,"bind:value":seed,"rows":3,"placeholder":`e.g. Make the export endpoint stream instead of buffering, and add a test.`,});/*Ωignore_startΩ*/() => seed = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-mode`,});  }
     { svelteHTML.createElement("select", {     "id":`gl-mode`,"class":`input in`,"bind:value":mode,});/*Ωignore_startΩ*/() => mode = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`build`,});  } { svelteHTML.createElement("option", { "value":`research`,});  } }
     { svelteHTML.createElement("p", { "class":`muted small`,});              }
    if(mode === 'build'){
     { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-repo`,});  }
     { svelteHTML.createElement("div", { "class":`repo-row`,});
       { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`gl-repo`,"class":`input in grow`,"bind:value":repoPath,"placeholder":`/absolute/path/to/repo`,});/*Ωignore_startΩ*/() => repoPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {     "type":`button`,"class":`btn`,"onclick":() => (picking = true),});  }
     }
    }
     { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-define-provider`,});   { svelteHTML.createElement("span", { "class":`hint`,});        } }
     { svelteHTML.createElement("select", {       "id":`gl-define-provider`,"class":`input in`,"bind:value":defProvider,"onchange":() => (defModel = ''),});/*Ωignore_startΩ*/() => defProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/   for(let p of __sveltets_2_ensureArray(providers)){p; { svelteHTML.createElement("option", { "value":p,});p; }} }
     { const $$_rekciPledoM4C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM4C({ target: __sveltets_2_any(), props: {      "provider":defProvider,"value":defModel,"onchange":(m) => (defModel = m),}});}
     { svelteHTML.createElement("div", { "class":`frow`,});
      if(draft){
         { svelteHTML.createElement("input", {          "dir":`auto`,"aria-label":`Refine the draft`,"class":`input in grow`,"bind:value":feedback,"placeholder":`What to change about the draft`,});/*Ωignore_startΩ*/() => feedback = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {     "class":`btn`,"onclick":define,"disabled":!ws.currentId || defining || !feedback.trim() || !seed.trim() || (mode === 'build' && !repoPath.trim()),});
          defining ? 'Updating…' : 'Update draft';
         }
      }else{
         { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":define,"disabled":!ws.currentId || defining || !seed.trim() || (mode === 'build' && !repoPath.trim()),"title":!ws.currentId ? 'Add a workspace first' : !seed.trim() ? 'Describe the goal first' : mode === 'build' && !repoPath.trim() ? 'Choose the repository first' : 'Draft criteria and a budget from your goal',});
          defining ? 'Drafting…' : 'Draft goal';
         }
      }
     }
   }

  if(draft){
     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-name`,});  }
       { svelteHTML.createElement("input", {        "dir":`auto`,"id":`gl-name`,"class":`input in`,"bind:value":name,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(draft.definition.summary){
         { svelteHTML.createElement("p", { "class":`muted`,});draft.definition.summary; }
      }

       { svelteHTML.createElement("div", { "class":`lbl`,});   { svelteHTML.createElement("span", { "class":`hint`,});        } }
          for(let c of __sveltets_2_ensureArray(draft.definition.acceptance_criteria)){let i = 1;c.id;
         { svelteHTML.createElement("div", { "class":`crit`,});
           { svelteHTML.createElement("div", { "class":`crit-row`,});
             { svelteHTML.createElement("input", {          "dir":`auto`,"aria-label":`Criterion description`,"class":`input in grow`,"bind:value":c.text,"placeholder":`Criterion description`,});/*Ωignore_startΩ*/() => c.text = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => removeCriterion(i),"aria-label":`Remove criterion`,"title":`Remove criterion`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});} }
           }
           { svelteHTML.createElement("div", { "class":`crit-row`,});
             { svelteHTML.createElement("select", {       "aria-label":`Verification kind`,"class":`input in kind`,"value":c.verify_kind,"onchange":(e) => setKind(c, e.currentTarget.value as AcceptanceCriterion['verify_kind']),});
               { svelteHTML.createElement("option", { "value":`agent`,});  }
              if(c.verify_kind === 'manual'){ { svelteHTML.createElement("option", { "value":`manual`,});   }}
               { svelteHTML.createElement("option", { "value":`human`,});  }
               { svelteHTML.createElement("option", { "value":`command`,});  }
             }
            if(c.verify_kind === 'command'){
               { svelteHTML.createElement("input", {          "dir":`ltr`,"aria-label":`Verification command`,"class":`input in grow mono`,"bind:value":c.verify_cmd,"placeholder":`shell command (exit 0 = met), e.g. cargo test`,});/*Ωignore_startΩ*/() => c.verify_cmd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            }else{
               { svelteHTML.createElement("input", {          "dir":`auto`,"aria-label":`How to verify`,"class":`input in grow`,"bind:value":c.verify,"placeholder":`how to verify (behavior/file)`,});/*Ωignore_startΩ*/() => c.verify = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            }
           }
          if(c.verify_kind === 'command'){
             { svelteHTML.createElement("input", {          "dir":`auto`,"aria-label":`What this command checks`,"class":`input in grow`,"bind:value":c.verify,"placeholder":`what this checks (for humans)`,});/*Ωignore_startΩ*/() => c.verify = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          }
         }
      }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":addCriterion,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
      if(!canLaunch()){
        
         { svelteHTML.createElement("p", {   "class":`muted small`,"id":`goal-criteria-hint`,});          }
      }
     }

     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("div", { "class":`lbl`,});  }
       { svelteHTML.createElement("div", { "class":`budget`,});
         { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "class":`input in num`,"type":`number`,"min":`1`,"bind:value":maxIterations,});/*Ωignore_startΩ*/() => maxIterations = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "class":`input in num`,"type":`number`,"min":`1`,"bind:value":maxMinutes,});/*Ωignore_startΩ*/() => maxMinutes = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "class":`input in num`,"type":`number`,"min":`1`,"bind:value":perPhaseMinutes,});/*Ωignore_startΩ*/() => perPhaseMinutes = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {          "class":`input in num`,"type":`number`,"min":`1`,"max":`6`,"bind:value":executorCount,});/*Ωignore_startΩ*/() => executorCount = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       }
       { svelteHTML.createElement("div", { "class":`lbl`,});   { svelteHTML.createElement("span", { "class":`hint`,});     } }
       { svelteHTML.createElement("div", { "class":`budget`,});
         { svelteHTML.createElement("label", {});
           { svelteHTML.createElement("select", {     "class":`input in num prov`,"bind:value":execProvider,"onchange":() => (execModel = ''),});/*Ωignore_startΩ*/() => execProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let p of __sveltets_2_ensureArray(providers)){p;
               { svelteHTML.createElement("option", { "value":p,});p; }
            }
           }
         }
        
         { svelteHTML.createElement("div", { "class":`model-ctl`,});
           { const $$_rekciPledoM6C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM6C({ target: __sveltets_2_any(), props: {      "provider":execProvider,"value":execModel,"onchange":(m) => (execModel = m),}});}
         }
       }
       { svelteHTML.createElement("p", { "class":`muted small`,});
              mode === 'research' ? 'research directory' : 'git worktree';        
       }
     }

     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("label", {   "class":`checkbox-row`,"title":mode === 'research' ? 'Research loops don’t commit' : undefined,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":allowCommits,"disabled":mode === 'research',});/*Ωignore_startΩ*/() => allowCommits = __sveltets_2_any(null);/*Ωignore_endΩ*/}    }
       { svelteHTML.createElement("p", { "class":`muted small indent`,});                }
       { svelteHTML.createElement("label", { "class":`checkbox-row`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":requireReview,});/*Ωignore_startΩ*/() => requireReview = __sveltets_2_any(null);/*Ωignore_endΩ*/}      }
     }

     { svelteHTML.createElement("section", { "class":`block adv`,});
       { svelteHTML.createElement("button", {         "type":`button`,"class":`adv-toggle`,"aria-expanded":showAdvanced,"aria-controls":`gl-advanced`,"onclick":() => (showAdvanced = !showAdvanced),});
         { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":showAdvanced ? 'chevronDown' : 'chevronRight',"size":12,}});} 
         { svelteHTML.createElement("span", { "class":`hint adv-hint`,});        }
       }
      if(showAdvanced){
       { svelteHTML.createElement("div", { "id":`gl-advanced`,});
       { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-sources`,});        }
       { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`gl-sources`,"class":`input in area mono`,"rows":3,"bind:value":sourceLinks,"placeholder":`https://…`,});/*Ωignore_startΩ*/() => sourceLinks = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-skills`,});    }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`gl-skills`,"class":`input in`,"bind:value":selectedSkills,"placeholder":`e.g. db-mysql, golang-testing`,});/*Ωignore_startΩ*/() => selectedSkills = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        for(let role of __sveltets_2_ensureArray((['planner', 'evaluator', 'digester']))){
        const key = role as 'planner' | 'evaluator' | 'digester';
         { svelteHTML.createElement("label", {   "class":`lbl`,"for":`gl-${role}`,});role[0].toUpperCase() + role.slice(1);  }
         { svelteHTML.createElement("select", {       "id":`gl-${role}`,"class":`input in`,"bind:value":draft.suggested_config[key].provider,"onchange":() => { if (draft) draft.suggested_config[key].model = ''; },});/*Ωignore_startΩ*/() => draft.suggested_config[key].provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(providers)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
         }
         { const $$_rekciPledoM5C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM5C({ target: __sveltets_2_any(), props: {      "provider":draft.suggested_config[key].provider,"value":draft.suggested_config[key].model,"onchange":(m) => { if (draft) draft.suggested_config[key].model = m; },}});}
      }
       }
      }
     }
     { svelteHTML.createElement("div", { "class":`frow end`,});
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":cancel,});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":launch,"disabled":launching || !canLaunch(),"title":launching || canLaunch() ? undefined : 'Give the loop a name and at least one criterion, each with a description, how to verify it, and a shell command when verification runs a command',});
        launching ? 'Launching…' : 'Launch loop';
       }
     }
  }
 }
 PageBody}
 }

if(picking){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {        "title":`Choose a repository`,"gitOnly":true,"onpick":(p) => {
      repoPath = p;
      picking = false;
    },"onclose":() => (picking = false),}});}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GoalDefineForm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GoalDefineForm__SvelteComponent_ = ReturnType<typeof GoalDefineForm__SvelteComponent_>;
/*Ωignore_endΩ*/export default GoalDefineForm__SvelteComponent_;