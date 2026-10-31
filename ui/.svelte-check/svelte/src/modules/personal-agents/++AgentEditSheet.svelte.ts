///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { untrack } from 'svelte';
import PathField from '../../lib/components/PathField.svelte';
// Create/edit sheet for one personal agent: name, avatar, persona (soul),
// provider + ModelPicker, browser toggle, delivery — mirrors the
// scheduled-tasks destination form. Schedules are edited on the agent page.
import Modal from '../../lib/components/Modal.svelte';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import { agentProvidersWith, defaultAgentProvider } from '../../lib/providers';
import { personalAgents } from '../../lib/stores/personalAgents.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { loadErrorText } from '../../lib/loadError';
import { ui } from '../../lib/stores/ui.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import type { PersonalAgent } from '../../lib/api/types';
import { agentTemplates, templateById } from './templates';
;

  interface Props {
    /** null = create a new agent. */
    agent: PersonalAgent | null;
    /** Pre-select a template (id from `templates.ts`) when creating. */
    template?: string | null;
    onclose: () => void;
  };;

  type DestType = 'none' | 'slack' | 'telegram' | 'email' | 'webhook';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { agent, template = null, onclose }: Props = $props();
  /** Prefix for the label↔control ids (several sheets can be mounted). */
  const uid = $props.id();

  // Templates only apply to a brand-new agent: they pre-fill the form and, on
  // create, add the template's first schedule.
  const TEMPLATES = agentTemplates();
  // svelte-ignore state_referenced_locally
  let fTemplate = $state<string>(agent ? '' : (template ?? ''));
  let scheduleNote = $state('');
  let createdAgent = $state<PersonalAgent | null>(null);
  let pendingSchedule: Parameters<typeof personalAgents.createSchedule>[1] | null = null;

  function applyTemplate(id: string): void {
    if (createdAgent) return;
    fTemplate = id;
    const t = templateById(id);
    if (!t) {
      scheduleNote = '';
      return;
    }
    fName = t.name;
    fAvatar = t.avatar;
    fSoul = t.soul_md;
    scheduleNote = `Adds a schedule: every ${t.schedule.every_min} min — “${t.directive}”`;
  }

  let busy = $state(false);
  let error = $state('');
  let errorEl = $state<HTMLDivElement | null>(null);
  // Save is in the fixed footer: reveal errors even after scrolling a long form.
  $effect(() => { if (error) errorEl?.scrollIntoView({ block: 'nearest' }); });

  // The sheet is mounted fresh per open (parent {#if}-gates it), so capturing
  // the agent's initial values into the form state is intentional.
  // svelte-ignore state_referenced_locally
  const init = agent;
  const d = init?.delivery ?? {};
  const savedDest = (d.type as string) ?? 'none';

  let fName = $state(init?.name ?? '');
  let fAvatar = $state(init?.avatar ?? '');
  let fSoul = $state(init?.soul_md ?? '');
  let fProvider = $state(init?.provider || defaultAgentProvider());
  let fModel = $state(init?.model ?? ''); // '' = provider default
  let fCwd = $state(init?.cwd ?? '');
  let fBrowser = $state(init?.browser ?? false);
  let fEnabled = $state(init?.enabled ?? true);
  let fDestType = $state<'none' | 'slack' | 'telegram' | 'email' | 'webhook'>(
    ['slack', 'telegram', 'email', 'webhook'].includes(savedDest)
      ? (savedDest as 'slack' | 'telegram' | 'email' | 'webhook')
      : 'none',
  );
  let fChatId = $state((d.chat_id as string) ?? '');
  let fEmailTo = $state((d.to as string) ?? '');
  let fUrl = $state((d.url as string) ?? '');
  // Template pre-fill runs after every form field exists (they are `let`s above).
  // svelte-ignore state_referenced_locally
  if (!agent && template) applyTemplate(template);

  // Live registry (built-ins + custom), never dropping a saved custom slug.
  const PROVIDERS = $derived(agentProvidersWith(agent?.provider));

  function onProviderSelect(v: string): void {
    // A model id belongs to one provider — switching clears it, so a hidden
    // stale model (the picker hides for providers without a model flag) is
    // never saved against the new provider.
    if (v !== fProvider) fModel = '';
    if (v === 'custom') {
      if (PROVIDERS.includes(fProvider)) fProvider = '';
    } else {
      fProvider = v;
    }
  }


  function deliveryTarget(type: DestType, chat: string, email: string, url: string): string {
    switch (type) {
      case 'slack':
      case 'telegram':
        return chat.trim();
      case 'email':
        return email.trim();
      case 'webhook':
        return url.trim();
      default:
        return '';
    }
  }

  // Mounted fresh per open — untrack keeps the saved destination, not a live binding.
  const initialDestType = untrack(() => fDestType);
  const initialDestTarget = untrack(() => deliveryTarget(fDestType, fChatId, fEmailTo, fUrl));

  /** True when save would start or retarget an outward report channel. */
  function deliversReports(): boolean {
    if (fDestType === 'none') return false;
    const target = deliveryTarget(fDestType, fChatId, fEmailTo, fUrl);
    return fDestType !== initialDestType || target !== initialDestTarget;
  }

  function deliveryConfirm(): string {
    const target = deliveryTarget(fDestType, fChatId, fEmailTo, fUrl);
    switch (fDestType) {
      case 'slack':
      case 'telegram': {
        const where = fDestType === 'slack' ? 'Slack' : 'Telegram';
        const dest = target ? `${where} (${target})` : `${where} (the integration’s default channel)`;
        return `Every run’s report will be posted to ${dest}. Everyone in that channel can see it. Unchanged reports aren’t re-sent.`;
      }
      case 'email':
        return `Every run’s report will be emailed to ${target || 'the address you entered'}. That recipient is who sees it. Unchanged reports aren’t re-sent.`;
      case 'webhook':
        return `Every run’s report will be POSTed to ${target || 'the webhook URL'}. Whoever controls that URL can read it. Unchanged reports aren’t re-sent.`;
      default:
        return '';
    }
  }

  function saveLabel(): string {
    if (busy) return 'Saving…';
    if (deliversReports()) return 'Save and deliver reports';
    return 'Save';
  }

  function buildDelivery(): Record<string, unknown> {
    switch (fDestType) {
      case 'slack':
      case 'telegram':
        return fChatId ? { type: fDestType, chat_id: fChatId } : { type: fDestType };
      case 'email':
        return { type: 'email', to: fEmailTo };
      case 'webhook':
        return { type: 'webhook', url: fUrl };
      default:
        return { type: 'none' };
    }
  }

  async function save(): Promise<void> {
    if (busy) return;
    error = '';
    if (!fName.trim()) {
      error = 'Name is required.';
      return;
    }
    if (!agent && !createdAgent && !ws.currentId) {
      error = 'Add or select a workspace before saving. Your changes are kept in this form.';
      return;
    }
    if (deliversReports()) {
      const ok = await confirmer.ask(deliveryConfirm(), {
        title: 'Deliver run reports?',
        confirmLabel: 'Save and deliver',
      });
      if (!ok) return;
    }
    const body = {
      name: fName.trim(),
      avatar: fAvatar.trim(),
      soul_md: fSoul,
      provider: fProvider.trim(),
      model: fModel.trim(),
      cwd: fCwd.trim(),
      browser: fBrowser,
      delivery: buildDelivery(),
      enabled: fEnabled,
    };
    busy = true;
    try {
      if (agent) {
        await personalAgents.update(agent.id, body);
        toasts.success(`Saved ${body.name}`);
      } else if (createdAgent || ws.currentId) {
        if (!createdAgent) {
          const t = templateById(fTemplate);
          pendingSchedule = t ? {
            schedule: t.schedule, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
            directive: t.directive, enabled: true, idempotency_key: crypto.randomUUID(),
          } : null;
          createdAgent = await personalAgents.create(ws.currentId!, body);
        } else {
          createdAgent = await personalAgents.update(createdAgent.id, body);
        }
        if (pendingSchedule) {
          await personalAgents.createSchedule(createdAgent.id, pendingSchedule);
          pendingSchedule = null;
        }
        toasts.success(`Created ${body.name}`, body.enabled ? undefined : 'It’s paused — enable it to let its schedules fire.');
      }
      onclose();
    } catch (e) {
      error = createdAgent
        ? `Agent created; its schedule couldn’t be added. Save again to finish setup for ${createdAgent.name}. ${loadErrorText(e)}`
        : `Couldn’t save the agent. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":agent ? `Edit ${agent.name}` : 'New personal agent',"width":620,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy,"onclick":save,});saveLabel(); }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`sheet`,});
    if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: error, assertive: true })));{ const $$_div2 = svelteHTML.createElement("div", __sveltets_2_union($$action_0), {    "class":`err`,});errorEl = $$_div2;error; }}}
    if(!agent && !ws.currentId){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (ui.newWorkspaceOpen = true),});  }
    }

    if(!agent){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`${uid}-template`,});     }
         { svelteHTML.createElement("select", {           "id":`${uid}-template`,"class":`input`,"disabled":busy || !!createdAgent,"value":fTemplate,"onchange":(e) => applyTemplate((e.currentTarget as HTMLSelectElement).value),...__sveltets_2_empty({"data-testid":`agent-template`}),});
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let t of __sveltets_2_ensureArray(TEMPLATES)){t.id; { svelteHTML.createElement("option", { "value":t.id,});t.title;  t.description; }}
         }
        if(scheduleNote){ { svelteHTML.createElement("span", { "class":`note`,});scheduleNote; }}
       }
    }

     { svelteHTML.createElement("div", { "class":`field-row`,});
       { svelteHTML.createElement("div", { "class":`field grow`,});
         { svelteHTML.createElement("label", { "for":`${uid}-name`,});  }
         { svelteHTML.createElement("input", {          "dir":`auto`,"id":`${uid}-name`,"class":`input`,"bind:value":fName,"placeholder":`Daily Recap`,});/*Ωignore_startΩ*/() => fName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field narrow`,});
         { svelteHTML.createElement("label", { "for":`${uid}-avatar`,});  }
         { svelteHTML.createElement("input", {          "dir":`auto`,"id":`${uid}-avatar`,"class":`input`,"bind:value":fAvatar,"maxlength":8,});/*Ωignore_startΩ*/() => fAvatar = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`hint`,});     }
       }
     }

     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`${uid}-soul`,});           }
       { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`${uid}-soul`,"class":`input`,"bind:value":fSoul,"rows":6,"placeholder":`You are a diligent chronicler…`,});/*Ωignore_startΩ*/() => fSoul = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }

     { svelteHTML.createElement("div", { "class":`field-row`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`${uid}-provider`,});  }
         { svelteHTML.createElement("select", {         "id":`${uid}-provider`,"class":`input`,"value":PROVIDERS.includes(fProvider) ? fProvider : 'custom',"onchange":(e) => onProviderSelect((e.currentTarget as HTMLSelectElement).value),});
             for(let p of __sveltets_2_ensureArray(PROVIDERS)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
           { svelteHTML.createElement("option", { "value":`custom`,});  }
         }
       }
       { svelteHTML.createElement("div", { "class":`cell`,});
         { const $$_rekciPledoM4C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM4C({ target: __sveltets_2_any(), props: {      "provider":fProvider,"value":fModel,"onchange":(m) => (fModel = m),}});}
       }
     }

    if(!PROVIDERS.includes(fProvider)){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`${uid}-slug`,});   }
         { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`${uid}-slug`,"class":`input`,"bind:value":fProvider,"placeholder":`my-custom-agent (register it in Settings first)`,});/*Ωignore_startΩ*/() => fProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
    }

     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`${uid}-cwd`,});          }
       { const $$_dleiFhtaP3C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP3 = new $$_dleiFhtaP3C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:fCwd,}});/*Ωignore_startΩ*/() => fCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP3.$$bindings = 'value'; { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`${uid}-cwd`,"class":`input`,"bind:value":fCwd,"placeholder":`defaults to the agent’s own workspace`,});/*Ωignore_startΩ*/() => fCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
     }

     { svelteHTML.createElement("div", { "class":`field-row`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`${uid}-dest`,});  }
         { svelteHTML.createElement("select", {     "id":`${uid}-dest`,"class":`input`,"bind:value":fDestType,});/*Ωignore_startΩ*/() => fDestType = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`none`,});       }
           { svelteHTML.createElement("option", { "value":`slack`,});  }
           { svelteHTML.createElement("option", { "value":`telegram`,});  }
           { svelteHTML.createElement("option", { "value":`email`,});  }
           { svelteHTML.createElement("option", { "value":`webhook`,});  }
         }
       }
      if(fDestType === 'slack' || fDestType === 'telegram'){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`${uid}-chat`,});     }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`${uid}-chat`,"class":`input`,"bind:value":fChatId,"placeholder":`defaults to the integration channel`,});/*Ωignore_startΩ*/() => fChatId = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      } else if (fDestType === 'email'){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`${uid}-email`,});   }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`${uid}-email`,"class":`input`,"bind:value":fEmailTo,"placeholder":`you@example.com`,});/*Ωignore_startΩ*/() => fEmailTo = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      } else if (fDestType === 'webhook'){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`${uid}-url`,});  }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`${uid}-url`,"class":`input`,"bind:value":fUrl,"placeholder":`https://…`,});/*Ωignore_startΩ*/() => fUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }
     }
    if(fDestType !== 'none'){
       { svelteHTML.createElement("p", {   "class":`note dim`,...__sveltets_2_empty({"data-testid":`delivery-note`}),});
        if(fDestType === 'slack' || fDestType === 'telegram'){        fDestType === 'slack' ? 'Slack' : 'Telegram';
          fChatId.trim() ? `(${fChatId.trim()})` : '(the integration’s default channel)';       } else if (fDestType === 'email'){        fEmailTo.trim() || 'the address above'; }else{        fUrl.trim() || 'the URL above'; }
           
       }
    }

     { svelteHTML.createElement("div", { "class":`toggles`,});
       { svelteHTML.createElement("label", { "class":`chk`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fBrowser,});/*Ωignore_startΩ*/() => fBrowser = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 
       }
       { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fEnabled,});/*Ωignore_startΩ*/() => fEnabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}         }
     }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentEditSheet__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentEditSheet__SvelteComponent_ = ReturnType<typeof AgentEditSheet__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentEditSheet__SvelteComponent_;