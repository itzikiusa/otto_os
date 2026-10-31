///<reference types="svelte" />
;// Create or edit a swarm agent: identity, soul, scope, provider, reports-to,
// skills (with must-use), and an optional schedule.

import { untrack } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { contextApi } from '../../lib/api/context';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import type { AgentSchedule, AgentSkill, CreateAgentReq, SwarmAgent } from './types';
import { agentProvidersWith, defaultAgentProvider } from '../../lib/providers';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { canEditSwarmIdentity, IDENTITY_OWNER_NOTE } from './access';
;

  interface Props {
    agent?: SwarmAgent | null;
    /** Prefill (e.g. from the recruiter). Ignored when `agent` is set. */
    prefill?: Partial<CreateAgentReq> | null;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { agent = null, prefill = null, onclose }: Props = $props();

  // Props are read once for this keyed one-shot modal (untrack avoids the
  // "only captures initial value" reactivity warning).
  const editing = untrack(() => !!agent);
  const src = untrack(() => agent ?? prefill ?? {}) as Partial<SwarmAgent>;
  const initSched = untrack(
    () => (agent?.schedule ?? prefill?.schedule ?? null) as AgentSchedule | null,
  );
  // Preserve a library-soul reference (soul_name) through edit AND duplicate —
  // the form only edits the inline soul_md, so without this a duplicate of a
  // library-soul agent would silently fall back to the default soul.
  const initSoulName = untrack(
    () => (agent?.soul_name ?? prefill?.soul_name ?? null) as string | null,
  );

  let name = $state(src.name ?? '');
  let title = $state(src.title ?? '');
  let provider = $state(src.provider ?? swarm.detail?.config.provider ?? defaultAgentProvider());
  // Model alias (e.g. opus / sonnet / haiku) — blank = the provider's default.
  // Lets you duplicate an agent and run the copy on a different model.
  let model = $state((src.model ?? '') as string);
  let reportsTo = $state<string>((src.reports_to ?? '') as string);
  let specialization = $state(src.specialization ?? '');
  let soulMd = $state((src.soul_md ?? '') as string);
  let scopeMd = $state((src.scope_md ?? '') as string);
  let avatar = $state(src.avatar ?? '🤖');
  let skills = $state<AgentSkill[]>([...((src.skills ?? []) as AgentSkill[])]);
  let newSkill = $state('');
  let scheduled = $state(!!initSched?.enabled);
  let cadence = $state<AgentSchedule['cadence']>(initSched?.cadence ?? 'daily');
  let everyMin = $state(initSched?.every_min ?? 60);
  let at = $state(initSched?.at ?? '09:00');
  let weekday = $state(initSched?.weekday ?? 1);
  let directive = $state(initSched?.directive ?? '');

  // PC-2 (access-control-6-01 / 6-03): prompt, soul, provider, model and the
  // standing directive are the swarm creator's, a ws admin's or root's to
  // write — on a hire as on an edit. Everyone else edits the rest; the
  // identity fields render read-only with the owner note.
  const canEditIdentity = $derived(canEditSwarmIdentity(
    {
      isRoot: auth.isRoot,
      myRole: ws.workspaces.find((w) => w.id === swarm.detail?.workspace_id)?.my_role,
      me: auth.me?.id,
    },
    swarm.detail?.created_by,
  ));
  // A non-creator's hire carries no identity (the server would refuse it):
  // it starts on the swarm's defaults, whatever a recruiter prefill proposed.
  if (!editing && untrack(() => !canEditIdentity)) {
    provider = untrack(() => swarm.detail?.config.provider ?? provider);
    model = '';
    specialization = '';
    soulMd = '';
    scopeMd = '';
    directive = '';
  }

  // Live registry (built-ins + custom, e.g. grok); keep the agent's current
  // provider even if it's since been removed from the registry.
  const providers = $derived(agentProvidersWith(provider));

  let busy = $state(false);
  let knownSkills = $state<string[]>([]);
  $effect(() => {
    contextApi.listSkills().then((s) => { knownSkills = s.map((sk) => sk.name); }).catch(() => {});
  });

  function addSkill() {
    const n = newSkill.trim();
    if (!n) return;
    if (!skills.some((s) => s.name === n)) skills = [...skills, { name: n, must_use: false }];
    newSkill = '';
  }

  // `at` is wall-clock time in this zone (the scheduler is timezone-aware).
  // An existing agent keeps its stored zone — pre-timezone schedules have
  // none and stay UTC; new schedules use the Mac's zone.
  const browserTz = (() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
    } catch {
      return 'UTC';
    }
  })();
  const schedTz = untrack(() => (initSched ? (initSched.timezone ?? 'UTC') : browserTz));

  // Unchecking "Scheduled runs" sends `schedule: null`, which clears it
  // server-side; the server also carries the stored run cursor across edits.
  function buildSchedule(): AgentSchedule | null {
    if (!scheduled) return null;
    const base: AgentSchedule = { cadence, directive, enabled: true, timezone: schedTz };
    // The server clamps to ≥ 5 min (cadence.rs); send what will actually run.
    if (cadence === 'interval') base.every_min = Math.max(5, Math.round(Number(everyMin) || 5));
    if (cadence === 'daily') base.at = at;
    if (cadence === 'weekly') {
      base.at = at;
      base.weekday = weekday;
    }
    return base;
  }

  async function save() {
    if (!name.trim() || busy || !swarm.detail) return;
    busy = true;
    const body: CreateAgentReq = {
      name: name.trim(),
      provider,
      model: model.trim() || null,
      title: title.trim(),
      reports_to: reportsTo || null,
      specialization: specialization.trim(),
      soul_md: soulMd.trim() || null,
      soul_name: initSoulName,
      scope_md: scopeMd.trim(),
      avatar: avatar.trim(),
      skills,
      schedule: buildSchedule(),
    };
    try {
      if (editing && agent) await swarm.updateAgent(agent.id, body);
      else await swarm.createAgent(swarm.detail.id, body);
      toasts.success(editing ? 'Agent updated' : 'Agent hired');
      onclose();
    } catch (e) {
      toastError(editing ? 'Couldn’t save the agent' : 'Couldn’t hire the agent', e);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":editing ? `Edit ${agent?.name}` : 'New agent',"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":save,"disabled":!name.trim() || busy,});
      editing ? 'Save' : 'Hire';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`grid2`,});
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ag-name`,});  }
       { svelteHTML.createElement("input", {        "dir":`auto`,"id":`ag-name`,"class":`input`,"bind:value":name,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ag-title`,});   }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ag-title`,"class":`input`,"bind:value":title,"placeholder":`e.g. CTO`,});/*Ωignore_startΩ*/() => title = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ag-provider`,});  }
       { svelteHTML.createElement("select", {       "id":`ag-provider`,"class":`input`,"bind:value":provider,"disabled":!canEditIdentity,});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(providers)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
       }
     }
    
    if(canEditIdentity){
       { const $$_rekciPledoM2C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM2C({ target: __sveltets_2_any(), props: {     provider,"value":model,"onchange":(m) => (model = m),}});}
    } else if (model){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ag-model`,});  }
          { svelteHTML.createElement("input", {        "id":`ag-model`,"class":`input`,"dir":`ltr`,"value":model,"readonly":true,});}
       }
    }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ag-reports`,});  }
       { svelteHTML.createElement("select", {     "id":`ag-reports`,"class":`input`,"bind:value":reportsTo,});/*Ωignore_startΩ*/() => reportsTo = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});    }
           for(let a of __sveltets_2_ensureArray((swarm.detail?.agents ?? []).filter((a) => a.id !== agent?.id))){a.id;
           { svelteHTML.createElement("option", { "value":a.id,});a.name; a.title;  }
        }
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ag-avatar`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ag-avatar`,"class":`input`,"bind:value":avatar,"maxlength":4,});/*Ωignore_startΩ*/() => avatar = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ag-spec`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ag-spec`,"class":`input`,"bind:value":specialization,"readonly":!canEditIdentity,});/*Ωignore_startΩ*/() => specialization = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ag-soul`,});    }
     { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`ag-soul`,"class":`input`,"rows":3,"bind:value":soulMd,"readonly":!canEditIdentity,});/*Ωignore_startΩ*/() => soulMd = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
   }
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ag-scope`,});    }
     { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`ag-scope`,"class":`input`,"rows":2,"bind:value":scopeMd,"readonly":!canEditIdentity,});/*Ωignore_startΩ*/() => scopeMd = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
   }
  if(!canEditIdentity){
     { svelteHTML.createElement("p", { "class":`owner-note dim`,});IDENTITY_OWNER_NOTE; }
  }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ag-skill`,});   }
     { svelteHTML.createElement("div", { "class":`skill-add`,});
       { svelteHTML.createElement("input", {              "dir":`auto`,"id":`ag-skill`,"class":`input grow`,"placeholder":`skill name…`,"bind:value":newSkill,"list":`ag-skill-list`,"onkeydown":(e) => e.key === 'Enter' && addSkill(),});/*Ωignore_startΩ*/() => newSkill = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("datalist", { "id":`ag-skill-list`,});
           for(let sk of __sveltets_2_ensureArray(knownSkills)){sk; { svelteHTML.createElement("option", { "value":sk,}); }}
       }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":addSkill,});  }
     }
     { svelteHTML.createElement("div", { "class":`skills`,});
          for(let s of __sveltets_2_ensureArray(skills)){let i = 1;s.name;
         { svelteHTML.createElement("span", {  "class":`skill-chip`,});s.must_use;
           { svelteHTML.createElement("button", {           "class":`link must-toggle`,"aria-label":s.must_use ? `Make ${s.name} optional` : `Require ${s.name} (must use)`,"aria-pressed":s.must_use,"title":s.must_use ? 'Must use — click to make optional' : 'Optional — click to require',"onclick":() => (skills[i] = { ...s, must_use: !s.must_use }),});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});}
           }
          s.name;
           { svelteHTML.createElement("button", {        "class":`link`,"aria-label":`Remove skill ${s.name}`,"title":`Remove skill`,"onclick":() => (skills = skills.filter((_, j) => j !== i)),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
         }
      }
     }
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "class":`row`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":scheduled,});/*Ωignore_startΩ*/() => scheduled = __sveltets_2_any(null);/*Ωignore_endΩ*/}  
     }
    if(scheduled){
       { svelteHTML.createElement("div", { "class":`sched`,});
         { svelteHTML.createElement("select", {     "aria-label":`Schedule cadence`,"class":`input small`,"bind:value":cadence,});/*Ωignore_startΩ*/() => cadence = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`interval`,});   }
           { svelteHTML.createElement("option", { "value":`daily`,});  }
           { svelteHTML.createElement("option", { "value":`weekly`,});  }
         }
        if(cadence === 'interval'){
           { svelteHTML.createElement("input", {            "class":`input small`,"type":`number`,"min":`5`,"bind:value":everyMin,"onblur":() => (everyMin = Math.max(5, Math.round(Number(everyMin) || 5))),"aria-label":`Every N minutes (minimum 5)`,});/*Ωignore_startΩ*/() => everyMin = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
        if(cadence === 'daily' || cadence === 'weekly'){
          
           { svelteHTML.createElement("input", {          "class":`input small`,"type":`time`,"bind:value":at,"aria-label":`Run time (${schedTz})`,"title":`Time of day in ${schedTz}`,});/*Ωignore_startΩ*/() => at = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {   "class":`dim`,"title":`The time is interpreted in this timezone`,});schedTz; }
        }
        if(cadence === 'weekly'){
           { svelteHTML.createElement("select", {     "aria-label":`Weekday`,"class":`input small`,"bind:value":weekday,});/*Ωignore_startΩ*/() => weekday = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":0,});  } { svelteHTML.createElement("option", { "value":1,});  }
             { svelteHTML.createElement("option", { "value":2,});  } { svelteHTML.createElement("option", { "value":3,});  }
             { svelteHTML.createElement("option", { "value":4,});  } { svelteHTML.createElement("option", { "value":5,});  }
             { svelteHTML.createElement("option", { "value":6,});  }
           }
        }
       }
       { svelteHTML.createElement("textarea", {               "dir":`auto`,"class":`input`,"rows":2,"aria-label":`Standing directive`,"placeholder":canEditIdentity ? 'Standing directive (what to do each run)…' : '',"bind:value":directive,"readonly":!canEditIdentity,"title":canEditIdentity ? undefined : IDENTITY_OWNER_NOTE,});/*Ωignore_startΩ*/() => directive = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
    }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentEditor__SvelteComponent_ = ReturnType<typeof AgentEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentEditor__SvelteComponent_;