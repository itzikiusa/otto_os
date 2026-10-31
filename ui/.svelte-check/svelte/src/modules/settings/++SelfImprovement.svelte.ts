///<reference types="svelte" />
;
import { announce, announceShown, focusAppContent } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import { NO_WORKSPACE } from '../../lib/labels';
import { pollWhileVisible } from '../../lib/poll';
import { latestOnly } from '../../lib/latest';
import { toastError } from '../../lib/toastError';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import { guardUnsaved } from '../../lib/leaveGuard';
import SectionIntro from './SectionIntro.svelte';
import SettingToggle from './SettingToggle.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Self-Improvement settings page: per-workspace scheduled self-reflection.
// Periodically reviews recent sessions and improves the workspace's memory
// and handling skills — safe edits apply automatically, risky ones queue for
// approval here, with a recent-runs log.
import { api } from '../../lib/api/client';
import { improveApi } from '../../lib/api/improve';
import type {
    Autonomy,
    ImprovementEdit,
    ImprovementRun,
    SelfImprovementConfig,
  } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { improvementBus } from '../../lib/events.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import EmptyState from '../../lib/components/EmptyState.svelte';
import DiffView from '../../lib/components/DiffView.svelte';
import Icon from '../../lib/components/Icon.svelte';
import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus, sentenceCase } from '../../lib/status';
import { rel } from '../../lib/stores/now.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let cfg: SelfImprovementConfig | null = $state(null);
  let allowlistText = $state('');
  let runs: ImprovementRun[] = $state([]);
  let pending: ImprovementEdit[] = $state([]);
  let loading = $state(false);
  let loadError = $state('');
  let saving = $state(false);
  // The editable fields as last loaded/saved, and the workspace they belong
  // to. The 30 s poll and every improvement event re-load this page; they
  // used to overwrite `cfg` too, snapping a half-edited form back with no
  // word. Now the form is only refreshed while it has no unsaved edits.
  let savedKey = $state('');
  let cfgWs: string | null = null;
  function formKey(c: SelfImprovementConfig, allow: string): string {
    return JSON.stringify([
      c.enabled,
      c.live_evolve,
      c.cadence_minutes,
      c.lookback_hours,
      c.autonomy,
      [...c.providers].sort(),
      allow.split(',').map((x) => x.trim()).filter(Boolean),
    ]);
  }
  const dirty = $derived(cfg != null && formKey(cfg, allowlistText) !== savedKey);
  // Leaving Settings (or this page) with unsaved edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: 'the self-improvement settings' }));
  // number inputs bind `null` when cleared; the daemon would answer with a raw
  // deserialize error.
  const formError = $derived.by(() => {
    if (!cfg) return '';
    const whole = (n: unknown) => typeof n === 'number' && Number.isInteger(n) && n >= 1;
    if (!whole(cfg.cadence_minutes)) return '“Run every” must be a whole number of minutes (1 or more).';
    if (!whole(cfg.lookback_hours)) return '“Look back” must be a whole number of hours (1 or more).';
    return '';
  });
  let running = $state(false);
  let busyEdit: string | null = $state(null);
  let busyKind: 'approve' | 'deny' | null = $state(null);

  // Evolve-now state: the most recent per-session evolve result.
  let evolving = $state(false);
  // null = never run; [] = ran, no skill changes; [...] = changed skill refs
  let evolveResult: string[] | null = $state(null);

  const wsId = $derived(ws.currentId);
  // The currently-active / focused session in the workspace — used by "Evolve now".
  const activeSession = $derived(ws.activeSession);

  // Agent CLIs the analysis can run on: the live provider registry (built-ins +
  // custom, e.g. grok), minus the `shell` pseudo-provider — NOT /meta.tools,
  // which also lists non-agent tools like git/clickhouse. Anything already
  // selected is kept so a since-removed provider doesn't silently vanish. Each
  // selected provider yields its own set of suggestions.
  const providerChoices = $derived.by(() => {
    const set = new Set<string>(agentProviders());
    for (const p of cfg?.providers ?? []) set.add(p);
    return [...set];
  });

  function toggleProvider(name: string): void {
    if (!cfg) return;
    const set = new Set(cfg.providers);
    if (set.has(name)) set.delete(name);
    else set.add(name);
    // Always keep at least one provider selected.
    cfg.providers = set.size > 0 ? [...set] : [defaultAgentProvider()];
  }

  // ---------------------------------------------------------------------------
  // Load on workspace change
  // ---------------------------------------------------------------------------

  // (The bus effect below also loads on a workspace change — a second
  // `wsId` effect here loaded the page twice on every mount; O8.)

  // ---------------------------------------------------------------------------
  // Live-refresh driven by improvement_updated WS event (T1).
  // Falls back to a capped 30s poll when the page is visible — the poll is
  // intentionally slower so the WS path is the primary update mechanism.
  // ---------------------------------------------------------------------------

  const POLL_INTERVAL_MS = 30_000;
  const POLL_MAX_TICKS = 20; // stop polling after 10 minutes of inactivity


  $effect(() => {
    // Subscribe to improvementBus: re-load whenever the bus fires.
    const _tick = improvementBus.tick;
    if (wsId) void load(wsId);
  });

  // The fallback poll pauses while the window is hidden and never stacks
  // requests (pollWhileVisible), and still gives up after POLL_MAX_TICKS.
  $effect(() => {
    const id = wsId;
    if (!id) return;
    let ticks = 0;
    const poller = pollWhileVisible(
      async () => {
        if (++ticks > POLL_MAX_TICKS) {
          poller.stop();
          return;
        }
        await load(id);
      },
      { ms: POLL_INTERVAL_MS, immediate: false },
    );
    return () => poller.stop();
  });

  function adopt(id: string, fresh: SelfImprovementConfig): void {
    cfg = fresh;
    cfgWs = id;
    allowlistText = fresh.skill_allowlist.join(', ');
    savedKey = formKey(fresh, allowlistText);
  }

  // Generation guard: a slow load for workspace A landing after a switch to B
  // used to be ADOPTED (the `cfgWs !== id` branch below), showing A's config
  // under B — and Save then wrote it to B.
  const loads = latestOnly();
  async function load(id: string): Promise<void> {
    const t = loads.begin();
    const live = () => t.current && wsId === id;
    if (cfgWs !== id) {
      // Another workspace's form/runs/edits never stay up while this one loads.
      cfg = null;
      cfgWs = null;
      runs = [];
      pending = [];
    }
    loading = true;
    try {
      const fresh = await improveApi.getConfig(id);
      if (!live()) return;
      if (!cfg || cfgWs !== id || !dirty) adopt(id, fresh);
      // Status fields always refresh (they aren't in the form).
      else cfg = { ...cfg, last_run_at: fresh.last_run_at, next_run_at: fresh.next_run_at };
      const r = await improveApi.listRuns(id);
      if (!live()) return;
      runs = r;
      const p = await improveApi.listEdits(id, 'pending');
      if (!live()) return;
      pending = p;
      loadError = '';
    } catch (e) {
      if (!live()) return;
      // Inline (first load) or a slim stale bar (background refresh) — never
      // a toast every 30 s while the daemon is unreachable.
      loadError = loadErrorText(e);
    } finally {
      if (t.current) loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Save config
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    if (!wsId || !cfg || formError) return;
    saving = true;
    try {
      const body = {
        enabled: cfg.enabled,
        cadence_minutes: cfg.cadence_minutes,
        lookback_hours: cfg.lookback_hours,
        skill_allowlist: allowlistText
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
        autonomy: cfg.autonomy as Autonomy,
        providers: cfg.providers,
        live_evolve: cfg.live_evolve,
      };
      adopt(wsId, await improveApi.putConfig(wsId, body));
      toasts.success('Self-improvement settings saved', cfg.enabled ? 'Enabled' : 'Disabled');
    } catch (e) {
      toastError('Couldn’t save self-improvement settings', e);
    } finally {
      saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Run now
  // ---------------------------------------------------------------------------

  async function runNow(): Promise<void> {
    if (!wsId) return;
    running = true;
    try {
      await improveApi.runNow(wsId);
      toasts.info('Self-reflection run started', 'Reviewing recent sessions…');
      await load(wsId);
    } catch (e) {
      toastError('Couldn’t start a run', e);
    } finally {
      running = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Approve / reject a pending edit
  // ---------------------------------------------------------------------------

  async function act(edit: ImprovementEdit, action: 'approve' | 'reject'): Promise<void> {
    if (!wsId) return;
    busyEdit = edit.id;
    busyKind = action === 'approve' ? 'approve' : 'deny';
    try {
      await improveApi[action](edit.id);
      toasts.success(action === 'approve' ? 'Edit approved and applied' : 'Edit denied', edit.target_ref);
      await load(wsId);
    } catch (e) {
      toastError(`Couldn’t ${action === 'approve' ? 'approve' : 'deny'} the edit`, e);
    } finally {
      busyEdit = null;
      busyKind = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Evolve now — triggers a per-session evolve pass and polls until done,
  // then summarises which skills changed (target === 'skill', status === 'applied').
  // The endpoint is fire-and-forget (returns run_id immediately); we poll
  // GET /improvement/runs/{run_id} at 2s intervals until the run settles.
  // ---------------------------------------------------------------------------

  const EVOLVE_POLL_MS = 2_000;
  const EVOLVE_POLL_MAX = 60; // give up after 2 min
  // The Evolve poll is a plain loop: stop it when the section unmounts (it
  // used to keep polling — and toasting — after leaving Settings).
  let disposed = false;
  $effect(() => () => {
    disposed = true;
  });

  async function evolveNow(): Promise<void> {
    if (!activeSession) {
      toasts.error('No active session', 'Open and focus a session first, then click Evolve.');
      return;
    }
    evolving = true;
    evolveResult = null;
    try {
      const { run_id } = await api.post<{ run_id: string }>(`/sessions/${activeSession.id}/evolve`);
      // Poll until the run settles.
      let ticks = 0;
      let settled = false;
      let failures = 0;
      while (!settled && ticks < EVOLVE_POLL_MAX) {
        await new Promise((r) => setTimeout(r, EVOLVE_POLL_MS));
        if (disposed) return;
        ticks++;
        // One transient error no longer aborts the whole wait; three in a row do.
        let got: Awaited<ReturnType<typeof improveApi.getRun>>;
        try {
          got = await improveApi.getRun(run_id);
          failures = 0;
        } catch (e) {
          if (++failures >= 3) throw e;
          continue;
        }
        if (disposed) return;
        const { run, edits } = got;
        if (run.status === 'done' || run.status === 'failed' || run.status === 'skipped') {
          settled = true;
          if (run.status === 'done') {
            // Collect the skill target_refs that were applied during this run.
            const changed = edits
              .filter((e) => e.target === 'skill' && e.status === 'applied')
              .map((e) => e.target_ref);
            evolveResult = changed;
            if (changed.length > 0) {
              toasts.success(
                `${plural(changed.length, 'skill')} updated`,
                changed.slice(0, 3).join(', ') + (changed.length > 3 ? '…' : ''),
              );
            } else {
              toasts.info('Evolve complete', 'No skill changes this session.');
            }
            // Refresh runs + pending so the page is up-to-date.
            if (wsId) await load(wsId);
          } else if (run.status === 'failed') {
            toasts.error('Couldn’t evolve the session', run.error ?? 'The run failed without saying why.');
          } else {
            // skipped
            evolveResult = [];
            toasts.info('Evolve skipped', 'Not enough session content to produce improvements.');
          }
        }
      }
      if (!settled) {
        toasts.info('Evolve running', 'The pass is taking longer than expected — check Recent runs.');
      }
    } catch (e) {
      toastError('Couldn’t evolve the session', e);
    } finally {
      evolving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Display helpers
  // ---------------------------------------------------------------------------

  function fmtDate(s: string | null): string {
    if (!s) return '';
    const d = new Date(s);
    return Number.isNaN(d.getTime()) ? s : d.toLocaleString();
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (dirty && !formError) announce('Unsaved changes — Save to apply them.'); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('self-improvement'),"subtitle":`Improve memory and skills from recent sessions`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(wsId && cfg){
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-icon":`play`}),...__sveltets_2_empty({"data-overflow":`1`}),"disabled":running,"title":`Review recent sessions now`,"onclick":runNow,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} running ? 'Starting…' : 'Run now';
         }
        
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-icon":`sparkle`}),"disabled":evolving || !activeSession,"title":activeSession ? `Evolve skills from “${activeSession.title}” now` : 'No active session — open one first',"onclick":evolveNow,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});} evolving ? 'Evolving…' : 'Evolve now';
         }
         { svelteHTML.createElement("button", {         "class":`btn small primary`,"disabled":saving || !dirty || !!formError,"title":formError || (dirty ? 'Save these settings' : 'No unsaved changes'),"onclick":save,});
          saving ? 'Saving…' : 'Save';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                { svelteHTML.createElement("strong", {});           } SectionIntro}

  if(!wsId){
    
     { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`folder`,"title":NO_WORKSPACE,"body":`Self-improvement is per workspace. Pick one from the workspace menu at the top of the sidebar to configure it.`,}});}
  } else if (!cfg){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`self-improvement settings`,"loading":loading || !loadError,"error":loadError || null,"empty":true,"rows":3,"onretry":() => wsId && void load(wsId),}});}
  }else{
    if(loadError){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {      "what":`self-improvement`,"error":loadError,"onretry":() => wsId && void load(wsId),}});}
    }
    if(formError){
       { svelteHTML.createElement("p", {   "class":`form-note error`,"role":`alert`,});formError; }
    } else if (dirty){
       { svelteHTML.createElement("p", { "class":`form-note unsaved`,});       }
    }
    
    if(evolveResult !== null){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(evolveResult.length === 0 ? 'Evolve finished — no skill changes this session.' : `Evolve finished — ${plural(evolveResult.length, 'skill')} updated.`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {    "class":`evolve-badge`,});evolveResult.length === 0;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":evolveResult.length === 0 ? 'info' : 'check',"size":14,}});}
         { svelteHTML.createElement("span", { "class":`grow`,});
          if(evolveResult.length === 0){       }else{
            plural(evolveResult.length, 'skill'); 
             { svelteHTML.createElement("span", { "class":`evolve-skills mono`,});evolveResult.join(', '); }
          }
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn`,"aria-label":`Dismiss evolve result`,"title":`Dismiss evolve result`,"onclick":() => { evolveResult = null; void focusAppContent(); },}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
       }}
    }
    
    if(pending.length > 0){
       { svelteHTML.createElement("h2", { "class":`section-title`,});   { svelteHTML.createElement("span", { "class":`chip warn`,});pending.length; } }
       { svelteHTML.createElement("div", { "class":`edit-list`,});
           for(let e of __sveltets_2_ensureArray(pending)){e.id;
           { svelteHTML.createElement("div", { "class":`edit-card card`,});
             { svelteHTML.createElement("div", { "class":`edit-head`,});
               { svelteHTML.createElement("span", {   "class":`edit-ref mono`,"title":e.target_ref,});e.target_ref; }
               { svelteHTML.createElement("span", { "class":`chip`,});sentenceCase(e.target); }
               { svelteHTML.createElement("span", { "class":`chip`,});sentenceCase(e.kind); }
               { svelteHTML.createElement("span", {  "class":`chip`,});e.risk === 'structural';sentenceCase(e.risk);  }
             }
             { svelteHTML.createElement("div", { "class":`edit-by`,}); { const $$_enilyBtnegA5C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA5C({ target: __sveltets_2_any(), props: {    "name":`Otto`,"at":e.created_at,}});} }
            if(e.rationale){
               { svelteHTML.createElement("div", { "class":`rationale`,});e.rationale; }
            }
            if(e.evidence.length > 0){
               { svelteHTML.createElement("div", { "class":`evidence dim`,}); e.evidence.join(', '); }
            }
             { svelteHTML.createElement("details", { "class":`diff`,});
               { svelteHTML.createElement("summary", {});    }
               { const $$_weiVffiD5C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD5C({ target: __sveltets_2_any(), props: {         "before":e.before_content ?? '',"after":e.after_content,"mode":`word`,"contextLines":4,}});}
             }
             { const $$_snoitcAlavorppA4C = __sveltets_2_ensureComponent(ApprovalActions); new $$_snoitcAlavorppA4C({ target: __sveltets_2_any(), props: {           "busy":busyEdit === e.id ? busyKind : null,"denyTarget":`this edit`,"askReason":false,"onapprove":() => act(e, 'approve'),"ondeny":() => act(e, 'reject'),}});}
           }
        }
       }
    }

     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", { "class":`card form tfirst`,});
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"label":`Review recent sessions on a schedule`,"checked":cfg.enabled,"testid":`si-enabled`,"onchange":(v) => { if (cfg) cfg.enabled = v; },}});
        if(cfg.enabled && cfg.next_run_at){ { svelteHTML.createElement("span", { "title":fmtDate(cfg.next_run_at),});  rel(cfg.next_run_at);  }} else if (cfg.enabled){   }else{       }
       SettingToggle}
       { svelteHTML.createElement("div", { "class":`grid2 indent`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`si-cadence`,});   }
           { svelteHTML.createElement("input", {               "id":`si-cadence`,"class":`input`,"type":`number`,"min":`1`,"bind:value":cfg.cadence_minutes,"disabled":!cfg.enabled,"title":cfg.enabled ? undefined : 'Turn on the schedule to change how often it runs',});/*Ωignore_startΩ*/() => cfg.cadence_minutes = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`si-lookback`,});   }
           { svelteHTML.createElement("input", {          "id":`si-lookback`,"class":`input`,"type":`number`,"min":`1`,"bind:value":cfg.lookback_hours,});/*Ωignore_startΩ*/() => cfg.lookback_hours = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`hint`,});     }
         }
       }
     }

     { svelteHTML.createElement("h2", { "class":`section-title`,});   }
     { svelteHTML.createElement("div", { "class":`card form`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`si-autonomy`,});  }
         { svelteHTML.createElement("select", {     "id":`si-autonomy`,"class":`input`,"bind:value":cfg.autonomy,});/*Ωignore_startΩ*/() => cfg.autonomy = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`tiered`,});         }
           { svelteHTML.createElement("option", { "value":`propose`,});      }
           { svelteHTML.createElement("option", { "value":`auto`,});      }
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`si-allow`,});  }
         { svelteHTML.createElement("input", {              "dir":`auto`,"id":`si-allow`,"class":`input`,"bind:value":allowlistText,"spellcheck":`false`,"autocomplete":`off`,"placeholder":`support-triage-router, code-review`,});/*Ωignore_startΩ*/() => allowlistText = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`hint`,});
                      
                     
         }
       }
     }

     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", { "class":`card form`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", {   "class":`field-label`,"id":`si-providers-lbl`,});  }
         { svelteHTML.createElement("div", {     "class":`provider-grid`,"role":`group`,"aria-labelledby":`si-providers-lbl`,});
             for(let p of __sveltets_2_ensureArray(providerChoices)){p;
            const on = cfg.providers.includes(p);
             { svelteHTML.createElement("button", {              "type":`button`,"class":`provider-chip pill-toggle`,"aria-pressed":on,"disabled":on && cfg.providers.length === 1,"title":on && cfg.providers.length === 1 ? 'At least one provider is required' : undefined,"onclick":() => toggleProvider(p),});on;
              if(on){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} { svelteHTML.createElement("span", { "class":`mono`,});p; }
             }
          }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});
                       
                        
                         
         }
       }
       { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {           "label":`Live evolve`,"hint":`Improve skills right after each interaction, using the first provider above.`,"checked":cfg.live_evolve,"testid":`si-live-evolve`,"onchange":(v) => { if (cfg) cfg.live_evolve = v; },}});}
     }

    
     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
    if(runs.length === 0){
       { svelteHTML.createElement("p", { "class":`card-info dim`,});             }
    }else{
       { svelteHTML.createElement("div", { "class":`run-list`,});
           for(let r of __sveltets_2_ensureArray(runs)){r.id;
           { svelteHTML.createElement("div", { "class":`run-card card`,});
             { svelteHTML.createElement("div", { "class":`run-head`,});
               { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {  "status":runStatus(r.status),}});}
               { svelteHTML.createElement("span", { "class":`run-meta dim`,});
                sentenceCase(r.trigger);  plural(r.sessions_reviewed, 'session');  r.applied;   r.pending; 
               }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("span", {   "class":`run-time dim`,"title":fmtDate(r.started_at),});rel(r.started_at); }
             }
            if(r.summary){ { svelteHTML.createElement("div", { "class":`run-summary dim`,});r.summary; }}
            if(r.error){ { svelteHTML.createElement("div", { "class":`run-error`,});r.error; }}
           }
        }
       }
    }
  }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SelfImprovement__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SelfImprovement__SvelteComponent_ = ReturnType<typeof SelfImprovement__SvelteComponent_>;
/*Ωignore_endΩ*/export default SelfImprovement__SvelteComponent_;