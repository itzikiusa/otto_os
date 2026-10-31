///<reference types="svelte" />
;
import { loadErrorText } from '../../lib/loadError';
// Recruiter: name a role → an AI helper proposes a full agent definition
// (title, reports-to, specialization, soul, skills, provider, schedule) → edit
// → Hire.
import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import Modal from '../../lib/components/Modal.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { isAbortError } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { AgentSkill, CreateAgentReq, RecruitedAgent } from './types';
import { agentProvidersWith, defaultAgentProvider } from '../../lib/providers';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import Icon from '../../lib/components/Icon.svelte';
;

  interface Props {
    onclose: () => void;
    /** Open directly on the editable proposal (e.g. hiring from a completed
     *  recruit run whose modal was closed). */
    proposal?: RecruitedAgent | null;
    /** The recruit run this proposal came from — marked hired on success so its
     *  Runs-list "Hire" button disappears. */
    proposalRunId?: string | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onclose, proposal = null, proposalRunId = null }: Props = $props();

  // Read the seed proposal once (untrack avoids the props-in-$state warning).
  const seed = untrack(() => proposal);
  const seededReportsTo = untrack(
    () =>
      (seed?.reports_to_title
        ? (swarm.detail?.agents ?? []).find(
            (a) => a.title.toLowerCase() === seed.reports_to_title!.toLowerCase(),
          )?.id
        : '') ?? '',
  );

  let step = $state(seed ? 1 : 0);
  let role = $state('');
  let context = $state('');
  let busy = $state(false);
  let recruitCtl: AbortController | null = null;

  // Editable proposal fields (pre-filled when hiring from a completed run).
  let name = $state(seed?.name ?? '');
  let title = $state(seed?.title ?? '');
  let provider = $state(seed?.suggested_provider ?? defaultAgentProvider());
  let model = $state(seed?.suggested_model ?? '');
  let reportsTo = $state(seededReportsTo);
  let specialization = $state(seed?.specialization ?? '');
  let soulMd = $state(seed?.soul_md ?? '');
  let scopeMd = $state(seed?.scope_md ?? '');
  let avatar = $state(seed?.avatar ?? '🤖');
  let skills = $state<AgentSkill[]>(
    seed ? seed.skills.map((s) => ({ name: s.name, must_use: s.must_use })) : [],
  );
  let scheduleRaw = $state<RecruitedAgent['suggested_schedule']>(seed?.suggested_schedule ?? null);
  // How many copies to hire at once (e.g. the same role on 2 models / shifts).
  let count = $state(1);

  // Naming theme: the recruiter derives the agent's name from it (a fun, cohesive
  // roster). Defaults to the swarm's saved theme; persisted on recruit.
  const NAMING_THEMES = [
    'Famous footballers',
    'NBA legends',
    'F1 drivers',
    'Greek mythology',
    'Norse mythology',
    'Sci-fi captains',
    'Classical composers',
    'Renowned scientists',
    'Chess grandmasters',
    'Jazz legends',
  ];
  let namingTheme = $state(untrack(() => swarm.detail?.config?.naming_theme ?? ''));

  async function recruit() {
    if (!role.trim() || busy) return;
    recruitCtl = new AbortController();
    busy = true;
    // Remember the theme on the swarm so the whole roster stays consistent.
    if (swarm.detail && (swarm.detail.config?.naming_theme ?? '') !== namingTheme) {
      void swarm.setNamingTheme(swarm.detail.id, namingTheme);
    }
    try {
      const r = await swarm.recruit(
        role.trim(),
        context.trim() || undefined,
        namingTheme || undefined,
        recruitCtl.signal,
      );
      name = r.name;
      title = r.title || role.trim();
      provider = r.suggested_provider || defaultAgentProvider();
      model = r.suggested_model ?? '';
      specialization = r.specialization;
      soulMd = r.soul_md;
      scopeMd = r.scope_md;
      avatar = r.avatar || '🤖';
      skills = r.skills.map((s) => ({ name: s.name, must_use: s.must_use }));
      scheduleRaw = r.suggested_schedule ?? null;
      // Resolve reports-to by title against existing agents.
      if (r.reports_to_title) {
        const m = (swarm.detail?.agents ?? []).find(
          (a) => a.title.toLowerCase() === r.reports_to_title!.toLowerCase(),
        );
        reportsTo = m?.id ?? '';
      }
      step = 1;
    } catch (e) {
      if (isAbortError(e)) toasts.info('Recruiting stopped');
      else toastError('The Recruiter couldn’t propose an agent', e);
    } finally {
      busy = false;
      recruitCtl = null;
    }
  }

  async function stopRecruit() {
    const what = role.trim() || 'this role';
    const ok = await confirmer.ask(
      `Stop recruiting “${what}”? The recruiter’s session is closed and this proposal is discarded.`,
      { title: 'Stop recruiting', confirmLabel: 'Stop', danger: true },
    );
    if (!ok || !busy) return;
    if (swarm.detail) void swarm.stopAgentRun(swarm.detail.id, 'recruit');
    recruitCtl?.abort();
  }

  async function hire() {
    if (!name.trim() || busy || !swarm.detail) return;
    busy = true;
    const n = Math.max(1, Math.min(20, Math.floor(count) || 1));
    const base: CreateAgentReq = {
      name: name.trim(),
      provider,
      model: model.trim() || null,
      title: title.trim(),
      reports_to: reportsTo || null,
      specialization: specialization.trim(),
      soul_md: soulMd.trim() || null,
      scope_md: scopeMd.trim(),
      avatar: avatar.trim(),
      skills,
      schedule: scheduleRaw ?? null,
    };
    const hiredNames: string[] = [];
    try {
      for (let i = 0; i < n; i++) {
        // Suffix names when hiring multiple so they’re distinguishable.
        const body = n > 1 ? { ...base, name: `${base.name} ${i + 1}` } : base;
        await swarm.createAgent(swarm.detail.id, body);
        hiredNames.push(body.name);
      }
      toasts.success(n > 1 ? `Hired ${n}× ${name}` : `${name} hired`);
      if (proposalRunId) swarm.markRecruitHired(proposalRunId);
      onclose();
    } catch (e) {
      // A mid-batch failure still hired the first copies — a partial success
      // (warning) that names who exists, so a retry doesn’t silently double them.
      if (hiredNames.length > 0) {
        toasts.warn(
          `Hired ${hiredNames.length} of ${n}`,
          `Hired: ${hiredNames.join(', ')}. Couldn’t hire the rest: ${loadErrorText(e)}`,
        );
      } else {
        toastError('Couldn’t hire the agent', e);
      }
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Recruit an agent`,"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(step === 0){
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":onclose,});  }
      if(busy){
         { svelteHTML.createElement("button", {     "class":`btn small danger`,"onclick":() => void stopRecruit(),"title":`Stop recruiting ${role.trim() || 'this role'}`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} 
         }
      }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":recruit,"disabled":!role.trim() || busy,});
        busy ? 'Recruiting…' : 'Propose agent';
       }
    }else{
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (step = 0),});  }
       { svelteHTML.createElement("label", {   "class":`count`,"title":`Hire this many copies (e.g. the same role on different models)`,});
          { svelteHTML.createElement("input", {          "class":`input num`,"type":`number`,"min":`1`,"max":`20`,"bind:value":count,});/*Ωignore_startΩ*/() => count = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":hire,"disabled":!name.trim() || busy,});
        busy ? 'Hiring…' : count > 1 ? `Hire ${count}` : 'Hire';
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  if(step === 0){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`rc-role`,});       }
       { svelteHTML.createElement("input", {            "dir":`auto`,"id":`rc-role`,"class":`input`,"placeholder":`e.g. CTO, Backend Dev, UX Researcher`,"bind:value":role,"onkeydown":(e) => e.key === 'Enter' && recruit(),});/*Ωignore_startΩ*/() => role = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`rc-ctx`,});   }
       { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`rc-ctx`,"class":`input`,"rows":3,"bind:value":context,"placeholder":`Context to guide the recruiter…`,});/*Ωignore_startΩ*/() => context = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`rc-theme`,});   { svelteHTML.createElement("span", { "class":`dim`,});       } }
       { svelteHTML.createElement("select", {     "id":`rc-theme`,"class":`input`,"bind:value":namingTheme,});/*Ωignore_startΩ*/() => namingTheme = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});    }
           for(let t of __sveltets_2_ensureArray(NAMING_THEMES)){t; { svelteHTML.createElement("option", { "value":t,});t; }}
       }
     }
     { svelteHTML.createElement("p", { "class":`dim small`,});                }
  }else{
     { svelteHTML.createElement("div", { "class":`grid2`,});
       { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rc-name`,});  } { svelteHTML.createElement("input", {        "dir":`auto`,"id":`rc-name`,"class":`input`,"bind:value":name,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rc-title`,});  } { svelteHTML.createElement("input", {        "dir":`auto`,"id":`rc-title`,"class":`input`,"bind:value":title,});/*Ωignore_startΩ*/() => title = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`rc-prov`,});  }
         { svelteHTML.createElement("select", {     "id":`rc-prov`,"class":`input`,"bind:value":provider,});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(agentProvidersWith(provider))){p; { svelteHTML.createElement("option", { "value":p,});p; }}
         }
       }
      
       { const $$_rekciPledoM2C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM2C({ target: __sveltets_2_any(), props: {     provider,"value":model,"onchange":(m) => (model = m),}});}
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`rc-rep`,});  }
         { svelteHTML.createElement("select", {     "id":`rc-rep`,"class":`input`,"bind:value":reportsTo,});/*Ωignore_startΩ*/() => reportsTo = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});    }
             for(let a of __sveltets_2_ensureArray(swarm.detail?.agents ?? [])){a.id; { svelteHTML.createElement("option", { "value":a.id,});a.name; a.title;  }}
         }
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rc-spec`,});  } { svelteHTML.createElement("input", {        "dir":`auto`,"id":`rc-spec`,"class":`input`,"bind:value":specialization,});/*Ωignore_startΩ*/() => specialization = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
     { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rc-soul`,});  } { svelteHTML.createElement("textarea", {         "dir":`auto`,"id":`rc-soul`,"class":`input`,"rows":3,"bind:value":soulMd,});/*Ωignore_startΩ*/() => soulMd = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
     { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rc-scope`,});  } { svelteHTML.createElement("textarea", {         "dir":`auto`,"id":`rc-scope`,"class":`input`,"rows":2,"bind:value":scopeMd,});/*Ωignore_startΩ*/() => scopeMd = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("span", { "class":`label`,});  { svelteHTML.createElement("span", { "class":`dim`,});     } }
       { svelteHTML.createElement("div", { "class":`skills`,});
            for(let s of __sveltets_2_ensureArray(skills)){let i = 1;s.name;
           { svelteHTML.createElement("span", {  "class":`skill-chip`,});s.must_use;
             { svelteHTML.createElement("button", {          "class":`link must-toggle`,"aria-label":s.must_use ? `Make ${s.name} optional` : `Require ${s.name} (must use)`,"aria-pressed":s.must_use,"title":s.must_use ? 'Must use — click to make optional' : 'Optional — click to require',"onclick":() => (skills[i] = { ...s, must_use: !s.must_use }),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});} }
            s.name;
             { svelteHTML.createElement("button", {        "class":`link`,"aria-label":`Remove skill ${s.name}`,"title":`Remove skill`,"onclick":() => (skills = skills.filter((_, j) => j !== i)),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
           }
        }
        if(skills.length === 0){ { svelteHTML.createElement("span", { "class":`dim small`,});    }}
       }
     }
    if(scheduleRaw){ { svelteHTML.createElement("p", { "class":`dim small`,});  scheduleRaw.cadence;scheduleRaw.at ? ` @ ${scheduleRaw.at} UTC` : '';        }}
  }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RecruiterWizard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RecruiterWizard__SvelteComponent_ = ReturnType<typeof RecruiterWizard__SvelteComponent_>;
/*Ωignore_endΩ*/export default RecruiterWizard__SvelteComponent_;