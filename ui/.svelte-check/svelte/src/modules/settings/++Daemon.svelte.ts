///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import { settingsSaveErrorText } from './ownerOnly';
import PageBody from '../../lib/components/PageBody.svelte';
// Daemon settings (root): network listener toggle + port, log path display.
import { api } from '../../lib/api/client';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmer } from '../../lib/confirm.svelte';
import Icon from '../../lib/components/Icon.svelte';
import SettingToggle from './SettingToggle.svelte';
import { guardUnsaved } from '../../lib/leaveGuard';
import { DEFAULT_MANUAL_GRACE_SECS, formatGrace, policyFromSettings } from '../../lib/idleSuspend';
import { idleSuspend } from '../../lib/stores/idleSuspend.svelte';
import {
    exemptCandidates,
    readExempt,
    sameExempt,
    sandboxBody,
    type ProcessSandbox,
  } from './sandboxExempt';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  interface NetworkListener {
    enabled: boolean;
    port: number;
  }

  let loading = $state(true);
  // A failed load shows inline with Retry — never the form with its defaults
  // (listener off, sandbox off) dressed up as the real settings, one Save away
  // from writing them back.
  let loadError = $state('');
  let saving = $state(false);
  let enabled = $state(false);
  let port = $state(7700);
  let sandboxEnabled = $state(false);
  let sandboxNetwork = $state<'full' | 'loopback' | 'none'>('full');
  // Providers that run unconfined while the sandbox is on (`exempt_providers`).
  let sandboxExempt = $state<string[]>([]);
  // Last-saved values: the ONE Save (in the header) sends only the groups
  // that differ from these, and is disabled while nothing does.
  let savedListener = $state<NetworkListener>({ enabled: false, port: 7700 });
  // The stored value: keys this form doesn't edit (the legacy `providers`
  // list) survive a save.
  let sandboxStored: Partial<ProcessSandbox> = {};
  let savedSandbox = $state({ enabled: false, network: 'full', exempt: [] as string[] });
  const listenerDirty = $derived(enabled !== savedListener.enabled || port !== savedListener.port);
  const sandboxDirty = $derived(
    sandboxEnabled !== savedSandbox.enabled ||
      sandboxNetwork !== savedSandbox.network ||
      !sameExempt(sandboxExempt, savedSandbox.exempt),
  );
  // Session lifetime: `session_persistence` (sessions you start survive a
  // daemon restart, default on) and `manual_idle_suspend_secs` (quiet time
  // before a session you started is suspended; 0 = never, default 24 h).
  let persistEnabled = $state(true);
  let manualGrace = $state(DEFAULT_MANUAL_GRACE_SECS);
  let savedSessions = $state({ persist: true, manualGrace: DEFAULT_MANUAL_GRACE_SECS });
  const sessionsDirty = $derived(
    persistEnabled !== savedSessions.persist || manualGrace !== savedSessions.manualGrace,
  );
  /** Presets for the manual idle grace (seconds; 0 = never). A value set
   *  elsewhere (the API) that matches none is offered as-is. */
  const GRACE_PRESETS = [1800, 3600, 4 * 3600, 8 * 3600, 86_400, 3 * 86_400, 7 * 86_400, 0];
  const graceOptions = $derived(
    GRACE_PRESETS.includes(manualGrace) ? GRACE_PRESETS : [...GRACE_PRESETS.slice(0, -1), manualGrace, 0],
  );
  function graceLabel(secs: number): string {
    if (secs === 0) return 'Never';
    return secs === DEFAULT_MANUAL_GRACE_SECS ? `${formatGrace(secs)} (default)` : formatGrace(secs);
  }
  const dirty = $derived(listenerDirty || sandboxDirty || sessionsDirty);
  // The inputs' min/max are advisory only; an out-of-range or empty port would
  // be saved as-is (and an empty one silently falls back to the loopback port).
  const portValid = $derived(Number.isInteger(port) && port >= 1024 && port <= 65535);
  const portError = $derived(enabled && !portValid ? 'Enter a whole number from 1024 to 65535.' : '');
  // Latest full settings object (the PUT response). Saves send ONLY the keys
  // they change: PUT /settings upserts exactly the keys in the body, so
  // spreading this page-load snapshot reverted keys written since (auto-update
  // last-run, MCP/PR-review settings, another window) and wrote false
  // skip-permissions / network-listener audit entries on every save.
  let allSettings: Record<string, unknown> = $state({});
  const exemptOptions = $derived(
    exemptCandidates(auth.meta?.providers ?? [], allSettings['providers'], sandboxExempt),
  );
  function setExempt(provider: string, unconfined: boolean): void {
    sandboxExempt = unconfined
      ? [...new Set([...sandboxExempt, provider])].sort((a, b) => a.localeCompare(b))
      : sandboxExempt.filter((p) => p !== provider);
  }

  $effect(() => {
    void load();
  });
  // An un-Saved listener/sandbox change would be dropped silently on leave.
  $effect(() => guardUnsaved(() => dirty, { what: 'the daemon settings' }));

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      allSettings = await api.get<Record<string, unknown>>('/settings');
      const nl = allSettings['network_listener'] as NetworkListener | undefined;
      if (nl) {
        enabled = nl.enabled;
        port = nl.port;
      }
      savedListener = { enabled, port };
      const sb = allSettings['process_sandbox'] as ProcessSandbox | undefined;
      sandboxStored = sb ? { ...sb } : {};
      if (sb) {
        sandboxEnabled = sb.enabled;
        sandboxNetwork = sb.network ?? 'full';
      }
      sandboxExempt = readExempt(sb);
      savedSandbox = { enabled: sandboxEnabled, network: sandboxNetwork, exempt: sandboxExempt };
      // Unset = the daemon default (on); only an explicit `false` turns it off.
      persistEnabled = allSettings['session_persistence'] !== false;
      manualGrace = policyFromSettings(allSettings).manualGraceSecs;
      savedSessions = { persist: persistEnabled, manualGrace };
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function save(): Promise<void> {
    if (!dirty || portError) return;
    // Exposing the daemon beyond this Mac is outward-facing: say where it
    // goes and who can reach it before writing it.
    if (
      listenerDirty &&
      enabled &&
      !savedListener.enabled &&
      !(await confirmer.ask(
        `After the daemon restarts, Otto's login page is served on https://0.0.0.0:${port} — anyone on your network can reach it. Only do this on a trusted network.`,
        { title: 'Expose Otto on your network?', confirmLabel: 'Enable listener', danger: true },
      ))
    )
      return;
    // Only the changed groups: an unchanged network_listener in the body would
    // still write a network-listener audit entry.
    const body: Record<string, unknown> = {};
    if (listenerDirty) body.network_listener = { enabled, port };
    const sandboxValue = sandboxBody(sandboxStored, sandboxEnabled, sandboxNetwork, sandboxExempt);
    if (sandboxDirty) body.process_sandbox = sandboxValue;
    if (sessionsDirty) {
      if (persistEnabled !== savedSessions.persist) body.session_persistence = persistEnabled;
      if (manualGrace !== savedSessions.manualGrace) body.manual_idle_suspend_secs = manualGrace;
    }
    const saveListener = listenerDirty;
    const saveSandbox = sandboxDirty;
    const saveSessions = sessionsDirty;
    const submittedListener = { enabled, port };
    const submittedSandbox = { enabled: sandboxEnabled, network: sandboxNetwork, exempt: [...sandboxExempt] };
    const submittedSessions = { persist: persistEnabled, manualGrace };
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', body);
      const notes: string[] = [];
      if (saveListener) {
        savedListener = submittedListener;
        if (auth.meta) auth.meta.network_listener = submittedListener.enabled;
        // The listener is bound once at daemon start — say so rather than
        // claim a socket that isn't open yet.
        notes.push(
          submittedListener.enabled
            ? `https://0.0.0.0:${submittedListener.port} after the daemon restarts`
            : 'Loopback only after the daemon restarts',
        );
      }
      if (saveSandbox) {
        savedSandbox = submittedSandbox;
        sandboxStored = sandboxValue;
        const unconfined = submittedSandbox.exempt.length;
        notes.push(
          submittedSandbox.enabled
            ? `New sessions confined (network: ${submittedSandbox.network})` +
                (unconfined ? `, except ${submittedSandbox.exempt.join(', ')}` : '')
            : 'Sandbox off for new sessions',
        );
      }
      if (saveSessions) {
        savedSessions = submittedSessions;
        // The Agents panes' "suspends in …" hint reads this policy.
        idleSuspend.policy = policyFromSettings(allSettings);
        notes.push(
          submittedSessions.persist
            ? 'New sessions you start keep running across daemon restarts'
            : 'New sessions stop when the daemon restarts',
        );
      }
      toasts.success('Daemon settings saved', notes.join(' · '));
    } catch (e) {
      toasts.error('Couldn’t save daemon settings', settingsSaveErrorText(e, loadErrorText));
    } finally {
      saving = false;
    }
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (dirty) announce('Unsaved changes — Save to apply them.'); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('daemon'),"subtitle":`ottod ${auth.meta?.version ?? ''} · API v${auth.meta?.api_version ?? 1}`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(!loading && !loadError){
         { svelteHTML.createElement("button", {         "class":`btn small primary`,"disabled":!dirty || saving || !!portError,"title":portError || (dirty ? 'Save the daemon settings' : 'No changes to save'),"onclick":() => void save(),});
          saving ? 'Saving…' : 'Save';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});

  if(loading){
     { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":3,"height":40,}});}
  } else if (loadError){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {      "what":`daemon settings`,"error":loadError,"empty":true,"onretry":() => void load(),}});}
  }else{
    if(dirty){ { svelteHTML.createElement("p", { "class":`unsaved`,});       }}
     { svelteHTML.createElement("h2", { "class":`section-title first`,});  }
     { svelteHTML.createElement("div", { "class":`card pad dm-card`,});
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"label":`Enable network listener (binds 0.0.0.0)`,"checked":enabled,"onchange":(v) => { enabled = v; },}});
                     
            
       SettingToggle}
      if(enabled){
         { svelteHTML.createElement("p", { "class":`warn-note indent`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}               }
      }
       { svelteHTML.createElement("div", { "class":`field port indent`,});
         { svelteHTML.createElement("label", { "for":`dm-port`,});  }
         { svelteHTML.createElement("input", {                     "id":`dm-port`,"class":`input mono`,"type":`number`,"min":`1024`,"max":`65535`,"bind:value":port,"disabled":!enabled,"title":enabled ? undefined : 'Enable the network listener to change its port',"aria-invalid":!!portError,"aria-describedby":portError ? 'dm-port-err' : undefined,});/*Ωignore_startΩ*/() => port = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(portError){ { svelteHTML.createElement("span", {     "class":`hint port-error`,"id":`dm-port-err`,"role":`alert`,});portError; }}
       }
     }

     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", { "class":`card pad dm-card`,});
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"label":`Confine agent sessions with the OS sandbox (macOS Seatbelt)`,"checked":sandboxEnabled,"testid":`sandbox-enabled`,"onchange":(v) => { sandboxEnabled = v; },}});
                      
                        
                     
                   
               
       SettingToggle}
       { svelteHTML.createElement("div", { "class":`field net indent`,});
         { svelteHTML.createElement("label", { "for":`dm-sandbox-net`,});  }
         { svelteHTML.createElement("select", {             "id":`dm-sandbox-net`,"class":`input`,"bind:value":sandboxNetwork,"disabled":!sandboxEnabled,"title":sandboxEnabled ? undefined : 'Turn on the sandbox to choose its network access',...__sveltets_2_empty({"data-testid":`sandbox-network`}),});/*Ωignore_startΩ*/() => sandboxNetwork = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`full`,});      }
           { svelteHTML.createElement("option", { "value":`loopback`,});  }
           { svelteHTML.createElement("option", { "value":`none`,});  }
         }
       }
      if(sandboxEnabled && sandboxNetwork !== 'full'){
         { svelteHTML.createElement("p", { "class":`warn-note indent`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}                   }
      }
      if(exemptOptions.length > 0){
         { svelteHTML.createElement("fieldset", {   "class":`exempt indent`,...__sveltets_2_empty({"data-testid":`sandbox-exempt`}),});
           { svelteHTML.createElement("legend", {});  }
           { svelteHTML.createElement("p", { "class":`hint-line`,});
                         
                    
           }
             for(let p of __sveltets_2_ensureArray(exemptOptions)){p;
             { const $$_elggoTgnitteS4C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS4C({ target: __sveltets_2_any(), props: {             "label":p,"checked":sandboxExempt.includes(p),"disabled":!sandboxEnabled,"title":sandboxEnabled ? undefined : 'Turn on the sandbox to choose which providers it skips',"testid":`sandbox-exempt-${p}`,"onchange":(v) => setExempt(p, v),}});}
          }
         }
      }
     }

     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", { "class":`card pad dm-card`,});
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"label":`Keep sessions running when the daemon restarts`,"checked":persistEnabled,"testid":`session-persistence`,"onchange":(v) => { persistEnabled = v; },}});
                        
                    
                     
                   
                   
        
       SettingToggle}
       { svelteHTML.createElement("div", { "class":`field grace`,});
         { svelteHTML.createElement("label", { "for":`dm-manual-grace`,});      }
         { svelteHTML.createElement("select", {         "id":`dm-manual-grace`,"class":`input`,"bind:value":manualGrace,...__sveltets_2_empty({"data-testid":`manual-idle-grace`}),});/*Ωignore_startΩ*/() => manualGrace = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let secs of __sveltets_2_ensureArray(graceOptions)){secs;
             { svelteHTML.createElement("option", { "value":secs,});graceLabel(secs); }
          }
         }
       }
       { svelteHTML.createElement("p", { "class":`hint-line`,});
                       
                     
             
       }
     }

     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", { "class":`card pad dm-card logs`,});
       { svelteHTML.createElement("div", { "class":`log-line`,});
         { svelteHTML.createElement("span", { "class":`dim`,});  }
         { svelteHTML.createElement("span", {   "class":`mono path`,"title":`~/Library/Logs/Otto/ottod.log.YYYY-MM-DD`,});  }
       }
       { svelteHTML.createElement("p", { "class":`hint-line`,});         }
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('settings/logs'),});   } }
     }
  }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Daemon__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Daemon__SvelteComponent_ = ReturnType<typeof Daemon__SvelteComponent_>;
/*Ωignore_endΩ*/export default Daemon__SvelteComponent_;