///<reference types="svelte" />
;// Analysis tab — multi-provider per-lens config, summarizer select, live polling.

import { rel } from '../../lib/stores/now.svelte';
import { announceShown } from '../../lib/announce.svelte';
import type { Poller } from '../../lib/poll';
import { liveQuery } from '../../lib/live';
import { untrack } from 'svelte';
import { product } from '../../lib/stores/product.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import { runStatus, type StatusInfo } from '../../lib/status';
import { toasts } from '../../lib/toast.svelte';
import { runStateLabel } from '../../lib/labels';
import { loadErrorText } from '../../lib/loadError';
import LoadState from '../../lib/components/LoadState.svelte';
import { toastError } from '../../lib/toastError';
import type { ProductAnalysis, ProductAnalysisDetail, ProductAnalysisAgent } from './types';
import type { ProductLens } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  /** Shared run vocabulary (lib/status.ts); `partial` (some agents failed)
   *  is analysis-specific and reads as a warning. */
  function anStatus(s: string): StatusInfo {
    return s === 'partial' ? { key: 'partial', label: 'Partial', tone: 'warning' } : runStatus(s);
  }
  
  
  
  
  
  
  

  // ── Lens definitions ─────────────────────────────────────────────────────────
  // The lens catalog is fetched from GET /workspaces/{id}/product/lenses so the
  // set of offered checks (and their defaults) is backend-curated and stays in
  // sync with the bundled product skills. If that fetch fails or returns empty
  // we fall back to this hardcoded trio so the tab never breaks.
  const FALLBACK_LENSES: ProductLens[] = [
    { skill: 'po-story-overview',           label: 'PO Overview',          description: '', default_on: true },
    { skill: 'story-architecture-overview', label: 'Architecture',         description: '', default_on: true },
    { skill: 'story-clarifying-questions',  label: 'Clarifying questions', description: '', default_on: true },
  ];

  let lenses = $state<ProductLens[]>(FALLBACK_LENSES);

  // ── Provider state (fetched from /meta) ──────────────────────────────────────
  let availableProviders = $state<string[]>(['claude']);
  let configLoaded = $state(false);

  // ── Per-lens UI state (keyed by lens skill; rebuilt from `lenses`) ────────────
  // Whether each lens is enabled.
  let lensEnabled = $state<Record<string, boolean>>({});
  // Selected providers per lens (multi-select chips).
  let lensProviders = $state<Record<string, string[]>>({});

  let summarizerProvider = $state<string>(defaultAgentProvider());

  /** Load the provider list (/meta) and the lens catalog, then (re)build the
   *  per-lens enabled/provider maps as fresh objects keyed by the returned
   *  lenses, clamping providers to what's actually available. Plain async fn
   *  (NOT a $derived) so mutating $state here is safe. */
  async function initConfig(): Promise<void> {
    if (configLoaded) return;

    // Agent providers from the live registry (built-ins + custom, e.g. grok),
    // minus the shell pseudo-provider. Single source of truth — no re-fetch.
    availableProviders = agentProviders();

    // Lens catalog (curated, backend-driven). Fall back to the trio on any
    // failure or empty result so the tab is always usable.
    try {
      const fetched = await product.loadLenses();
      if (Array.isArray(fetched) && fetched.length > 0) lenses = fetched;
    } catch {
      // keep FALLBACK_LENSES
    }

    // Honor the configured default agent (Settings → Providers), else claude.
    const defaultPick = defaultAgentProvider();

    // Build fresh maps keyed by the live lens list — never carry stale keys.
    lensEnabled = Object.fromEntries(lenses.map((l) => [l.skill, l.default_on]));
    lensProviders = Object.fromEntries(lenses.map((l) => [l.skill, [defaultPick]]));

    if (!availableProviders.includes(summarizerProvider)) {
      summarizerProvider = defaultPick;
    }
    configLoaded = true;
  }

  // ── Focus input ───────────────────────────────────────────────────────────────
  let focusText = $state<string>('');

  // ── Other UI state ────────────────────────────────────────────────────────────
  let running = $state(false);
  let loadingHistory = $state(false);
  let historyLoaded = $state(false);
  let activeDetail = $state<ProductAnalysisDetail | null>(null);
  let activeId = $state<string | null>(null);
  // Loading / failure of the picked past run's detail, shown inline in the run
  // area (a failure used to toast and leave the "start a new run" invitation).
  let detailLoading = $state(false);
  let detailError = $state<string | null>(null);
  let collapsed = $state<Record<string, boolean>>({});

  // ── Load provider list + lens catalog on mount ────────────────────────────────
  $effect(() => {
    void initConfig();
    return () => {};
  });

  // ── Polling ────────────────────────────────────────────────────────────────────
  let pollTimer: Poller | null = null;
  const POLL_INTERVAL_MS = 3000;
  // Overall bound: lenses and the summarizer each time out server-side after
  // 10 min, so 30 min of "running" means Otto can no longer tell — say so.
  const POLL_MAX_MS = 30 * 60_000;
  // Consecutive failed reads before the waiting state names the cause.
  const POLL_FAIL_SHOWN = 2;
  let viewGeneration = 0;
  let readSequence = 0;
  let pollStartedAt = 0;
  let polling = $state(false);
  let pollFailures = $state(0);
  let lastPollError = $state<string | null>(null);
  /** Why polling stopped for analysis `id` (403/404, or the overall bound). */
  let pollStopped = $state<{ id: string; text: string } | null>(null);

  function clearPoll(): void {
    ++viewGeneration;
    polling = false;
    if (pollTimer !== null) {
      pollTimer.stop();
      pollTimer = null;
    }
  }

  function isTerminal(status: string): boolean {
    return status === 'done' || status === 'error' || status === 'partial';
  }

  async function pollOnce(): Promise<void> {
    if (!activeId) return;
    const id = activeId;
    if (polling && Date.now() - pollStartedAt > POLL_MAX_MS) {
      const cause = pollFailures > 0 ? lastPollError : null;
      clearPoll();
      pollStopped = { id, text: cause ? `Couldn’t check for the analysis results: ${cause}` : 'Stopped checking after 30 minutes. The analysis may still be running.' };
      return;
    }
    const generation = viewGeneration;
    const request = ++readSequence;
    const owned = product.captureSelection();
    try {
      const detail = await product.getAnalysis(id);
      if (!owned() || generation !== viewGeneration || id !== activeId || request !== readSequence) return;
      activeDetail = detail;
      pollFailures = 0;
      lastPollError = null;
      if (isTerminal(detail.analysis.status)) clearPoll();
    } catch (e) {
      if (!owned() || generation !== viewGeneration || id !== activeId || !polling) return;
      pollFailures++;
      lastPollError = loadErrorText(e);
      // Gone (404) or no longer readable (403): re-polling can't recover — stop
      // and say why, instead of leaving "running" on screen forever.
      const status = (e as { status?: unknown } | null)?.status;
      if (status === 403 || status === 404) {
        clearPoll();
        pollStopped = { id, text: `Couldn’t check progress: ${lastPollError}` };
      }
    }
  }

  function startPolling(id: string): void {
    clearPoll();
    activeId = id;
    detailError = null;
    polling = true;
    pollStartedAt = Date.now();
    pollFailures = 0;
    lastPollError = null;
    pollStopped = null;
    // Immediate first poll, then a settle-then-schedule chain: `product_changed`
    // (onSectionChange, below) settles it; 15 s safety net while events flow,
    // POLL_INTERVAL_MS while the event socket is down.
    pollTimer = liveQuery({ run: () => pollOnce(), on: [], fallbackMs: POLL_INTERVAL_MS, safetyMs: 15_000, jitter: 0 });
  }

  // Subscribe to `product_changed { section: 'analysis' }` WS events so we can
  // clear the poll and refresh immediately instead of waiting for the next tick.
  $effect(() => {
    const off = product.onSectionChange('analysis', (_status: string) => {
      void pollOnce(); // final refresh
    });
    return off;
  });

  // Stop polling when story changes or tab unmounts.
  $effect(() => {
    product.selectedId;
    return () => { clearPoll(); };
  });

  $effect(() => {
    if (activeDetail && isTerminal(activeDetail.analysis.status)) clearPoll();
  });

  // ── Story / derived ────────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);

  $effect(() => {
    // Depend ONLY on the selected story. Everything else is wrapped in untrack:
    // `loadHistory()` reads `historyLoaded` (its early-return guard) and we reset
    // `historyLoaded = false` here, so without untrack this effect would depend on
    // the very flag it flips → it self-retriggers on every load, hammering
    // GET /analyses in a tight loop (and saturating the browser's per-host
    // connection pool, which starved the other tabs' fetches).
    product.selectedId;
    untrack(() => {
      activeDetail = null;
      activeId = null;
      detailLoading = false;
      detailError = null;
      running = false;
      historyLoaded = false;
      collapsed = {};
      clearPoll();
      // Eagerly prime the run-history list so the History <select> is already
      // populated the first time it's opened. A native <select> paints its option
      // list at open time, so the old lazy `onfocus` fetch resolved too late — the
      // first open showed an empty list and the runs only appeared on the second
      // open. historyLoaded was just reset above, so this won't early-return.
      void loadHistory();
    });
  });

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function toggleLens(skill: string): void {
    lensEnabled[skill] = !lensEnabled[skill];
  }

  function toggleLensProvider(skill: string, provider: string): void {
    const cur = lensProviders[skill] ?? [];
    if (cur.includes(provider)) {
      lensProviders[skill] = cur.filter((p) => p !== provider);
    } else {
      lensProviders[skill] = [...cur, provider];
    }
  }

  // Derived: can we run?
  const canRun = $derived(
    lenses.some(
      (l) => lensEnabled[l.skill] && (lensProviders[l.skill]?.length ?? 0) > 0
    )
  );

  // ── Actions ────────────────────────────────────────────────────────────────────

  async function runAnalysis(): Promise<void> {
    if (running || !canRun) return;
    const agents = lenses.filter(
      (l) => lensEnabled[l.skill] && (lensProviders[l.skill]?.length ?? 0) > 0
    ).map((l) => ({
      skill: l.skill,
      name: l.label,
      providers: lensProviders[l.skill],
    }));
    const owned = product.captureSelection();
    const generation = viewGeneration;
    running = true;
    try {
      const trimmedFocus = focusText.trim() || undefined;
      const analysis = await product.analyze({
        agents,
        summarizer_provider: summarizerProvider || defaultAgentProvider(),
        focus: trimmedFocus,
      });
      if (owned() && generation === viewGeneration) startPolling(analysis.id);
    } catch (e) {
      if (owned()) toastError('Couldn’t start the analysis', e);
    } finally {
      if (owned()) running = false;
    }
  }

  let historyError = $state<string | null>(null);

  async function loadHistory(): Promise<void> {
    if (historyLoaded) return;
    loadingHistory = true;
    historyError = null;
    const sid = product.selectedId;
    const owned = product.captureSelection();
    try {
      await product.loadAnalyses();
      if (!owned()) return;
      historyLoaded = true;
      // Open on the latest run, not on an empty "select a past run" pane.
      if (!activeId && product.selectedId === sid && product.analyses.length > 0) {
        const latest = [...product.analyses].sort((a, b) => (b.created_at ?? '').localeCompare(a.created_at ?? ''))[0];
        void selectHistory(latest);
      }
    } catch (e) {
      if (owned()) historyError = loadErrorText(e);
    } finally {
      if (owned()) loadingHistory = false;
    }
  }

  async function selectHistory(a: ProductAnalysis): Promise<void> {
    await selectRun(a.id);
  }

  /** Load a past run into the run area. Re-picking the run whose load failed
   *  (or whose detail isn't on screen) retries it; the loaded one is a no-op. */
  async function selectRun(id: string): Promise<void> {
    if (id === activeId && activeDetail) return;
    clearPoll();
    activeId = id;
    activeDetail = null;
    detailError = null;
    detailLoading = true;
    const generation = viewGeneration;
    const request = ++readSequence;
    const owned = product.captureSelection();
    const current = () => owned() && generation === viewGeneration && activeId === id && request === readSequence;
    try {
      const detail = await product.getAnalysis(id);
      if (!current()) return;
      activeDetail = detail;
      if (!isTerminal(detail.analysis.status)) startPolling(id);
    } catch (e) {
      if (current()) detailError = loadErrorText(e);
    } finally {
      if (current()) detailLoading = false;
    }
  }

  function retrySelect(): void {
    if (activeId) void selectRun(activeId);
  }

  // Inline terminal state — multiple may be open at once, keyed by session id.
  // NOTE: No ws.openSession() here — the inline <LazyTerminal sessionId={...} />
  // connects directly by id. Calling openSession would push it into the Agents
  // grid sidebar which we don't want.
  let openTerminals = $state<Set<string>>(new Set());
  function toggleTerminal(sid: string): void {
    const next = new Set(openTerminals);
    if (next.has(sid)) next.delete(sid);
    else next.add(sid);
    openTerminals = next;
  }

  // ── Per-agent retry ───────────────────────────────────────────────────────
  // Tracks which agent IDs currently have a retry in-flight.
  let retryingAgents = $state<Set<string>>(new Set());

  async function retryAgent(analysisId: string, agentId: string, agentName: string): Promise<void> {
    if (retryingAgents.has(agentId)) return;
    const next = new Set(retryingAgents);
    next.add(agentId);
    retryingAgents = next;
    const owned = product.captureSelection();
    const generation = viewGeneration;
    try {
      await product.retryAgent(analysisId, agentId);
      toasts.info(`Re-running ${agentName}…`);
      // Resume polling so results refresh automatically.
      if (owned() && generation === viewGeneration && analysisId === activeId) startPolling(analysisId);
    } catch (e) {
      toastError('Couldn’t retry the analysis', e);
    } finally {
      const done = new Set(retryingAgents);
      done.delete(agentId);
      retryingAgents = done;
    }
  }

  // ── Per-agent stop (manual override for a running/waiting agent) ────────────
  let stoppingAgents = $state<Set<string>>(new Set());

  async function stopAgent(analysisId: string, agentId: string, agentName: string): Promise<void> {
    if (stoppingAgents.has(agentId)) return;
    const next = new Set(stoppingAgents);
    next.add(agentId);
    stoppingAgents = next;
    try {
      await product.stopAgent(analysisId, agentId);
      toasts.info(`Stopped ${agentName}`);
      // Refresh so the stopped (errored) state shows immediately.
      void pollOnce();
    } catch (e) {
      toastError('Couldn’t stop the analysis', e);
    } finally {
      const done = new Set(stoppingAgents);
      done.delete(agentId);
      stoppingAgents = done;
    }
  }

  // ── Findings parsing ──────────────────────────────────────────────────────────

  interface OpenQuestion { text: string; rationale: string; category: string; }
  interface SuggestedLearning { kind: string; title: string; body: string; }
  interface Findings {
    summary?: string;
    related_repos?: string[];
    functionalities?: string[];
    integration_points?: string[];
    risks?: string[];
    open_questions?: OpenQuestion[];
    suggested_learnings?: SuggestedLearning[];
  }

  function parseFindings(json: string | null): Findings | null {
    if (!json) return null;
    try { return JSON.parse(json) as Findings; } catch { return null; }
  }

  function toggleCollapse(key: string): void {
    collapsed = { ...collapsed, [key]: !collapsed[key] };
  }

  function fmtDate(s: string | null): string {
    if (!s) return '';
    try { return rel(s); } catch { return s; }
  }

  // Separate summarizer agent from regular lens agents
  const currentAgents = $derived(activeDetail?.agents ?? []);
  const summarizerAgent = $derived(
    currentAgents.find((a) => a.skill === 'summarizer' || a.name?.toLowerCase().startsWith('summarizer'))
  );
  const lensAgents = $derived(
    currentAgents.filter((a) => a !== summarizerAgent)
  );
  const currentAnalysis = $derived(activeDetail?.analysis ?? null);
  const analysisStatus = $derived(currentAnalysis?.status ?? '');
;
async () => {


 { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});pollStopped && pollStopped.id === activeId ? pollStopped.text : polling && pollFailures >= POLL_FAIL_SHOWN && lastPollError ? `Couldn’t check progress: ${lastPollError}` : ''; }

if(!story){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`analysis-tab`,});

    
     { svelteHTML.createElement("section", { "class":`run-panel card`,});
       { svelteHTML.createElement("div", { "class":`run-header`,});
         { svelteHTML.createElement("span", { "class":`section-head`,});  }
       }

       { svelteHTML.createElement("div", { "class":`lens-grid`,});
           for(let lens of __sveltets_2_ensureArray(lenses)){lens.skill;
           { svelteHTML.createElement("div", {  "class":`lens-row`,});!lensEnabled[lens.skill];
            
             { svelteHTML.createElement("label", {   "class":`lens-toggle`,"title":lens.description || lens.label,});
               { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":lensEnabled[lens.skill],"onchange":() => toggleLens(lens.skill),"disabled":running,});}
               { svelteHTML.createElement("span", { "class":`lens-name`,});lens.label; }
             }

            
             { svelteHTML.createElement("div", {  "class":`chip-group`,});!lensEnabled[lens.skill];
                 for(let p of __sveltets_2_ensureArray(availableProviders)){p;
                 { svelteHTML.createElement("button", {            "class":`chip`,"aria-pressed":(lensProviders[lens.skill] ?? []).includes(p),"disabled":running || !lensEnabled[lens.skill],"onclick":() => toggleLensProvider(lens.skill, p),"title":p,});(lensProviders[lens.skill] ?? []).includes(p);
                  p;
                 }
              }
             }
           }
        }
       }

      
       { svelteHTML.createElement("div", { "class":`focus-wrap`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`focus-input`,});  { svelteHTML.createElement("span", { "class":`focus-optional`,});  } }
         { svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`focus-input`,"class":`focus-input`,"rows":2,"placeholder":`What to focus on — e.g. 'security implications' or 'edge cases around payments'`,"bind:value":focusText,"disabled":running,});/*Ωignore_startΩ*/() => focusText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }

      
       { svelteHTML.createElement("div", { "class":`bottom-row`,});
         { svelteHTML.createElement("div", { "class":`summarizer-wrap`,});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`summarizer-sel`,});  }
           { svelteHTML.createElement("select", {         "id":`summarizer-sel`,"class":`small-select`,"bind:value":summarizerProvider,"disabled":running,});/*Ωignore_startΩ*/() => summarizerProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let p of __sveltets_2_ensureArray(availableProviders)){p;
               { svelteHTML.createElement("option", { "value":p,});p; }
            }
           }
         }

         { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":runAnalysis,"disabled":running || !canRun,"title":!running && !canRun ? 'Enable at least one lens and pick a provider for it' : undefined,});
          running ? 'Starting…' : 'Run analysis';
         }
       }
     }

    
     { svelteHTML.createElement("section", { "class":`history-row`,});
       { svelteHTML.createElement("span", { "class":`field-label`,});  }
       { svelteHTML.createElement("select", {            "aria-label":`Analysis history`,"class":`hist-select`,"onfocus":loadHistory,"onchange":(e) => {
          const id = (e.target as HTMLSelectElement).value;
          const found = product.analyses.find((a) => a.id === id);
          if (found) void selectHistory(found);
        },"disabled":loadingHistory,"value":activeId ?? '',});
         { svelteHTML.createElement("option", {"value":"",});      }
           for(let a of __sveltets_2_ensureArray(product.analyses)){a.id;
           { svelteHTML.createElement("option", { "value":a.id,});
            fmtDate(a.created_at);  runStateLabel(a.status);
           }
        }
       }
      if(loadingHistory){
         { svelteHTML.createElement("span", {       "class":`spinner`,"role":`img`,"aria-label":`Loading past runs`,"title":`Loading past runs`,}); }
      } else if (historyError){
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('span'),({ text: `Couldn’t load past runs. ${historyError}`, assertive: true })));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {     "class":`dim-sm hist-error`,"title":historyError,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}     }}
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void loadHistory(),});  }
      } else if (historyLoaded && product.analyses.length === 0){
         { svelteHTML.createElement("span", { "class":`dim-sm`,});             }
      }
     }

    
    if(pollStopped && pollStopped.id === activeId){
       { svelteHTML.createElement("div", { "class":`poll-note`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} pollStopped.text;
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { if (activeId) startPolling(activeId); },});  }
       }
    } else if (polling && pollFailures >= POLL_FAIL_SHOWN && lastPollError){
       { svelteHTML.createElement("div", { "class":`poll-note`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}    lastPollError;
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => pollTimer?.now(),});  }
       }
    }

    
    if(activeDetail){

      
      if((analysisStatus === 'done' || analysisStatus === 'partial') && currentAnalysis?.summary){
         { svelteHTML.createElement("section", { "class":`synthesis-card card`,});
           { svelteHTML.createElement("div", { "class":`section-head`,});  }
          if(summarizerAgent){
             { svelteHTML.createElement("div", { "class":`summarizer-badge`,});
               { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":summarizerAgent.status}),}); { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {  "status":anStatus(summarizerAgent.status),}});} }
              if(summarizerAgent.session_id){
                 { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => toggleTerminal(summarizerAgent!.session_id!),"title":`Open summarizer session`,});
                  openTerminals.has(summarizerAgent.session_id) ? 'Hide' : 'Open'; 
                 }
              }
             }
          }
          if(summarizerAgent?.session_id && openTerminals.has(summarizerAgent.session_id)){
             { svelteHTML.createElement("div", { "class":`rp-term`,});
              summarizerAgent.session_id; {
                 { const $$_lanimreTyzaL3C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL3C({ target: __sveltets_2_any(), props: {    "sessionId":summarizerAgent.session_id,"forceDark":true,"preferDom":true,"resumeOnOpen":false,}});}
              }
             }
          }
           { svelteHTML.createElement("p", { "class":`synthesis-body`,});currentAnalysis.summary; }
         }
      }

      
       { svelteHTML.createElement("section", { "class":`agents-section`,});
         { svelteHTML.createElement("div", { "class":`agents-header`,});
           { svelteHTML.createElement("span", { "class":`section-head`,});  }
           { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":analysisStatus}),}); { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {  "status":anStatus(analysisStatus),}});} }
         }
         { svelteHTML.createElement("div", { "class":`rp-agents`,});
             for(let agent of __sveltets_2_ensureArray(currentAgents)){agent.id;
            const isLens = agent !== summarizerAgent;
            const isRetrying = retryingAgents.has(agent.id);
             { svelteHTML.createElement("div", { "class":`rp-agent card`,});
               { svelteHTML.createElement("div", { "class":`rp-agent-top`,});
                 { svelteHTML.createElement("span", { "class":`rp-agent-name`,});agent.name || agent.skill; }
                 { const $$_enilyBtnegA5C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA5C({ target: __sveltets_2_any(), props: {      "provider":agent.provider,"model":agent.model,"at":agent.finished_at ?? agent.started_at,}});}
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                if(agent.session_id){
                   { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => toggleTerminal(agent.session_id!),});
                    openTerminals.has(agent.session_id) ? 'Hide' : 'Open';
                   }
                }
                if(isLens && (agent.status === 'error' || agent.status === 'done')){
                   { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":isRetrying,"onclick":() => retryAgent(currentAnalysis!.id, agent.id, agent.name || agent.skill),"title":`Re-run this agent`,});
                    isRetrying ? 'Retrying…' : 'Retry';
                   }
                }
                if(agent.status === 'running' || agent.status === 'waiting'){
                   { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":stoppingAgents.has(agent.id),"onclick":() => stopAgent(currentAnalysis!.id, agent.id, agent.name || agent.skill),"title":`Stop this agent`,});
                    stoppingAgents.has(agent.id) ? 'Stopping…' : 'Stop';
                   }
                }
                 { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":agent.status}),}); { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {  "status":anStatus(agent.status),}});} }
               }
              if(agent.error && agent.status === 'error' && !isLens){
                 { svelteHTML.createElement("p", { "class":`rp-agent-note error-note`,});agent.error; }
              }
              if(agent.status === 'waiting'){
                 { svelteHTML.createElement("p", { "class":`rp-agent-waiting`,});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}         { svelteHTML.createElement("strong", {});  }   
                        
                 }
              }
              if(agent.session_id && openTerminals.has(agent.session_id)){
                 { svelteHTML.createElement("div", { "class":`rp-term`,});
                  agent.session_id; {
                     { const $$_lanimreTyzaL5C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL5C({ target: __sveltets_2_any(), props: {    "sessionId":agent.session_id,"forceDark":true,"preferDom":true,"resumeOnOpen":false,}});}
                  }
                 }
              }
             }
          }
         }
       }

      
         for(let agent of __sveltets_2_ensureArray(lensAgents)){agent.id;
        if(agent.status === 'done' && agent.findings_json){
          const findings = parseFindings(agent.findings_json);
          if(findings){
             { svelteHTML.createElement("section", { "class":`findings-card card`,});
               { svelteHTML.createElement("div", { "class":`findings-header`,});
                 { svelteHTML.createElement("span", { "class":`findings-who`,});
                   { svelteHTML.createElement("span", { "class":`findings-agent-name`,});agent.name || agent.skill; }
                   { const $$_enilyBtnegA4C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA4C({ target: __sveltets_2_any(), props: {      "provider":agent.provider,"model":agent.model,"at":agent.finished_at,}});}
                 }
                 { svelteHTML.createElement("div", { "class":`findings-right`,});
                  if(agent.session_id){
                     { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => toggleTerminal(agent.session_id!),"title":`Open session`,});
                      openTerminals.has(agent.session_id) ? 'Hide' : 'Open';
                     }
                  }
                  if(currentAnalysis){
                     { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":retryingAgents.has(agent.id),"onclick":() => retryAgent(currentAnalysis!.id, agent.id, agent.name || agent.skill),"title":`Re-run this agent`,});
                      retryingAgents.has(agent.id) ? 'Retrying…' : 'Retry';
                     }
                  }
                   { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":`done`}),}); { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {  "status":anStatus('done'),}});} }
                 }
               }

              if(agent.session_id && openTerminals.has(agent.session_id)){
                 { svelteHTML.createElement("div", { "class":`rp-term`,});
                  agent.session_id; {
                     { const $$_lanimreTyzaL3C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL3C({ target: __sveltets_2_any(), props: {    "sessionId":agent.session_id,"forceDark":true,"preferDom":true,"resumeOnOpen":false,}});}
                  }
                 }
              }

              if(findings.summary){
                 { svelteHTML.createElement("p", { "class":`findings-summary`,});findings.summary; }
              }

              
              if(findings.related_repos && findings.related_repos.length > 0){
                const key = agent.id + ':repos';
                 { svelteHTML.createElement("div", { "class":`collapsible`,});
                   { svelteHTML.createElement("button", {   "class":`coll-trigger`,"onclick":() => toggleCollapse(key),});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsed[key] ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                     
                     { svelteHTML.createElement("span", { "class":`coll-count`,}); findings.related_repos.length;  }
                   }
                  if(!collapsed[key]){
                     { svelteHTML.createElement("ul", { "class":`findings-list`,});
                        for(let repo of __sveltets_2_ensureArray(findings.related_repos)){
                         { svelteHTML.createElement("li", { "class":`mono-sm`,});repo; }
                      }
                     }
                  }
                 }
              }

              
              if(findings.functionalities && findings.functionalities.length > 0){
                const key = agent.id + ':func';
                 { svelteHTML.createElement("div", { "class":`collapsible`,});
                   { svelteHTML.createElement("button", {   "class":`coll-trigger`,"onclick":() => toggleCollapse(key),});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsed[key] ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                    
                     { svelteHTML.createElement("span", { "class":`coll-count`,}); findings.functionalities.length;  }
                   }
                  if(!collapsed[key]){
                     { svelteHTML.createElement("ul", { "class":`findings-list`,});
                        for(let f of __sveltets_2_ensureArray(findings.functionalities)){
                         { svelteHTML.createElement("li", {});f; }
                      }
                     }
                  }
                 }
              }

              
              if(findings.integration_points && findings.integration_points.length > 0){
                const key = agent.id + ':int';
                 { svelteHTML.createElement("div", { "class":`collapsible`,});
                   { svelteHTML.createElement("button", {   "class":`coll-trigger`,"onclick":() => toggleCollapse(key),});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsed[key] ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                     
                     { svelteHTML.createElement("span", { "class":`coll-count`,}); findings.integration_points.length;  }
                   }
                  if(!collapsed[key]){
                     { svelteHTML.createElement("ul", { "class":`findings-list`,});
                        for(let ip of __sveltets_2_ensureArray(findings.integration_points)){
                         { svelteHTML.createElement("li", {});ip; }
                      }
                     }
                  }
                 }
              }

              
              if(findings.risks && findings.risks.length > 0){
                const key = agent.id + ':risks';
                 { svelteHTML.createElement("div", { "class":`collapsible`,});
                   { svelteHTML.createElement("button", {   "class":`coll-trigger`,"onclick":() => toggleCollapse(key),});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsed[key] ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                    
                     { svelteHTML.createElement("span", { "class":`coll-count`,}); findings.risks.length;  }
                   }
                  if(!collapsed[key]){
                     { svelteHTML.createElement("ul", { "class":`findings-list risk-list`,});
                        for(let risk of __sveltets_2_ensureArray(findings.risks)){
                         { svelteHTML.createElement("li", {});risk; }
                      }
                     }
                  }
                 }
              }

              
              if(findings.open_questions && findings.open_questions.length > 0){
                const key = agent.id + ':oq';
                 { svelteHTML.createElement("div", { "class":`collapsible`,});
                   { svelteHTML.createElement("button", {   "class":`coll-trigger`,"onclick":() => toggleCollapse(key),});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsed[key] ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                     
                     { svelteHTML.createElement("span", { "class":`coll-count`,}); findings.open_questions.length;  }
                   }
                  if(!collapsed[key]){
                     { svelteHTML.createElement("ul", { "class":`findings-list question-list`,});
                        for(let q of __sveltets_2_ensureArray(findings.open_questions)){
                         { svelteHTML.createElement("li", {});
                           { svelteHTML.createElement("span", { "class":`q-text`,});q.text; }
                          if(q.category){
                             { svelteHTML.createElement("span", { "class":`q-cat`,});q.category; }
                          }
                         }
                      }
                     }
                  }
                 }
              }

              
              if(findings.suggested_learnings && findings.suggested_learnings.length > 0){
                const key = agent.id + ':sl';
                 { svelteHTML.createElement("div", { "class":`collapsible`,});
                   { svelteHTML.createElement("button", {   "class":`coll-trigger`,"onclick":() => toggleCollapse(key),});
                     { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsed[key] ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                     
                     { svelteHTML.createElement("span", { "class":`coll-count`,}); findings.suggested_learnings.length;  }
                   }
                  if(!collapsed[key]){
                     { svelteHTML.createElement("div", { "class":`sl-hint`,});
                       { svelteHTML.createElement("span", { "class":`sl-hint-text`,});                  }
                       { svelteHTML.createElement("button", {   "class":`sl-hint-btn`,"onclick":() => { product.view = 'learnings'; },});
                         
                       }
                     }
                     { svelteHTML.createElement("div", { "class":`sl-list`,});
                        for(let sl of __sveltets_2_ensureArray(findings.suggested_learnings)){
                         { svelteHTML.createElement("div", { "class":`sl-item`,});
                           { svelteHTML.createElement("div", { "class":`sl-header`,});
                             { svelteHTML.createElement("span", { "class":`sl-kind`,});sl.kind; }
                             { svelteHTML.createElement("span", { "class":`sl-title`,});sl.title; }
                           }
                          if(sl.body){
                             { svelteHTML.createElement("p", { "class":`sl-body`,});sl.body; }
                          }
                         }
                      }
                     }
                  }
                 }
              }
             }
          }
        } else if (agent.status === 'error'){
           { svelteHTML.createElement("section", { "class":`findings-card card error-card`,});
             { svelteHTML.createElement("div", { "class":`findings-header`,});
               { svelteHTML.createElement("span", { "class":`findings-agent-name`,});agent.name || agent.skill; }
               { svelteHTML.createElement("div", { "class":`findings-right`,});
                if(agent.session_id){
                   { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => toggleTerminal(agent.session_id!),"title":`Open session`,});
                    openTerminals.has(agent.session_id) ? 'Hide' : 'Open';
                   }
                }
                if(currentAnalysis){
                   { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":retryingAgents.has(agent.id),"onclick":() => retryAgent(currentAnalysis!.id, agent.id, agent.name || agent.skill),"title":`Re-run this agent`,});
                    retryingAgents.has(agent.id) ? 'Retrying…' : 'Retry';
                   }
                }
                 { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":`error`}),}); { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {  "status":anStatus('error'),}});} }
               }
             }
            if(agent.session_id && openTerminals.has(agent.session_id)){
               { svelteHTML.createElement("div", { "class":`rp-term`,});
                agent.session_id; {
                   { const $$_lanimreTyzaL3C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL3C({ target: __sveltets_2_any(), props: {    "sessionId":agent.session_id,"forceDark":true,"preferDom":true,"resumeOnOpen":false,}});}
                }
               }
            }
             { svelteHTML.createElement("p", { "class":`error-msg`,});agent.error ?? 'This agent stopped without reporting a reason.'; }
           }
        }
      }

    } else if (detailLoading || detailError){
      
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`this analysis run`,"loading":detailLoading,"error":detailError,"empty":true,"onretry":retrySelect,}});}
    } else if (!running){
       { svelteHTML.createElement("div", { "class":`muted`,});       }
    }else{
       { svelteHTML.createElement("div", { "class":`muted`,});  }
    }

   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AnalysisTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AnalysisTab__SvelteComponent_ = ReturnType<typeof AnalysisTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default AnalysisTab__SvelteComponent_;