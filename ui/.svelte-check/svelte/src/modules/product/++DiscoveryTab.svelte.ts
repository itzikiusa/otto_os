///<reference types="svelte" />
;
import { rel } from '../../lib/stores/now.svelte';
// Discovery tab — lists discovery swarm runs for the current story, lets the
// user expand a run to read the report + per-task summaries + board messages,
// and provides a "Run Discovery" button (with team picker) for repeat runs.
import { product } from '../../lib/stores/product.svelte';
import { toastError } from '../../lib/toastError';
import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { runStatus } from '../../lib/status';
import { loadErrorText } from '../../lib/loadError';
import { swarm } from '../../lib/stores/swarm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
import { kindLabel } from '../../lib/labels';
import { renderMarkdown } from '../../lib/md';
import { confirmer } from '../../lib/confirm.svelte';
import type { DiscoveryRunSummary, DiscoveryRunDetail } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── State ─────────────────────────────────────────────────────────────────
  let runs = $state<DiscoveryRunSummary[]>([]);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  // Expanded run: id → DiscoveryRunDetail (null = loading, undefined = not fetched)
  let expandedId = $state<string | null>(null);
  let expandedDetail = $state<DiscoveryRunDetail | null>(null);
  let expandLoading = $state(false);
  let expandError = $state<string | null>(null);

  // Run Discovery controls
  let targetSwarmId = $state('');
  let running = $state(false);
  let listSequence = 0;
  let detailSequence = 0;

  // ── Swarms ────────────────────────────────────────────────────────────────
  $effect(() => {
    const wsId = ws.currentId;
    if (wsId) void swarm.ensureSwarms(wsId);
  });

  // ── Load on mount / story change ──────────────────────────────────────────
  $effect(() => {
    // Re-run whenever the selected story changes.
    product.selectedId;
    ++detailSequence;
    expandedId = null;
    expandedDetail = null;
    void loadRuns();
  });

  async function loadRuns(): Promise<void> {
    const sequence = ++listSequence;
    const storyId = product.selectedId;
    loading = true;
    loadError = null;
    try {
      const result = await product.listDiscoveryRuns();
      if (sequence === listSequence && storyId === product.selectedId) runs = result;
    } catch (e) {
      if (sequence === listSequence && storyId === product.selectedId) loadError = loadErrorText(e);
    } finally {
      if (sequence === listSequence && storyId === product.selectedId) loading = false;
    }
  }

  async function toggleRun(id: string): Promise<void> {
    const sequence = ++detailSequence;
    const storyId = product.selectedId;
    if (expandedId === id) {
      expandedId = null;
      expandedDetail = null;
      return;
    }
    expandedId = id;
    expandedDetail = null;
    expandError = null;
    expandLoading = true;
    try {
      const result = await product.getDiscoveryRun(id);
      if (sequence === detailSequence && storyId === product.selectedId) expandedDetail = result;
    } catch (e) {
      if (sequence === detailSequence && storyId === product.selectedId) expandError = loadErrorText(e);
    } finally {
      if (sequence === detailSequence && storyId === product.selectedId) expandLoading = false;
    }
  }

  async function runDiscovery(): Promise<void> {
    if (running) return;
    const story = product.detail?.story;
    if (!story || !ws.currentId) return;

    const targetSwarm = swarm.swarms.find((s) => s.id === targetSwarmId);
    const teamName = targetSwarm ? `“${targetSwarm.name}”` : 'a swarm';

    const attCount = 0; // attachment count not tracked here; overview has the panel
    const ok = await confirmer.ask(
      `Run Discovery in ${teamName}? This will START the swarm and send the story info${attCount > 0 ? ` + ${attCount} attachments` : ''} as discovery context.`,
      { title: 'Run discovery', confirmLabel: 'Run discovery', danger: false },
    );
    if (!ok) return;

    running = true;
    try {
      await product.discover(targetSwarmId ? { swarm_id: targetSwarmId } : {});
      toasts.success('Discovery started', 'The swarm is now analyzing the story.');
      await loadRuns();
    } catch (e) {
      toastError('Couldn’t run discovery', e);
    } finally {
      running = false;
    }
  }

  async function viewInSwarm(summary: DiscoveryRunSummary): Promise<void> {
    if (!ws.currentId) return;
    try {
      await swarm.openProject(ws.currentId, summary.run.swarm_id, summary.run.project_id);
      router.go('swarm');
    } catch (e) {
      toastError('Couldn’t open swarm', e);
    }
  }

  // ── Helpers ───────────────────────────────────────────────────────────────
  function statusColor(s: string): string {
    switch (s) {
      case 'done': return 'status-done';
      case 'error': return 'status-error';
      case 'running': return 'status-running';
      default: return 'status-other';
    }
  }

;
async () => {

 { svelteHTML.createElement("div", { "class":`discovery-tab`,});
  
   { svelteHTML.createElement("div", { "class":`toolbar`,});
     { svelteHTML.createElement("span", { "class":`toolbar-title`,});  }
     { svelteHTML.createElement("span", { "class":`grow`,}); }

    
    if(swarm.swarms.length > 1){
       { svelteHTML.createElement("select", {     "class":`picker`,"bind:value":targetSwarmId,"title":`Which swarm runs the discovery`,});/*Ωignore_startΩ*/() => targetSwarmId = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let s of __sveltets_2_ensureArray(swarm.swarms)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.name; }}
       }
    }

     { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":runDiscovery,"disabled":running,"title":`Launch a new discovery swarm run for this story`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});} running ? 'Starting…' : 'Run discovery';
     }

     { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":loadRuns,"disabled":loading,"title":`Refresh discovery runs`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} loading ? 'Refreshing…' : 'Refresh';
     }
   }

  
  if((loading || loadError) && runs.length === 0){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`discovery runs`,"loading":loading,"error":loadError,"empty":true,"onretry":() => void loadRuns(),}});}
  } else if (runs.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {       "icon":`search`,"title":`No discovery runs yet`,"body":`Run discovery to have a swarm analyze this story and report its findings here.`,}});}
  }else{
    if(loadError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`discovery runs`,"variant":`compact`,"loading":loading,"error":loadError,"onretry":() => void loadRuns(),}});}
    }
     { svelteHTML.createElement("div", { "class":`run-list`,});
         for(let summary of __sveltets_2_ensureArray(runs)){summary.run.id;
        const isOpen = expandedId === summary.run.id;
         { svelteHTML.createElement("div", {  "class":`run-card`,});isOpen;
          
          
           { svelteHTML.createElement("div", { "class":`run-head-row`,});
             { svelteHTML.createElement("button", {     "class":`run-header`,"aria-expanded":isOpen,"onclick":() => toggleRun(summary.run.id),});
               { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":isOpen ? 'chevronDown' : 'chevronRight',"size":12,}});} }
               { svelteHTML.createElement("span", { "class":`status-badge`,}); { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {  "status":runStatus(summary.derived_status),}});} }
               { svelteHTML.createElement("span", {   "class":`run-date`,"title":new Date(summary.run.created_at).toLocaleString(),});rel(summary.run.created_at); }
               { svelteHTML.createElement("span", { "class":`run-progress`,});
                summary.done_count; summary.task_count; 
               }
             }
             { svelteHTML.createElement("button", {       "class":`view-swarm-btn`,"onclick":() => void viewInSwarm(summary),"title":`Open in Swarm`,});
                  { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
             }
           }

          
          if(isOpen){
             { svelteHTML.createElement("div", { "class":`run-body`,});
              if(expandLoading || expandError){
                 { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`the run details`,"variant":`compact`,"loading":expandLoading,"error":expandError,"empty":true,"onretry":() => { const id = summary.run.id; expandedId = null; void toggleRun(id); },}});}
              } else if (expandedDetail){
                
                if(expandedDetail.run.report_md){
                   { svelteHTML.createElement("div", { "class":`report-section`,});
                     { svelteHTML.createElement("div", { "class":`section-label`,});  }
                     { svelteHTML.createElement("div", { "class":`md-body`,}); renderMarkdown(expandedDetail.run.report_md); }
                   }
                }else{
                   { svelteHTML.createElement("div", { "class":`muted inner-pad`,});          }
                }

                
                if(expandedDetail.task_summaries.length > 0){
                   { svelteHTML.createElement("div", { "class":`tasks-section`,});
                     { svelteHTML.createElement("div", { "class":`section-label`,});  }
                     { svelteHTML.createElement("div", { "class":`task-list`,});
                         for(let task of __sveltets_2_ensureArray(expandedDetail.tasks)){task.id;
                        const taskSummaryEntry = expandedDetail.task_summaries.find(([tid]) => tid === task.id);
                        const taskSummary = taskSummaryEntry ? taskSummaryEntry[1] : null;
                         { svelteHTML.createElement("div", { "class":`task-item`,});
                           { svelteHTML.createElement("div", { "class":`task-row`,});
                             { svelteHTML.createElement("span", {   "class":`task-status-dot ${statusColor(task.status)}`,"aria-hidden":`true`,}); }
                             { const $$_egdaBsutatS8C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS8C({ target: __sveltets_2_any(), props: {    "status":runStatus(task.status),"variant":`text`,}});}
                             { svelteHTML.createElement("span", { "class":`task-title`,});task.title; }
                           }
                          if(taskSummary){
                             { svelteHTML.createElement("p", { "class":`task-summary-text`,});taskSummary; }
                          }
                         }
                      }
                     }
                   }
                }

                
                if(expandedDetail.messages.length > 0){
                   { svelteHTML.createElement("div", { "class":`messages-section`,});
                     { svelteHTML.createElement("div", { "class":`section-label`,});  }
                     { svelteHTML.createElement("div", { "class":`message-list`,});
                         for(let msg of __sveltets_2_ensureArray(expandedDetail.messages)){msg.id;
                         { svelteHTML.createElement("div", { "class":`message-item`,});
                           { svelteHTML.createElement("span", { "class":`message-role`,});kindLabel(msg.kind); }
                           { svelteHTML.createElement("span", { "class":`message-content`,});msg.body; }
                         }
                      }
                     }
                   }
                }

                
                 { svelteHTML.createElement("div", { "class":`run-footer`,});
                   { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => viewInSwarm(summary),});
                        { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
                   }
                 }
              }
             }
          }
         }
      }
     }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DiscoveryTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DiscoveryTab__SvelteComponent_ = ReturnType<typeof DiscoveryTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default DiscoveryTab__SvelteComponent_;