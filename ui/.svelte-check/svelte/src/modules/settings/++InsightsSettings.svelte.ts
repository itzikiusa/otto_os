///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { toastError } from '../../lib/toastError';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
import SectionIntro from './SectionIntro.svelte';
import SettingToggle from './SettingToggle.svelte';
// Settings → Insights: opt-in toggles for scheduled HTML insight reports.
// All three are OFF by default. Runs are catch-up — if the app was closed at
// the scheduled time, the report is generated the next time the app is open,
// so a scheduled report is never silently missed.
import { insightsApi } from '../../lib/api/insights';
import type { InsightsConfig } from '../../lib/api/types';
import { router } from '../../lib/router.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { contextApi } from '../../lib/api/context';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let cfg: InsightsConfig | null = $state(null);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  // Quiet inline confirmation for the report-agent fields (the checkboxes
  // show their own state; toasting every change was noise).
  // One quiet "Saved" per section (a toggle or select applies immediately —
  // layout.md §4.4), next to the heading of the section that changed.
  let savedIn = $state<'schedule' | 'agent' | null>(null);
  let savedTimer: ReturnType<typeof setTimeout> | null = null;
  function flashSaved(where: 'schedule' | 'agent'): void {
    savedIn = where;
    // a11y-6-01: announced at the state edge, not by the inserted tick.
    announce('Saved');
    if (savedTimer) clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (savedIn = null), 2500);
  }

  // Reports are written by the bundled `insights` skill: say whether it's
  // installed instead of only telling people it's required (a schedule with
  // the skill missing produced nothing, silently). Unknown (no access to the
  // bundled list) = say nothing extra.
  let skillState = $state<'installed' | 'missing' | null>(null);
  $effect(() => {
    void contextApi
      .listBundled()
      .then((list) => {
        const sk = list.find((x) => x.name === 'insights');
        skillState = sk ? (sk.state === 'not_installed' ? 'missing' : 'installed') : null;
      })
      .catch(() => (skillState = null));
  });
  const anyOn = $derived.by(() => {
    const c = cfg as InsightsConfig | null;
    return !!c && (c.daily || c.weekly || c.monthly);
  });
  // A provider/model change made while another save is in flight is queued
  // (merged) and sent right after — it used to be dropped while the picker
  // kept showing it, so the reports ran on the old model.
  let queuedAgent: Partial<InsightsConfig> | null = null;
  let failedAgent: Partial<InsightsConfig> | null = $state(null);
  let agentSaveError = $state('');
  // Local model draft — the picker's free-text path fires per keystroke, so we
  // debounce the PUT instead of racing saveAgent's `saving` guard.
  let modelDraft = $state('');
  let modelSaveTimer: ReturnType<typeof setTimeout> | null = $state(null);
  function onModelChange(m: string): void {
    // The draft belongs to the picker that produced it, even if a pending
    // provider write later rolls cfg back to the last acknowledged pair.
    const provider = failedAgent?.provider ?? cfg?.provider ?? '';
    modelDraft = m;
    savedIn = null;
    if (modelSaveTimer) clearTimeout(modelSaveTimer);
    modelSaveTimer = setTimeout(() => { modelSaveTimer = null; void saveAgent({ provider, model: m }, true); }, 500);
  }

  // ---------------------------------------------------------------------------
  // Load on mount
  // ---------------------------------------------------------------------------

  $effect(() => {
    void load();
  });

  let loaded = false;
  async function load(): Promise<void> {
    if (loaded) return;
    loaded = true;
    loading = true;
    loadError = '';
    try {
      cfg = await insightsApi.getConfig();
      modelDraft = cfg.model || '';
    } catch (e) {
      loaded = false; // let Retry load again
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // PUT on every toggle change (optimistic + revert on failure)
  // ---------------------------------------------------------------------------

  /** Persist a provider/model change (which agent generates the reports). */
  async function saveAgent(patch: Partial<InsightsConfig>, preserveDraft = false): Promise<void> {
    if (!cfg) return;
    if ('provider' in patch && !preserveDraft) {
      if (modelSaveTimer) clearTimeout(modelSaveTimer);
      modelSaveTimer = null;
      modelDraft = patch.model ?? '';
    }
    if (saving) {
      queuedAgent = { ...(queuedAgent ?? {}), ...patch };
      return;
    }
    const next: InsightsConfig = { ...cfg, ...patch };
    const prev = cfg;
    cfg = next;
    failedAgent = null;
    saving = true;
    try {
      cfg = await insightsApi.putConfig(next);
      failedAgent = null;
      agentSaveError = '';
      if (!queuedAgent && !modelSaveTimer) flashSaved('agent');
    } catch (e) {
      cfg = prev;
      // Keep the newest coherent pair for explicit Retry. Do not drain a
      // provider's queued model into the provider restored by rollback.
      failedAgent = { provider: next.provider, ...queuedAgent, model: modelDraft };
      queuedAgent = null;
      if (modelSaveTimer) clearTimeout(modelSaveTimer);
      modelSaveTimer = null;
      agentSaveError = loadErrorText(e);
      toastError('Couldn’t save the report agent', e);
    } finally {
      saving = false;
    }
    await drainQueuedAgent();
  }

  async function drainQueuedAgent(): Promise<void> {
    if (!queuedAgent) return;
    const patch = queuedAgent;
    queuedAgent = null;
    await saveAgent(patch, true);
  }

  async function retryAgent(): Promise<void> {
    if (failedAgent && !saving && !modelSaveTimer) await saveAgent(failedAgent, true);
  }

  async function toggle(key: keyof InsightsConfig): Promise<void> {
    if (!cfg || saving) return;
    const next: InsightsConfig = { ...cfg, [key]: !cfg[key] };
    const prev = cfg;
    cfg = next; // optimistic
    saving = true;
    try {
      cfg = await insightsApi.putConfig(next);
      flashSaved('schedule');
    } catch (e) {
      cfg = prev; // revert
      toastError(`Couldn’t turn ${String(key)} reports ${next[key] ? 'on' : 'off'}`, e);
    } finally {
      saving = false;
    }
    await drainQueuedAgent();
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('insights'),"subtitle":`Scheduled HTML reports about your Otto activity`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});   { svelteHTML.createElement("strong", {});  }   { svelteHTML.createElement("strong", {});   }            { svelteHTML.createElement("strong", {});  }                            SectionIntro}

  if(!cfg){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`insights settings`,"loading":loading || !loadError,"error":loadError || null,"empty":true,"rows":3,"onretry":() => void load(),}});}
  }else{
    if(skillState === 'missing'){
       { svelteHTML.createElement("p", {   "class":`skill-warn`,"role":anyOn ? 'alert' : undefined,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
         { svelteHTML.createElement("span", {});  { svelteHTML.createElement("span", { "class":`mono`,});  }     anyOn ? 'scheduled reports can’t be generated' : 'reports can’t be generated yet';  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('settings/skills'),});  }
       }
    }
     { svelteHTML.createElement("h2", { "class":`section-title`,}); if(savedIn === 'schedule'){ { svelteHTML.createElement("span", { "class":`saved`,});  }} }
     { svelteHTML.createElement("div", { "class":`card toggles`,});
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {          "label":`Daily`,"hint":`Covers the previous day (UTC), generated once it has ended.`,"checked":cfg.daily,"disabled":saving,"onchange":() => toggle('daily'),}});}
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {          "label":`Weekly`,"hint":`Runs on Monday (UTC), covering the previous Monday–Sunday.`,"checked":cfg.weekly,"disabled":saving,"onchange":() => toggle('weekly'),}});}
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {          "label":`Monthly`,"hint":`Runs on the 1st (UTC), covering the previous month.`,"checked":cfg.monthly,"disabled":saving,"onchange":() => toggle('monthly'),}});}
     }

     { svelteHTML.createElement("h2", { "class":`section-title`,});  if(savedIn === 'agent'){ { svelteHTML.createElement("span", { "class":`saved`,});  }} }
    if(agentSaveError){
       { svelteHTML.createElement("p", { "role":`alert`,});agentSaveError;  { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":saving || !!modelSaveTimer,"onclick":retryAgent,});    } }
    }
     { svelteHTML.createElement("div", { "class":`card agent-row`,});
       { svelteHTML.createElement("div", { "class":`field agent-field`,});
         { svelteHTML.createElement("label", { "for":`ins-provider`,});  }
         { svelteHTML.createElement("select", {           "id":`ins-provider`,"class":`input`,"value":(failedAgent?.provider ?? cfg.provider) || '',"disabled":saving,"onchange":(e) => {
            // A model belongs to its provider — drop it rather than send a
            // Claude model id to codex.
            modelDraft = '';
            void saveAgent({ provider: e.currentTarget.value, model: '' });
          },});
           { svelteHTML.createElement("option", {"value":"",}); defaultAgentProvider();  }
             for(let p of __sveltets_2_ensureArray(agentProviders())){p; { svelteHTML.createElement("option", { "value":p,});p; }}
         }
       }
      
       { svelteHTML.createElement("div", { "class":`agent-field model`,});
         { const $$_rekciPledoM4C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM4C({ target: __sveltets_2_any(), props: {         "provider":(failedAgent?.provider ?? cfg.provider) || defaultAgentProvider(),"value":modelDraft,"hint":`Model the report agent runs with (blank = the provider’s default).`,"onchange":onModelChange,}});}
       }
     }

     { svelteHTML.createElement("p", { "class":`note`,});
      if(skillState === 'installed'){     { svelteHTML.createElement("span", { "class":`mono`,});  } } else if (skillState === null){   { svelteHTML.createElement("span", { "class":`mono`,});  }    
         { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go('settings/skills'),});   } }
         
       { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go('insights'),});  }  
            
     }
  }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const InsightsSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type InsightsSettings__SvelteComponent_ = ReturnType<typeof InsightsSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default InsightsSettings__SvelteComponent_;