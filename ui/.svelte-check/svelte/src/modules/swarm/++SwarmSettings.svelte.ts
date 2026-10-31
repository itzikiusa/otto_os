///<reference types="svelte" />
;
import PathField from '../../lib/components/PathField.svelte';
import { toastError } from '../../lib/toastError';
// Swarm settings: three tabs —
//   • Standing goals — the swarm's quality bar, applied to every task (PUT set)
//   • Team skills    — library skills every agent inherits (config.skills)
//   • Triggers       — channel rules that auto-launch swarm work
import Modal from '../../lib/components/Modal.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Switch from '../../lib/components/Switch.svelte';
import Tabs from '../../lib/components/Tabs.svelte';
import GoalEditor from './GoalEditor.svelte';
import SkillPicker from './SkillPicker.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { CreateGoalReq, CreateTriggerReq, Swarm, SwarmChannelTrigger, SwarmGoal } from './types';
;
  type Tab = 'goals' | 'skills' | 'triggers';;
;type $$ComponentProps =  { onclose: () => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onclose }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const detail = $derived(swarm.detail);

  let tab = $state<Tab>('goals');

  // -- Standing goals (edited as a local draft set, PUT on Save) --------------
  let draftGoals = $state<CreateGoalReq[]>([]);
  let goalsLoaded = $state(false);
  // True once the saved set has landed in the draft. Save PUTs the WHOLE set,
  // so saving before that would replace the swarm's goals with an empty list.
  let goalsReady = $state(false);
  let savingGoals = $state(false);
  let goalEditorOpen = $state(false);
  let goalEditIndex = $state<number>(-1);

  let goalsLoading = $state(false);
  /** (Re)load the saved set into the draft. A failed load leaves the draft NOT
   *  ready (Save stays off): saving a list we never loaded would clobber it. */
  async function loadGoals(sid: string): Promise<void> {
    goalsLoading = true;
    await swarm.loadStandingGoals(sid);
    goalsLoading = false;
    if (swarm.standingGoalsError) {
      goalsReady = false;
      return;
    }
    draftGoals = swarm.standingGoals.map((g) => ({
      title: g.title,
      description: g.description,
      metric: g.metric ?? undefined,
      comparator: g.comparator ?? undefined,
      target_value: g.target_value ?? undefined,
      block_value: g.block_value ?? undefined,
      verify_cmd: g.verify_cmd ?? undefined,
      max_retries: g.max_retries,
      blocking: g.blocking,
      order_idx: g.order_idx,
    }));
    goalsReady = true;
  }
  $effect(() => {
    const sid = detail?.id;
    if (!sid || goalsLoaded) return;
    goalsLoaded = true;
    void loadGoals(sid);
  });

  function addGoal() {
    goalEditIndex = -1;
    goalEditorOpen = true;
  }
  function editGoal(i: number) {
    goalEditIndex = i;
    goalEditorOpen = true;
  }
  function onGoalSubmit(req: CreateGoalReq) {
    if (goalEditIndex >= 0) draftGoals = draftGoals.map((g, i) => (i === goalEditIndex ? req : g));
    else draftGoals = [...draftGoals, req];
  }
  function removeGoal(i: number) {
    draftGoals = draftGoals.filter((_, j) => j !== i);
  }
  async function saveGoals() {
    if (!detail) return;
    savingGoals = true;
    try {
      await swarm.putStandingGoals(detail.id, draftGoals);
      toasts.success('Standing goals saved');
    } catch (e) {
      toastError('Couldn’t save the standing goals', e);
    } finally {
      savingGoals = false;
    }
  }

  // -- Team skills (config.skills) -------------------------------------------
  const teamSkills = $derived(((detail?.config.skills ?? []) as unknown[]).map((s) => String(s)));
  async function setTeamSkills(next: string[]) {
    if (!detail) return;
    const cfg = { ...(detail.config ?? {}), skills: next };
    try {
      await swarm.updateSwarm(detail.id, { config: cfg } as Partial<Swarm>);
    } catch (e) {
      toastError('Couldn’t save the team skills', e);
    }
  }

  // -- Triggers --------------------------------------------------------------
  let triggersLoaded = $state(false);
  let triggerForm = $state<CreateTriggerReq | null>(null);
  let triggerEditId = $state<string | null>(null);
  let savingTrigger = $state(false);
  let triggersLoading = $state(false);
  async function loadTriggers(sid: string): Promise<void> {
    triggersLoading = true;
    await swarm.loadTriggers(sid);
    triggersLoading = false;
  }

  $effect(() => {
    const sid = detail?.id;
    if (!sid || triggersLoaded) return;
    triggersLoaded = true;
    void loadTriggers(sid);
  });

  function channelName(channel: string): string {
    switch (channel) {
      case 'slack':
        return 'Slack';
      case 'telegram':
        return 'Telegram';
      case 'webhook':
        return 'Webhook';
      default:
        return channel;
    }
  }

  function triggerWhere(t: { channel: string; keyword?: string | null; match_chat?: string | null }): string {
    const keyword = t.keyword?.trim() || 'any message';
    const chat = t.match_chat?.trim() || 'any chat';
    return `${channelName(t.channel)} trigger “${keyword}” in ${chat}`;
  }

  function newTrigger() {
    triggerEditId = null;
    triggerForm = { channel: 'slack', match_chat: '', keyword: '', repo_path: '', auto_start: true, reply: false, enabled: true };
  }
  function editTrigger(t: SwarmChannelTrigger) {
    triggerEditId = t.id;
    triggerForm = {
      channel: t.channel,
      match_chat: t.match_chat,
      keyword: t.keyword,
      repo_path: t.repo_path ?? '',
      auto_start: t.auto_start,
      reply: t.reply,
      enabled: t.enabled,
    };
  }
  async function saveTrigger() {
    if (!detail || !triggerForm || savingTrigger) return;
    const chat = triggerForm.match_chat?.trim() ?? '';
    if (triggerForm.reply && !chat) {
      toasts.warn(
        'Name a chat to reply in',
        'Reply stays off until you pick one chat. Otherwise the swarm would post in every conversation.',
      );
      return;
    }
    if (triggerForm.reply) {
      const channel = channelName(triggerForm.channel);
      const ok = await confirmer.ask(
        `Reply in the ${channel} chat “${chat}”? Anyone who can read that chat will see the swarm’s reply.`,
        { title: 'Reply on the channel', confirmLabel: 'Reply there' },
      );
      if (!ok) return;
    }
    savingTrigger = true;
    try {
      const body: CreateTriggerReq = {
        channel: triggerForm.channel,
        match_chat: triggerForm.match_chat?.trim() || undefined,
        keyword: triggerForm.keyword?.trim() || undefined,
        repo_path: triggerForm.repo_path?.trim() || undefined,
        auto_start: triggerForm.auto_start,
        reply: triggerForm.reply,
        enabled: triggerForm.enabled,
      };
      // An edit must CLEAR what the user emptied: '' for the text filters
      // and `null` for the repo path (an omitted key leaves it unchanged).
      if (triggerEditId)
        await swarm.updateTrigger(triggerEditId, {
          ...body,
          match_chat: triggerForm.match_chat?.trim() ?? '',
          keyword: triggerForm.keyword?.trim() ?? '',
          repo_path: triggerForm.repo_path?.trim() || null,
        });
      else await swarm.createTrigger(detail.id, body);
      toasts.success(triggerEditId ? 'Trigger updated' : 'Trigger added');
      triggerForm = null;
      triggerEditId = null;
    } catch (e) {
      toastError('Couldn’t save the trigger', e);
    } finally {
      savingTrigger = false;
    }
  }
  async function toggleEnabled(t: SwarmChannelTrigger) {
    try {
      await swarm.updateTrigger(t.id, { enabled: !t.enabled });
    } catch (e) {
      toastError('Couldn’t update the trigger', e);
    }
  }
  async function delTrigger(t: SwarmChannelTrigger) {
    if (
      await confirmer.ask(`Delete this ${triggerWhere(t)}? Matching messages stop launching swarm work.`, {
        title: 'Delete trigger',
        confirmLabel: 'Delete',
        danger: true,
      })
    ) {
      try {
        await swarm.deleteTrigger(t.id);
      } catch (e) {
        toastError('Couldn’t delete the trigger', e);
      }
    }
  }

  const TABS: { id: Tab; label: string; icon: IconName }[] = [
    { id: 'goals', label: 'Standing goals', icon: 'check' },
    { id: 'skills', label: 'Team skills', icon: 'zap' },
    { id: 'triggers', label: 'Triggers', icon: 'comment' },
  ];
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Swarm settings${detail ? ` — ${detail.name}` : ''}`,"width":640,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":onclose,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`settings-tabs`,});
     { const $$_sbaT2C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT2C({ target: __sveltets_2_any(), props: {          "label":`Swarm settings section`,"tabs":TABS,"value":tab,"onchange":(id) => (tab = id),"idBase":`swarm-settings`,}});}
   }

   { svelteHTML.createElement("div", {     "role":`tabpanel`,"id":`swarm-settings-panel-${tab}`,"aria-labelledby":`swarm-settings-tab-${tab}`,});
  if(tab === 'goals'){
     { svelteHTML.createElement("p", { "class":`hint`,});                   }
     { svelteHTML.createElement("div", { "class":`bar`,});
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":addGoal,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":saveGoals,"disabled":savingGoals || !goalsReady,"title":goalsReady ? undefined : swarm.standingGoalsError ? 'Load the saved standing goals first — saving now would replace them' : 'Loading the saved standing goals…',});
        savingGoals ? 'Saving…' : 'Save standing goals';
       }
     }
    if(!goalsReady){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           "what":`standing goals`,"variant":`compact`,"loading":goalsLoading || !swarm.standingGoalsError,"error":swarm.standingGoalsError,"empty":true,"onretry":() => detail && void loadGoals(detail.id),}});}
    } else if (draftGoals.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`check`,"title":`No standing goals`,"body":`Add goals the whole swarm must hit — e.g. tests pass, no new clippy warnings.`,}});}
    }else{
       { svelteHTML.createElement("div", { "class":`list`,});
            for(let g of __sveltets_2_ensureArray(draftGoals)){let i = 1;i;
           { svelteHTML.createElement("div", { "class":`row-item`,});
             { svelteHTML.createElement("div", { "class":`ri-main`,});
               { svelteHTML.createElement("span", { "class":`ri-title`,});g.title; }
              if(g.blocking){ { svelteHTML.createElement("span", {   "class":`blocking`,"title":`Blocks task completion until it passes`,});  }}
              if(g.metric){ { svelteHTML.createElement("span", { "class":`dim`,}); g.metric;g.comparator ? ` ${g.comparator}` : '';g.target_value != null ? ` ${g.target_value}` : ''; }}
             }
             { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":() => editGoal(i),"aria-label":`Edit standing goal “${g.title}”`,"title":`Edit standing goal “${g.title}”`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});} }
             { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":() => removeGoal(i),"aria-label":`Remove standing goal “${g.title}”`,"title":`Remove “${g.title}” (takes effect on Save)`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});} }
           }
        }
       }
    }
  } else if (tab === 'skills'){
     { svelteHTML.createElement("p", { "class":`hint`,});                 }
     { const $$_rekciPllikS2C = __sveltets_2_ensureComponent(SkillPicker); new $$_rekciPllikS2C({ target: __sveltets_2_any(), props: {    "selected":teamSkills,"onchange":setTeamSkills,}});}
  } else if (tab === 'triggers'){
     { svelteHTML.createElement("p", { "class":`hint`,});            }
    if(triggerForm){
       { svelteHTML.createElement("div", { "class":`form`,});
         { svelteHTML.createElement("div", { "class":`grid`,});
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", { "for":`t-channel`,});  }
             { svelteHTML.createElement("select", {     "id":`t-channel`,"class":`input`,"bind:value":triggerForm.channel,});/*Ωignore_startΩ*/() => triggerForm.channel = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":`slack`,});  }
               { svelteHTML.createElement("option", { "value":`telegram`,});  }
               { svelteHTML.createElement("option", { "value":`webhook`,});  }
             }
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", { "for":`t-chat`,});     { svelteHTML.createElement("span", { "class":`dim`,});   } }
             { svelteHTML.createElement("input", {          "dir":`auto`,"id":`t-chat`,"class":`input`,"bind:value":triggerForm.match_chat,"placeholder":`e.g. #builds or chat id`,});/*Ωignore_startΩ*/() => triggerForm.match_chat = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", { "for":`t-kw`,});  { svelteHTML.createElement("span", { "class":`dim`,});   } }
             { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`t-kw`,"class":`input`,"bind:value":triggerForm.keyword,"placeholder":`e.g. /swarm`,});/*Ωignore_startΩ*/() => triggerForm.keyword = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", { "for":`t-repo`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
             { const $$_dleiFhtaP5C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP5 = new $$_dleiFhtaP5C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:triggerForm.repo_path,}});/*Ωignore_startΩ*/() => triggerForm.repo_path = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP5.$$bindings = 'value'; { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`t-repo`,"class":`input`,"bind:value":triggerForm.repo_path,"placeholder":`/path/to/repo`,});/*Ωignore_startΩ*/() => triggerForm.repo_path = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
           }
         }
         { svelteHTML.createElement("div", { "class":`toggles`,});
           { svelteHTML.createElement("label", { "class":`row`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":triggerForm.auto_start,});/*Ωignore_startΩ*/() => triggerForm.auto_start = __sveltets_2_any(null);/*Ωignore_endΩ*/}    }
           { svelteHTML.createElement("label", { "class":`row`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":triggerForm.reply,});/*Ωignore_startΩ*/() => triggerForm.reply = __sveltets_2_any(null);/*Ωignore_endΩ*/}     }
           { svelteHTML.createElement("label", { "class":`row`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":triggerForm.enabled,});/*Ωignore_startΩ*/() => triggerForm.enabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}  }
         }
         { svelteHTML.createElement("div", { "class":`form-actions`,});
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => { triggerForm = null; triggerEditId = null; },});  }
           { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":saveTrigger,"disabled":savingTrigger || !!swarm.triggersError,"title":swarm.triggersError ? 'Refresh the triggers first' : undefined,});
            triggerEditId ? 'Save trigger' : 'Add trigger';
           }
         }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`bar`,});
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":newTrigger,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
       }
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`triggers`,"variant":`compact`,"loading":triggersLoading,"error":swarm.triggersError,"empty":swarm.triggers.length === 0,"onretry":() => detail && void loadTriggers(detail.id),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`comment`,"title":`No triggers`,"body":`Add a trigger to launch swarm work from a Slack/Telegram message or a webhook.`,}});}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
        
         { svelteHTML.createElement("div", { "class":`list`,});
             for(let t of __sveltets_2_ensureArray(swarm.triggers)){t.id;
             { svelteHTML.createElement("div", { "class":`row-item`,});
               { svelteHTML.createElement("div", { "class":`ri-main`,});
                 { svelteHTML.createElement("span", { "class":`chip`,});channelName(t.channel); }
                 { svelteHTML.createElement("span", { "class":`ri-title`,});t.keyword || 'Any message'; }
                 { svelteHTML.createElement("span", { "class":`dim`,});t.match_chat ? `in ${t.match_chat}` : 'any chat'; }
                if(t.auto_start){ { svelteHTML.createElement("span", { "class":`dim`,});  }}
                if(t.reply){ { svelteHTML.createElement("span", { "class":`dim`,});  }}
               }
               { const $$_hctiwS5C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS5C({ target: __sveltets_2_any(), props: {         "checked":t.enabled,"onchange":() => toggleEnabled(t),"label":`Enable trigger “${t.keyword || 'Any message'}”`,"title":t.enabled ? 'Enabled — click to disable' : 'Disabled — click to enable',}});}
               { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":() => editTrigger(t),"aria-label":`Edit ${triggerWhere(t)}`,"title":`Edit ${triggerWhere(t)}`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});} }
               { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":() => delTrigger(t),"aria-label":`Delete ${triggerWhere(t)}`,"title":`Delete ${triggerWhere(t)}`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});} }
             }
          }
         }
       LoadState}
    }
  }
   }

  
 Modal}

if(goalEditorOpen){
   { const $$_rotidElaoG0C = __sveltets_2_ensureComponent(GoalEditor); new $$_rotidElaoG0C({ target: __sveltets_2_any(), props: {       "goal":goalEditIndex >= 0 ? (draftGoals[goalEditIndex] as unknown as SwarmGoal) : null,"onsubmit":onGoalSubmit,"onclose":() => { goalEditorOpen = false; goalEditIndex = -1; },}});}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SwarmSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SwarmSettings__SvelteComponent_ = ReturnType<typeof SwarmSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default SwarmSettings__SvelteComponent_;