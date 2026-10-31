///<reference types="svelte" />
;
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
// Notification preferences: expiry warning threshold + native/session toggles.
// Also exposes the `channels.notify_self_improvement` opt-in flag so the user
// can turn on Slack/Telegram self-improvement pings from one place (T6).
import { notifications } from '../../lib/stores/notifications.svelte';
import { api } from '../../lib/api/client';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { NotificationSettings } from '../../lib/api/types';
import { router } from '../../lib/router.svelte';
import SettingToggle from './SettingToggle.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Load once on mount if the store hasn't fetched yet.
  $effect(() => {
    void notifications.ensureLoaded();
  });

  function save(patch: Partial<NotificationSettings>): void {
    void notifications.saveSettings({ ...notifications.settings, ...patch });
  }

  function onThreshold(e: Event & { currentTarget: HTMLInputElement }): void {
    const el = e.currentTarget;
    const n = Math.round(Number(el.value));
    const cur = notifications.settings.expiry_threshold_days;
    // An empty/invalid entry, or one clamped back to the value already saved,
    // changes no state — so put the saved number back in the box explicitly
    // (otherwise it keeps showing e.g. "45" while 30 is in effect).
    const next = el.value.trim() === '' || !Number.isFinite(n) ? cur : Math.min(30, Math.max(1, n));
    el.value = String(next);
    if (next !== cur) save({ expiry_threshold_days: next });
  }

  // ---------------------------------------------------------------------------
  // channels.notify_self_improvement — persisted in the daemon settings store.
  // The backend `improve_notify` task reads it live from the global SettingsRepo.
  // Root-only (matches GET/PUT /api/v1/settings permission gate).
  // ---------------------------------------------------------------------------
  // Per-event channel notify toggles (all off by default, root-only).
  // ---------------------------------------------------------------------------

  type NotifyFlag = {
    key: string;
    label: string;
    sub: string;
    value: boolean;
    loading: boolean;
  };

  const CHANNEL_NOTIFY_FLAGS: Array<{ key: string; label: string; sub: string }> = [
    {
      key: 'channels.notify_self_improvement',
      label: 'Self-improvement events',
      sub: 'Posts a one-line summary when a run finishes or an approval is pending.',
    },
    {
      key: 'channels.notify_review_done',
      label: 'Code review completed',
      sub: 'Sends a message when a code-review run finishes or fails.',
    },
    {
      key: 'channels.notify_swarm_done',
      label: 'Agent swarm completed',
      sub: 'Sends a message when a swarm run finishes, is aborted, or fails.',
    },
    {
      key: 'channels.notify_insight_ready',
      label: 'Insights report ready',
      sub: 'Sends a message when a daily, weekly or monthly insights report is ready.',
    },
    {
      key: 'channels.notify_budget_exceeded',
      label: 'Budget cap exceeded',
      sub: 'Sends a message when a spend cap is crossed (requires budget enforcement to be on).',
    },
  ];

  // Reactive state for each flag: keyed by the settings key.
  let flagValues: Record<string, boolean> = $state(
    Object.fromEntries(CHANNEL_NOTIFY_FLAGS.map((f) => [f.key, false]))
  );
  let flagLoading: Record<string, boolean> = $state(
    Object.fromEntries(CHANNEL_NOTIFY_FLAGS.map((f) => [f.key, false]))
  );

  $effect(() => {
    if (auth.isRoot) void loadChannelFlags();
  });

  // Set when the flags couldn't be read: the toggles would otherwise all show
  // OFF (the defaults) even if some are on, so they're disabled until a retry.
  let flagsError = $state('');

  async function loadChannelFlags(): Promise<void> {
    for (const f of CHANNEL_NOTIFY_FLAGS) {
      flagLoading[f.key] = true;
    }
    flagsError = '';
    try {
      const all = await api.get<Record<string, unknown>>('/settings');
      for (const f of CHANNEL_NOTIFY_FLAGS) {
        flagValues[f.key] = all?.[f.key] === true;
      }
    } catch (e) {
      flagsError = loadErrorText(e);
    } finally {
      for (const f of CHANNEL_NOTIFY_FLAGS) {
        flagLoading[f.key] = false;
      }
    }
  }

  async function toggleFlag(key: string, checked: boolean): Promise<void> {
    flagValues[key] = checked;
    try {
      await api.put('/settings', { [key]: checked });
    } catch (e) {
      flagValues[key] = !checked; // revert
      toasts.error('Couldn’t save the channel notification', loadErrorText(e));
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('notifications'),"subtitle":`Alerts and credential-expiry warnings`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", { "class":`card s-card`,});
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {             "label":`Native macOS notifications for important alerts`,"hint":`Show system notifications for warnings and errors.`,"checked":notifications.settings.native_enabled,"disabled":!auth.isRoot,"title":auth.isRoot ? undefined : 'These settings apply to the whole Otto daemon — only the root user can change them',"onchange":(v) => save({ native_enabled: v }),}});}
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {             "label":`Banner when a session is waiting on you`,"hint":`A native notification when an agent finishes its turn and waits for input — only for sessions you are not looking at.`,"checked":notifications.settings.native_on_waiting !== false,"disabled":!auth.isRoot || !notifications.settings.native_enabled,"title":!auth.isRoot
        ? 'These settings apply to the whole Otto daemon — only the root user can change them'
        : notifications.settings.native_enabled
          ? undefined
          : 'Turn on native macOS notifications first',"onchange":(v) => save({ native_on_waiting: v }),}});}
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {             "label":`Notify on session events`,"hint":`A heads-up when a session finishes or is waiting for your input.`,"checked":notifications.settings.session_events,"disabled":!auth.isRoot,"title":auth.isRoot ? undefined : 'These settings apply to the whole Otto daemon — only the root user can change them',"onchange":(v) => save({ session_events: v }),}});}
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", { "class":`card s-card`,});
     { svelteHTML.createElement("div", { "class":`field threshold`,});
       { svelteHTML.createElement("label", { "for":`nt-threshold`,});         }
       { svelteHTML.createElement("input", {                 "id":`nt-threshold`,"class":`input num`,"type":`number`,"min":`1`,"max":`30`,"value":notifications.settings.expiry_threshold_days,"disabled":!auth.isRoot,"onchange":onThreshold,});}
       { svelteHTML.createElement("span", { "class":`hint`,});              }
     }
   }

  if(auth.isRoot){
     { svelteHTML.createElement("div", { "class":`section-title`,});  }
     { svelteHTML.createElement("p", { "class":`section-note`,});
                     
       { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go('settings/channels'),});    }     
     }
    if(flagsError){
       { svelteHTML.createElement("div", { "class":`flags-error`,});
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the channel notification settings`,"error":flagsError,"empty":true,"onretry":() => void loadChannelFlags(),}});}
       }
    }
     { svelteHTML.createElement("div", { "class":`card s-card`,});
         for(let flag of __sveltets_2_ensureArray(CHANNEL_NOTIFY_FLAGS)){flag.key;
         { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {             "label":flag.label,"hint":flag.sub,"checked":flagValues[flag.key],"disabled":flagLoading[flag.key] || !!flagsError,"title":flagsError ? 'Couldn’t read the current value — retry above' : undefined,"onchange":(v) => toggleFlag(flag.key, v),}});}
      }
     }
  }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Notifications__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Notifications__SvelteComponent_ = ReturnType<typeof Notifications__SvelteComponent_>;
/*Ωignore_endΩ*/export default Notifications__SvelteComponent_;