///<reference types="svelte" />
;// Autonomy tab: permission modes (Proactive / Directed / Scheduled), standing
// goals + daily budget, custom rules, and "your agent" (primary). One form,
// one Save — the daemon derives which rules it can enforce.

import { tick, untrack } from 'svelte';
import { personalAgentsApi } from '../../lib/api/personalAgents';
import { personalAgents } from '../../lib/stores/personalAgents.svelte';
import { router } from '../../lib/router.svelte';
import { guardUnsaved } from '../../lib/leaveGuard';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import type { PersonalAgentAutonomy, PersonalAgentRule } from '../../lib/api/types';
;

  interface Props {
    agentId: string;
    editable: boolean;
    /** May change the OWNER fields — goal texts and rules (PC-3). Defaults to
     *  `editable`; toggles, budget, primary and "Work on it now" stay `editable`. */
    ownerEditable?: boolean;
    /** Shown to an Editor who isn't the owner, under the owner-only sections. */
    ownerNote?: string;
    /** Told after a successful save (the page header shows mode badges). */
    onsaved?: (a: PersonalAgentAutonomy) => void;
  };;

  interface GoalDraft { id?: string; text: string; enabled: boolean; last_run_at: string | null };;
  interface RuleDraft { id?: string; text: string; enforce: PersonalAgentRule['enforce'] };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { agentId, editable, ownerEditable = undefined, ownerNote = '', onsaved }: Props = $props();
  const ownerEdit = $derived(editable && (ownerEditable ?? true));
  /** Prefix for the label↔control ids. */
  const uid = $props.id();



  let loading = $state(true);
  let goalsList = $state<HTMLElement>();
  let rulesList = $state<HTMLElement>();
  let addGoalBtn = $state<HTMLButtonElement>();
  let addRuleBtn = $state<HTMLButtonElement>();

  /** Add an empty goal/rule row and put the caret in it. */
  async function addRow(kind: 'goal' | 'rule'): Promise<void> {
    if (kind === 'goal') goals = [...goals, { text: '', enabled: true, last_run_at: null }];
    else rules = [...rules, { text: '', enforce: null }];
    await tick();
    const inputs = (kind === 'goal' ? goalsList : rulesList)?.querySelectorAll<HTMLInputElement>('input.grow');
    inputs?.[inputs.length - 1]?.focus();
  }

  /** Remove row `i`, then keep focus in the list (the keyed row holding the
   *  focused button is destroyed): the same index's remove button, else the
   *  previous one, else the list's Add button. */
  async function removeRow(kind: 'goal' | 'rule', i: number): Promise<void> {
    if (kind === 'goal') goals = goals.filter((_, j) => j !== i);
    else rules = rules.filter((_, j) => j !== i);
    await tick();
    const btns = (kind === 'goal' ? goalsList : rulesList)?.querySelectorAll<HTMLButtonElement>('button.rm-row');
    (btns?.[i] ?? btns?.[i - 1] ?? (kind === 'goal' ? addGoalBtn : addRuleBtn))?.focus();
  }
  let loadError = $state<string | null>(null);
  let saving = $state(false);
  let saved = $state<PersonalAgentAutonomy | null>(null);

  let proactiveOn = $state(false);
  let runsPerDay = $state(4);
  let maxMinutes = $state(15);
  let goals = $state<GoalDraft[]>([]);
  let rules = $state<RuleDraft[]>([]);
  let primary = $state(false);

  const schedules = $derived(personalAgents.schedulesByAgent[agentId] ?? []);
  const readOnlySchedules = $derived(schedules.filter((s) => s.permission === 'read_only').length);

  function apply(a: PersonalAgentAutonomy): void {
    saved = a;
    proactiveOn = a.proactive.enabled;
    runsPerDay = a.proactive.runs_per_day;
    maxMinutes = a.proactive.max_minutes;
    goals = a.goals.map((g) => ({ ...g }));
    rules = a.rules.map((r) => ({ ...r }));
    primary = a.primary;
  }

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      apply(await personalAgentsApi.autonomy(agentId));
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void agentId;
    untrack(() => void load());
  });

  const dirty = $derived.by(() => {
    if (!saved) return false;
    const cur = JSON.stringify({
      p: [proactiveOn, Number(runsPerDay), Number(maxMinutes)],
      g: goals.map((g) => [g.id ?? '', g.text.trim(), g.enabled]).filter((g) => g[1] !== ''),
      r: rules.map((r) => [r.id ?? '', r.text.trim()]).filter((r) => r[1] !== ''),
      primary,
    });
    const was = JSON.stringify({
      p: [saved.proactive.enabled, saved.proactive.runs_per_day, saved.proactive.max_minutes],
      g: saved.goals.map((g) => [g.id, g.text, g.enabled]),
      r: saved.rules.map((r) => [r.id, r.text]),
      primary: saved.primary,
    });
    return cur !== was;
  });
  $effect(() => router.guard(() => !saving));
  $effect(() => guardUnsaved(() => dirty, { what: 'this agent’s autonomy settings' }));

  async function save(): Promise<void> {
    if (saving) return;
    saving = true;
    try {
      const next = await personalAgentsApi.saveAutonomy(agentId, {
        proactive: {
          enabled: proactiveOn,
          runs_per_day: Math.min(24, Math.max(1, Math.round(Number(runsPerDay) || 1))),
          max_minutes: Math.min(60, Math.max(1, Math.round(Number(maxMinutes) || 1))),
        },
        goals: goals.map((g) => ({ id: g.id, text: g.text, enabled: g.enabled })),
        rules: rules.map((r) => ({ id: r.id, text: r.text })),
        primary,
      });
      apply(next);
      onsaved?.(next);
      toasts.success('Saved');
    } catch (e) {
      toasts.error('Couldn’t save the agent’s autonomy settings', loadErrorText(e));
    } finally {
      saving = false;
    }
  }

  let workingGoal = $state<string | null>(null);
  async function workNow(goalId: string): Promise<void> {
    workingGoal = goalId;
    try {
      await personalAgentsApi.runGoal(agentId, goalId);
      toasts.info('Working on the goal — read-only. Findings land in Runs.');
      router.go(`personal-agents/${agentId}/activity`);
    } catch (e) {
      toasts.error('Couldn’t start the goal run', loadErrorText(e));
    } finally {
      workingGoal = null;
    }
  }

  function enforceLabel(r: RuleDraft): string {
    if (!r.enforce) return 'Instructions only';
    const terms = r.enforce.terms.map((t) => `“${t}”`).join(', ');
    return r.enforce.kind === 'deny'
      ? `Enforced: blocks actions mentioning ${terms}`
      : `Enforced: asks you before actions mentioning ${terms}`;
  }
;
async () => {

 { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`this agent’s autonomy settings`,loading,"error":loadError,"rows":4,"onretry":() => void load(),}});
   { svelteHTML.createElement("div", { "class":`autonomy`,});
     { svelteHTML.createElement("section", {   "class":`pa-panel`,"aria-labelledby":`modes-h`,});
       { svelteHTML.createElement("h2", { "id":`modes-h`,});  }
       { svelteHTML.createElement("p", { "class":`hint`,});                   }
       { svelteHTML.createElement("div", { "class":`modes`,});
         { svelteHTML.createElement("div", {  "class":`mode`,});proactiveOn;
           { svelteHTML.createElement("div", { "class":`mode-head`,});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":14,}});}
             { svelteHTML.createElement("strong", {});  }
             { svelteHTML.createElement("span", { "class":`chip`,});  }
           }
           { svelteHTML.createElement("p", { "class":`mode-body`,});                        }
           { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":proactiveOn,"disabled":!editable || saving,});/*Ωignore_startΩ*/() => proactiveOn = __sveltets_2_any(null);/*Ωignore_endΩ*/}        }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`${uid}-runs`,});    }
               { svelteHTML.createElement("input", {              "id":`${uid}-runs`,"class":`input`,"type":`number`,"min":`1`,"max":`24`,"bind:value":runsPerDay,"disabled":!editable || saving,});/*Ωignore_startΩ*/() => runsPerDay = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`${uid}-minutes`,});    }
               { svelteHTML.createElement("input", {              "id":`${uid}-minutes`,"class":`input`,"type":`number`,"min":`1`,"max":`60`,"bind:value":maxMinutes,"disabled":!editable || saving,});/*Ωignore_startΩ*/() => maxMinutes = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
           }
         }
         { svelteHTML.createElement("div", { "class":`mode on`,});
           { svelteHTML.createElement("div", { "class":`mode-head`,});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`hand`,"size":14,}});}
             { svelteHTML.createElement("strong", {});  }
           }
           { svelteHTML.createElement("p", { "class":`mode-body`,});                  }
         }
         { svelteHTML.createElement("div", { "class":`mode on`,});
           { svelteHTML.createElement("div", { "class":`mode-head`,});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`calendar`,"size":14,}});}
             { svelteHTML.createElement("strong", {});  }
           }
           { svelteHTML.createElement("p", { "class":`mode-body`,});
                  
            schedules.length === 0
              ? 'No schedules yet.'
              : `${readOnlySchedules} of ${schedules.length} ${schedules.length === 1 ? 'is' : 'are'} read-only.`;
           }
           { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go(`personal-agents/${agentId}/schedules`),});  }
         }
       }
       { svelteHTML.createElement("p", {   "class":`note`,"role":`note`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`shield`,"size":12,}});}
                      
       }
     }

     { svelteHTML.createElement("section", {   "class":`pa-panel`,"aria-labelledby":`goals-h`,});
       { svelteHTML.createElement("div", { "class":`card-head`,});
         { svelteHTML.createElement("h2", { "id":`goals-h`,});  }
        if(ownerEdit){
           { const $$_button4 = svelteHTML.createElement("button", {      "class":`btn small`,"disabled":saving,"onclick":() => void addRow('goal'),});addGoalBtn = $$_button4;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
           }
        }
       }
      if(ownerNote){ { svelteHTML.createElement("p", {   "class":`hint`,"role":`note`,});ownerNote; }}
       { svelteHTML.createElement("p", { "class":`hint`,});                    }
      if(goals.length === 0){
         { svelteHTML.createElement("p", { "class":`muted`,});               }
      }else{
         { const $$_ul3 = svelteHTML.createElement("ul", {  "class":`list`,});goalsList = $$_ul3;
              for(let g of __sveltets_2_ensureArray(goals)){let i = 1;g.id ?? `new-${i}`;
             { svelteHTML.createElement("li", { "class":`item`,});
               { svelteHTML.createElement("input", {          "type":`checkbox`,"bind:checked":g.enabled,"disabled":!editable || saving,"aria-label":g.text.trim() ? `Goal enabled: ${g.text.trim()}` : 'Goal enabled',"title":`Enabled`,});/*Ωignore_startΩ*/() => g.enabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input grow`,"bind:value":g.text,"disabled":!ownerEdit || saving,"placeholder":`What should it keep an eye on?`,"aria-label":g.text.trim() ? `Goal: ${g.text.trim()}` : 'Goal',});/*Ωignore_startΩ*/() => g.text = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               { svelteHTML.createElement("span", { "class":`meta`,});  { const $$_emiTleR6C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR6C({ target: __sveltets_2_any(), props: {    "iso":g.last_run_at,"fallback":`never`,}});} }
              if(editable && g.id){
                 { svelteHTML.createElement("button", {         "class":`btn small`,"disabled":workingGoal !== null || dirty || saving,"aria-label":g.text.trim() ? `Work on it now: ${g.text.trim()}` : 'Work on it now',"title":dirty ? 'Save first' : 'Start a read-only run on this goal now',"onclick":() => g.id && workNow(g.id),});    }
              }
              if(ownerEdit){
                 { svelteHTML.createElement("button", {         "class":`icon-btn rm-row`,"disabled":saving,"aria-label":g.text.trim() ? `Remove goal: ${g.text.trim()}` : `Remove goal ${i + 1}`,"title":`Remove goal`,"onclick":() => void removeRow('goal', i),});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
                 }
              }
             }
          }
         }
      }
     }

     { svelteHTML.createElement("section", {   "class":`pa-panel`,"aria-labelledby":`rules-h`,});
       { svelteHTML.createElement("div", { "class":`card-head`,});
         { svelteHTML.createElement("h2", { "id":`rules-h`,});  }
        if(ownerEdit){
           { const $$_button4 = svelteHTML.createElement("button", {      "class":`btn small`,"disabled":saving,"onclick":() => void addRow('rule'),});addRuleBtn = $$_button4;
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
           }
        }
       }
      if(ownerNote){ { svelteHTML.createElement("p", {   "class":`hint`,"role":`note`,});ownerNote; }}
       { svelteHTML.createElement("p", { "class":`hint`,});                          }
      if(rules.length === 0){
         { svelteHTML.createElement("p", { "class":`muted`,});       }
      }else{
         { const $$_ul3 = svelteHTML.createElement("ul", {  "class":`list`,});rulesList = $$_ul3;
              for(let r of __sveltets_2_ensureArray(rules)){let i = 1;r.id ?? `new-${i}`;
             { svelteHTML.createElement("li", { "class":`item rule`,});
               { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input grow`,"bind:value":r.text,"disabled":!ownerEdit || saving,"placeholder":`e.g. Never post in #general`,"aria-label":rules.length > 1 ? `Rule ${i + 1}` : 'Rule',});/*Ωignore_startΩ*/() => r.text = __sveltets_2_any(null);/*Ωignore_endΩ*/}
              if(r.id && saved?.rules.some((x) => x.id === r.id && x.text === r.text.trim())){
                 { svelteHTML.createElement("span", {    "class":`chip`,"title":`Derived by Otto from the rule’s wording`,});!!r.enforce;
                  if(r.enforce){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}}enforceLabel(r);
                 }
              }else{
                 { svelteHTML.createElement("span", { "class":`meta`,});      }
              }
              if(ownerEdit){
                 { svelteHTML.createElement("button", {         "class":`icon-btn rm-row`,"disabled":saving,"aria-label":r.text.trim() ? `Remove rule: ${r.text.trim()}` : `Remove rule ${i + 1}`,"title":`Remove rule`,"onclick":() => void removeRow('rule', i),});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
                 }
              }
             }
          }
         }
      }
     }

     { svelteHTML.createElement("section", {   "class":`pa-panel`,"aria-labelledby":`primary-h`,});
       { svelteHTML.createElement("h2", { "id":`primary-h`,});  }
       { svelteHTML.createElement("p", { "class":`hint`,});                       }
       { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":primary,"disabled":!editable || saving,});/*Ωignore_startΩ*/() => primary = __sveltets_2_any(null);/*Ωignore_endΩ*/}      }
     }

    if(editable){
       { svelteHTML.createElement("div", { "class":`actions`,});
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":!dirty || saving,"onclick":() => saved && apply(saved),});  }
         { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!dirty || saving,"onclick":save,});saving ? 'Saving…' : 'Save'; }
       }
    }
   }
 LoadState}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentAutonomy__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentAutonomy__SvelteComponent_ = ReturnType<typeof AgentAutonomy__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentAutonomy__SvelteComponent_;