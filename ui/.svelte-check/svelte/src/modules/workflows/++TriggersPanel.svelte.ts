///<reference types="svelte" />
;// Triggers configuration panel: list, add, toggle and delete workflow
// triggers (schedule / webhook / event).  Shown in the workflow inspector
// sidebar when the "Triggers" tab is active.

import { onDestroy } from 'svelte';
import { toastError } from '../../lib/toastError';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Switch from '../../lib/components/Switch.svelte';
import { loadErrorText } from '../../lib/loadError';
import { api, baseUrl } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { WorkflowTrigger, TriggerKind } from '../../lib/api/types';
import { buildTriggerSpec, defaultTriggerForm, formFromTrigger, EVENT_KINDS } from './triggerForm';
import { copyTextOrThrow } from '../../lib/clipboard';
import { cadenceLabel } from '../../lib/cadence';
;

  interface Props {
    workflowId: string;
    workflowName?: string;
    triggers: WorkflowTrigger[];
    ontriggers?: (ts: WorkflowTrigger[]) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  


  let { workflowId, workflowName = '', triggers = $bindable([]), ontriggers }: Props = $props()/*Ωignore_startΩ*/;triggers;/*Ωignore_endΩ*/;

  // A copy-paste chat (Slack/Telegram) message that triggers THIS workflow by
  // name. Name is pre-filled; the rest are placeholders the user fills in. Copy
  // it, paste it into a channel where the bot lives, and pin it for reuse. The
  // bot mention is per-workspace (set in the Slack/Telegram client), so it's a
  // placeholder here — the parser keys off `Action: Workflow`, not the mention.
  const chatSnippet = $derived(
    `@<your-bot>\n` +
      `Action: Workflow\n` +
      `Name: ${workflowName || '<workflow name>'}\n` +
      `Msg: what you want done — instructions for the agents\n` +
      `Jira ticket:\n` +
      `Working Directory: ~/path/to/repo\n` +
      `Branch: <base branch>, create wt from it\n` +
      `Relevant Info: ~/other/repo\n` +
      `Goals:\n` +
      `  - goal 1\n` +
      `  - under 20 minutes`,
  );
  let alive = true;
  onDestroy(() => { alive = false; });
  let copied = $state(false);
  async function copyChat(): Promise<void> {
    try {
      await copyTextOrThrow(chatSnippet);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }

  /** A webhook trigger's full URL — the token in the path IS the credential
   *  (no bearer needed), so it could not be used from the UI before: the row
   *  showed only the token's first 8 characters. */
  function webhookUrl(t: WorkflowTrigger): string | null {
    const tok = t.spec?.token;
    if (typeof tok !== 'string' || !tok) return null;
    return `${baseUrl()}/api/v1/workflows/${encodeURIComponent(workflowId)}/webhook/${encodeURIComponent(tok)}`;
  }

  async function copyWebhook(t: WorkflowTrigger): Promise<void> {
    const url = webhookUrl(t);
    if (!url) return;
    try {
      await copyTextOrThrow(url);
      toasts.success('Webhook URL copied', 'Anyone with this URL can start the workflow — keep it private.');
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }

  let adding = $state(false);
  let form = $state(defaultTriggerForm());
  let editingId = $state<string | null>(null);
  let originalSpec: Record<string, unknown> = {};
  let preview = $state<string[] | null>(null);
  let previewTimezone = $state('UTC');
  let previewing = $state(false);
  function edit(t?: WorkflowTrigger): void {
    editingId = t?.id ?? null;
    originalSpec = t ? { ...(t.spec as Record<string, unknown>) } : {};
    form = t ? formFromTrigger(t) : defaultTriggerForm();
    preview = null;
    adding = true;
  }
  async function previewTrigger(): Promise<void> {
    previewing = true;
    const timezone = form.timezone;
    try {
      const result = await api.post<{ next_fire_times: string[] }>(`/workflows/${workflowId}/triggers/preview`, { kind: form.kind, spec: buildSpec() });
      if (!alive) return;
      previewTimezone = timezone;
      preview = result.next_fire_times;
    } catch (e) { toastError('Couldn’t preview the trigger', e); }
    finally { previewing = false; }
  }

  // A preview describes the form as it WAS: any later edit hides it, so a stale
  // "Next fires" list never sits under a changed cadence/timezone.
  $effect(() => {
    JSON.stringify(form);
    preview = null;
  });

  let saving = $state(false);

  // A failed load is shown inline with Retry (never as "No triggers", which
  // would claim the workflow only runs manually when we simply don't know).
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  async function load(): Promise<void> {
    loading = true;
    try {
      const ts = await api.get<WorkflowTrigger[]>(`/workflows/${workflowId}/triggers`);
      if (!alive) return;
      triggers = ts;
      loadError = null;
      ontriggers?.(ts);
    } catch (e) {
      if (alive) loadError = loadErrorText(e);
    } finally {
      if (alive) loading = false;
    }
  }

  const KIND_LABEL: Record<string, string> = { schedule: 'Schedule', webhook: 'Webhook', event: 'Event', chat: 'Chat binding' };

  $effect(() => {
    if (workflowId) void load();
  });

  function buildSpec(): Record<string, unknown> { return buildTriggerSpec(form, originalSpec); }

  async function addTrigger(): Promise<void> {
    if (saving) return;
    saving = true;
    try {
      const t = editingId
        ? await api.patch<WorkflowTrigger>(`/workflow-triggers/${editingId}`, { spec: buildSpec() })
        : await api.post<WorkflowTrigger>(`/workflows/${workflowId}/triggers`, { kind: form.kind, spec: buildSpec(), enabled: true });
      if (!alive) return;
      triggers = editingId ? triggers.map((old) => old.id === t.id ? t : old) : [...triggers, t];
      ontriggers?.(triggers);
      adding = false;
      toasts.success(editingId ? 'Trigger updated' : 'Trigger added');
    } catch (e) {
      toastError(editingId ? 'Couldn’t update the trigger' : 'Couldn’t add the trigger', e);
    } finally {
      saving = false;
    }
  }

  async function toggle(t: WorkflowTrigger): Promise<void> {
    try {
      const updated = await api.patch<WorkflowTrigger>(`/workflow-triggers/${t.id}`, {
        enabled: !t.enabled,
      });
      if (!alive) return;
      triggers = triggers.map((x) => (x.id === t.id ? updated : x));
      ontriggers?.(triggers);
    } catch (e) {
      toastError('Couldn’t update the trigger', e);
    }
  }

  // Deleting is irreversible — a webhook trigger's token goes with it, so any
  // external caller breaks — so it asks first, like deleting the workflow.
  // (Pausing is the reversible option: the on/off toggle.)
  async function remove(t: WorkflowTrigger): Promise<void> {
    const ok = await confirmer.ask(
      `Delete this ${(KIND_LABEL[t.kind] ?? t.kind).toLowerCase()} trigger (${describeSpec(t)})?${t.kind === 'webhook' ? ' Its webhook URL stops working.' : ''} To pause it instead, switch it off.`,
      { title: 'Delete trigger', danger: true, confirmLabel: 'Delete trigger' },
    );
    if (!ok) return;
    try {
      await api.del(`/workflow-triggers/${t.id}`);
      if (!alive) return;
      triggers = triggers.filter((x) => x.id !== t.id);
      ontriggers?.(triggers);
      toasts.success('Trigger deleted');
    } catch (e) {
      toastError('Couldn’t delete the trigger', e);
    }
  }

  function describeSpec(t: WorkflowTrigger): string {
    const s = t.spec as Record<string, unknown>;
    if (t.kind === 'schedule') {
      const c = s.cadence as string | undefined;
      // The shared label (ux-design-2-04): "Every day", not "every 1440 min".
      if (c && ['interval', 'daily', 'weekly', 'cron', 'once'].includes(c)) {
        return cadenceLabel(s, typeof s.timezone === 'string' ? s.timezone : 'UTC');
      }
      return 'Custom schedule';
    }
    if (t.kind === 'webhook') {
      const tok = s.token as string | undefined;
      return tok ? `token: ${tok.slice(0, 8)}…` : 'token pending';
    }
    if (t.kind === 'event') return `on ${s.event_kind ?? 'event'}`;
    if (t.kind === 'chat') {
      const ch = (s.channel as string | undefined) ?? '?';
      const chat = (s.chat as string | undefined) ?? '?';
      const thread = s.thread ? ` · thread ${s.thread}` : '';
      const mention = s.mention_only ? ' · @mention only' : '';
      return `${ch}: ${chat}${thread}${mention}`;
    }
    return '';
  }

  function kindIcon(kind: TriggerKind): IconName {
    if (kind === 'schedule') return 'clock';
    if (kind === 'webhook') return 'zap';
    if (kind === 'chat') return 'comment';
    return 'bell';
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`tp`,});
   { svelteHTML.createElement("div", { "class":`tp-head`,});
     { svelteHTML.createElement("span", { "class":`tp-title`,});  }
     { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => edit(),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} 
     }
   }

  if(adding){
     { svelteHTML.createElement("div", { "class":`add-form`,});
       { svelteHTML.createElement("label", { "class":`fl`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", {       "bind:value":form.kind,"disabled":editingId !== null,"title":editingId !== null ? 'Kind can’t be changed. Delete this trigger and add a new one.' : 'Trigger kind',});/*Ωignore_startΩ*/() => form.kind = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`schedule`,});  }
           { svelteHTML.createElement("option", { "value":`webhook`,});  }
           { svelteHTML.createElement("option", { "value":`event`,});  }
           { svelteHTML.createElement("option", { "value":`chat`,});  }
         }
       }

      if(form.kind === 'schedule'){
         { svelteHTML.createElement("label", { "class":`fl`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", { "bind:value":form.cadence,});/*Ωignore_startΩ*/() => form.cadence = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`interval`,});  }
             { svelteHTML.createElement("option", { "value":`daily`,});  }
             { svelteHTML.createElement("option", { "value":`weekly`,});  }
             { svelteHTML.createElement("option", { "value":`cron`,});  }
           }
         }
        if(form.cadence === 'interval'){
           { svelteHTML.createElement("label", { "class":`fl`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {        "type":`number`,"min":`1`,"max":`10080`,"bind:value":form.everyMin,});/*Ωignore_startΩ*/() => form.everyMin = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
        } else if (form.cadence === 'cron'){
           { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});   } { svelteHTML.createElement("input", {      "dir":`ltr`,"bind:value":form.cron,"placeholder":`0 9 * * 1-5`,});/*Ωignore_startΩ*/() => form.cron = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
        }else{
           { svelteHTML.createElement("label", { "class":`fl`,});
             { svelteHTML.createElement("span", {});      }
             { svelteHTML.createElement("input", {        "dir":`ltr`,"type":`text`,"placeholder":`09:00`,"bind:value":form.atTime,});/*Ωignore_startΩ*/() => form.atTime = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
          if(form.cadence === 'weekly'){
             { svelteHTML.createElement("label", { "class":`fl`,});
               { svelteHTML.createElement("span", {});  }
               { svelteHTML.createElement("select", { "bind:value":form.weekday,});/*Ωignore_startΩ*/() => form.weekday = __sveltets_2_any(null);/*Ωignore_endΩ*/
                    for(let d of __sveltets_2_ensureArray((['Mon','Tue','Wed','Thu','Fri','Sat','Sun']))){let i = 1;i;
                   { svelteHTML.createElement("option", { "value":i,});d; }
                }
               }
             }
          }
        }
         { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":form.timezone,"placeholder":`Asia/Jerusalem`,});/*Ωignore_startΩ*/() => form.timezone = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("textarea", {     "dir":`auto`,"bind:value":form.prompt,"rows":3,});/*Ωignore_startΩ*/() => form.prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
      } else if (form.kind === 'event'){
         { svelteHTML.createElement("label", { "class":`fl`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", { "bind:value":form.eventKind,});/*Ωignore_startΩ*/() => form.eventKind = __sveltets_2_any(null);/*Ωignore_endΩ*/   for(let [value, label] of __sveltets_2_ensureArray(EVENT_KINDS)){value; { svelteHTML.createElement("option", {value,});label; }} }
         }
         { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});   } { svelteHTML.createElement("textarea", {     "dir":`ltr`,"rows":3,"bind:value":form.filter,});/*Ωignore_startΩ*/() => form.filter = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
         { svelteHTML.createElement("p", { "class":`hint`,});       }
      } else if (form.kind === 'chat'){
         { svelteHTML.createElement("label", { "class":`fl`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", { "bind:value":form.chatChannel,});/*Ωignore_startΩ*/() => form.chatChannel = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`slack`,});  }
             { svelteHTML.createElement("option", { "value":`telegram`,});  }
           }
         }
         { svelteHTML.createElement("label", { "class":`fl`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {        "dir":`auto`,"type":`text`,"placeholder":`C0123456 (Slack) / -100987654321 (Telegram)`,"bind:value":form.chatId,});/*Ωignore_startΩ*/() => form.chatId = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", { "class":`fl`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {        "dir":`auto`,"type":`text`,"placeholder":`thread ts — leave blank to match any thread`,"bind:value":form.chatThread,});/*Ωignore_startΩ*/() => form.chatThread = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("label", {  "class":`chk-row`,});form.chatChannel === 'telegram';
           { svelteHTML.createElement("input", {       "type":`checkbox`,"bind:checked":form.chatMentionOnly,"disabled":form.chatChannel === 'telegram',});/*Ωignore_startΩ*/() => form.chatMentionOnly = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {});     form.chatChannel === 'telegram' ? ' (Slack only)' : ''; }
         }
         { svelteHTML.createElement("p", { "class":`hint`,});            }
      }else{
         { svelteHTML.createElement("p", { "class":`hint`,});
                
           { svelteHTML.createElement("code", {}); workflowId;  }
                 
         }
      }

       { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});   } { svelteHTML.createElement("select", { "bind:value":form.resultChannel,});/*Ωignore_startΩ*/() => form.resultChannel = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",});   } { svelteHTML.createElement("option", { "value":`slack`,});  } { svelteHTML.createElement("option", { "value":`telegram`,});  } } }
      if(form.resultChannel){
         { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});    } { svelteHTML.createElement("input", {    "dir":`auto`,"bind:value":form.resultChat,});/*Ωignore_startΩ*/() => form.resultChat = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {    "dir":`auto`,"bind:value":form.resultThread,});/*Ωignore_startΩ*/() => form.resultThread = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
      }
       { svelteHTML.createElement("label", { "class":`fl`,}); { svelteHTML.createElement("span", {});   } { svelteHTML.createElement("input", {        "dir":`ltr`,"type":`url`,"bind:value":form.resultWebhook,"placeholder":`https://example.com/result`,});/*Ωignore_startΩ*/() => form.resultWebhook = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":previewing,"onclick":previewTrigger,});previewing ? 'Checking…' : 'Preview / validate'; }
      if(preview){
         { svelteHTML.createElement("div", { "class":`hint`,});if(preview.length){  previewTimezone;  { svelteHTML.createElement("ul", {});  for(let at of __sveltets_2_ensureArray(preview)){ { svelteHTML.createElement("li", {});new Date(at).toLocaleString(undefined, { timeZone: previewTimezone }); }} }}else{   } }
      }
       { svelteHTML.createElement("div", { "class":`add-btns`,});
         { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => (adding = false),});  }
         { svelteHTML.createElement("button", {     "class":`btn primary small`,"disabled":saving,"onclick":addTrigger,});
          saving ? 'Saving…' : editingId ? 'Save trigger' : 'Add trigger';
         }
       }
     }
  }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"what":`triggers`,"variant":`compact`,loading,"error":loadError,"empty":triggers.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
      if(!adding){ { svelteHTML.createElement("p", { "class":`empty`,});        }}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     for(let t of __sveltets_2_ensureArray(triggers)){t.id;
     { svelteHTML.createElement("div", {  "class":`trig-row`,});!t.enabled;
       { svelteHTML.createElement("span", { "class":`trig-ic`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":kindIcon(t.kind),"size":13,}});} }
       { svelteHTML.createElement("div", { "class":`trig-body`,});
         { svelteHTML.createElement("span", { "class":`trig-kind`,});KIND_LABEL[t.kind] ?? t.kind; }
         { svelteHTML.createElement("span", {   "class":`trig-spec`,"title":describeSpec(t),});describeSpec(t); }
       }
       { const $$_hctiwS3C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS3C({ target: __sveltets_2_any(), props: {           "checked":t.enabled,"onchange":() => toggle(t),"tone":`success`,"label":`Enable ${KIND_LABEL[t.kind] ?? t.kind} trigger`,"title":t.enabled ? 'On — click to pause this trigger' : 'Off — click to enable this trigger',}});}
      if(t.kind === 'webhook' && webhookUrl(t)){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Copy the webhook URL (POST a JSON body to start a run)`,"aria-label":`Copy webhook URL`,"onclick":() => copyWebhook(t),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
      }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Edit trigger`,"aria-label":`Edit trigger`,"onclick":() => edit(t),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} }
       { svelteHTML.createElement("button", {       "class":`row-del`,"title":`Delete trigger…`,"aria-label":`Delete trigger…`,"onclick":() => remove(t),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
       }
     }
  }
   LoadState}

  
   { svelteHTML.createElement("div", { "class":`chat-trig`,});
     { svelteHTML.createElement("div", { "class":`st-head`,});
       { svelteHTML.createElement("span", { "class":`tp-title`,});   }
       { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":copyChat,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":copied ? 'check' : 'copy',"size":12,}});} copied ? 'Copied' : 'Copy';
       }
     }
     { svelteHTML.createElement("p", { "class":`st-hint`,});
                  
          { svelteHTML.createElement("strong", {});  }       
         { svelteHTML.createElement("strong", {});  }    
     }
     { svelteHTML.createElement("pre", { "class":`st-snip`,});chatSnippet; }
     { svelteHTML.createElement("p", { "class":`st-hint`,});
        { svelteHTML.createElement("code", {});  }      
       { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }    
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('triggers'), slots: {}, events: {} }}
const TriggersPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TriggersPanel__SvelteComponent_ = ReturnType<typeof TriggersPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default TriggersPanel__SvelteComponent_;