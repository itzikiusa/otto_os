///<reference types="svelte" />
;
import { rel } from '../../../lib/stores/now.svelte';
import { NO_WORKSPACE } from '../../../lib/labels';
// Insights tab: the latest report of the workspace's "Kubernetes watchdog"
// personal agent (found by the template marker in its persona), rendered
// from Markdown through the shared allowlist sanitizer, plus run history
// and a "Run now". Offers to create the agent from the template when the
// workspace has none.
import { loadErrorText } from '../../../lib/loadError';
import { toastError } from '../../../lib/toastError';
import { onDestroy, untrack } from 'svelte';
import { renderMarkdownGfm } from '../../../lib/md';
import { router } from '../../../lib/router.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { authedText } from '../../../lib/api/client';
import { personalAgentsApi } from '../../../lib/api/personalAgents';
import type { PersonalAgent, PersonalAgentRun } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import AgentEditSheet from '../../personal-agents/AgentEditSheet.svelte';
import { K8S_WATCHDOG_MARK } from '../../personal-agents/templates';
import { verdictOf } from './monitor-util';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let agent = $state<PersonalAgent | null>(null);
  let runs = $state<PersonalAgentRun[]>([]);
  let selected = $state<PersonalAgentRun | null>(null);
  let report = $state('');
  let loading = $state(true);
  let reportLoading = $state(false);
  let error = $state('');
  let creating = $state(false);
  let running = $state(false);
  let reportError = $state('');
  let loadVersion = 0;
  let reportVersion = 0;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;

  // lib/md: GFM + the allowlist sanitizer, never throws.
  const html = $derived(report ? renderMarkdownGfm(report, { breaks: true }) : '');
  const verdict = $derived(verdictOf(report));

  async function load(): Promise<void> {
    const version = ++loadVersion;
    const wsId = ws.currentId;
    agent = null;
    runs = [];
    running = false;
    error = '';
    void openRun(null);
    if (!wsId) {
      loading = false;
      return;
    }
    loading = true;
    try {
      const all = await personalAgentsApi.list(wsId);
      if (version !== loadVersion) return;
      agent = all.find((a) => a.soul_md.includes(K8S_WATCHDOG_MARK)) ?? null;
      if (agent) {
        const nextRuns = await personalAgentsApi.runs(agent.id);
        if (version !== loadVersion) return;
        runs = nextRuns.slice(0, 12);
        await openRun(runs.find((r) => r.report_path) ?? runs[0] ?? null);
      }
    } catch (e) {
      if (version === loadVersion) error = loadErrorText(e);
    } finally {
      if (version === loadVersion) loading = false;
    }
  }

  async function openRun(run: PersonalAgentRun | null): Promise<void> {
    const version = ++reportVersion;
    selected = run;
    report = '';
    reportError = '';
    reportLoading = false;
    if (!run?.report_path) return;
    reportLoading = true;
    try {
      const next = await authedText(personalAgentsApi.reportPath(run.id));
      if (version === reportVersion) report = next;
    } catch (e) {
      if (version === reportVersion) reportError = loadErrorText(e);
    } finally {
      if (version === reportVersion) reportLoading = false;
    }
  }

  $effect(() => {
    const id = ws.currentId;
    void id;
    untrack(() => {
      if (refreshTimer) clearTimeout(refreshTimer);
      void load();
    });
    return () => { loadVersion++; reportVersion++; };
  });

  onDestroy(() => {
    loadVersion++;
    reportVersion++;
    if (refreshTimer) clearTimeout(refreshTimer);
  });

  async function runNow(): Promise<void> {
    if (!agent || running) return;
    const version = loadVersion;
    running = true;
    try {
      await personalAgentsApi.run(agent.id);
      if (version !== loadVersion) return;
      toasts.success('Watchdog started', 'The report appears here when the run finishes.');
      refreshTimer = setTimeout(() => void load(), 4000);
    } catch (e) {
      if (version === loadVersion) toastError('Couldn’t run the insights', e);
    } finally {
      if (version === loadVersion) running = false;
    }
  }

  function verdictClass(v: string | null): string {
    if (v === 'HEALTHY') return 'ok';
    if (v === 'DEGRADED') return 'warn';
    if (v === 'INCIDENT') return 'bad';
    return '';
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`insights`,...__sveltets_2_empty({"data-testid":`k8s-monitor-insights`}),});
  if(loading && !agent){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":4,"height":40,}});}
  } else if (error){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`the watchdog`,"variant":`compact`,error,"empty":true,"onretry":() => void load(),}});}
  } else if (!ws.currentId){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`shield`,"title":NO_WORKSPACE,"body":`Personal agents belong to a workspace — its watchdog reports show here.`,}});}
  } else if (!agent){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {           "icon":`shield`,"title":`No Kubernetes watchdog yet`,"body":`Create a personal agent from the “Kubernetes watchdog” template: every 15 minutes it calls k8s_health on each monitored cluster and writes a short report with a verdict. Delivery to Slack / Telegram / email works like any other agent.`,"actionLabel":`Create watchdog agent`,"onaction":() => (creating = true),}});}
  }else{
     { svelteHTML.createElement("div", { "class":`head card`,});
       { svelteHTML.createElement("div", { "class":`who`,});
         { svelteHTML.createElement("span", { "class":`avatar`,});agent.avatar || '🛡️'; }
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("div", { "class":`name`,});agent.name; }
           { svelteHTML.createElement("div", { "class":`dim small`,});agent.provider;agent.model ? ` · ${agent.model}` : '';agent.enabled ? '' : ' · disabled'; }
         }
       }
       { svelteHTML.createElement("div", { "class":`row`,});
        if(verdict){ { svelteHTML.createElement("span", {   "class":`verdict ${verdictClass(verdict)}`,...__sveltets_2_empty({"data-testid":`k8s-monitor-verdict`}),});verdict; }}
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void runNow(),"disabled":running,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} running ? 'Starting…' : 'Run now'; }
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => router.go(`personal-agents/${encodeURIComponent(agent!.id)}`),});  }
       }
     }

     { svelteHTML.createElement("div", { "class":`body`,});
       { svelteHTML.createElement("aside", { "class":`runs card`,});
         { svelteHTML.createElement("div", { "class":`section-title`,});  }
        if(!runs.length){
           { svelteHTML.createElement("div", { "class":`dim small`,});   }
        }
           for(let r of __sveltets_2_ensureArray(runs)){r.id;
           { svelteHTML.createElement("button", {        "class":`run`,"aria-pressed":selected?.id === r.id,"title":r.summary || r.error || r.status,"onclick":() => void openRun(r),});selected?.id === r.id;
             { svelteHTML.createElement("span", { "class":`st ${r.status}`,}); }
             { svelteHTML.createElement("span", { "class":`when`,});r.started_at ? rel(r.started_at) : 'never'; }
             { svelteHTML.createElement("span", { "class":`sum dim`,});r.summary || r.error || r.status; }
           }
        }
       }
       { svelteHTML.createElement("article", { "class":`report card`,});
        if(reportLoading){
           { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"height":16,}});}
        } else if (reportError){
           { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the report`,"error":reportError,"empty":true,"onretry":() => void openRun(selected),}});}
        } else if (html){
           { svelteHTML.createElement("div", { "class":`md`,}); html; }
        } else if (selected){
           { svelteHTML.createElement("div", { "class":`dim`,});    selected.error ? `: ${selected.error}` : '.'; }
        }else{
           { svelteHTML.createElement("div", { "class":`dim`,});   }
        }
       }
     }
  }
 }

if(creating){
   { const $$_teehStidEtnegA0C = __sveltets_2_ensureComponent(AgentEditSheet); new $$_teehStidEtnegA0C({ target: __sveltets_2_any(), props: {      "agent":null,"template":`k8s-watchdog`,"onclose":() => { creating = false; void load(); },}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MonitorInsights__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MonitorInsights__SvelteComponent_ = ReturnType<typeof MonitorInsights__SvelteComponent_>;
/*Ωignore_endΩ*/export default MonitorInsights__SvelteComponent_;