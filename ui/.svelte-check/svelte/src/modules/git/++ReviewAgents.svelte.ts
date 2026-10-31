///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Shared per-agent results block for BOTH the PR review (ReviewPanel) and the
// local working-tree review (LocalReviewPanel) — so "Open" (inline live
// terminal), "Retry", per-agent findings and status pills behave identically
// and live in one place. The backend is already shared: retry hits
// POST /reviews/{id}/agents/{index}/retry, keyed by review id.
import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { api } from '../../lib/api/client';
import type { Review } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus, type BadgeTone } from '../../lib/status';
import { severityLabel } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import { summaryFallback, summaryFallbackTitle } from './reviewFallback';
;

  interface Props {
    review: Review;
    /** 'running' and 'done' both show every agent, incl. the trailing
     *  summarizer — hiding the summarizer when done left a wiped run (agents
     *  done, 0 comments) with nothing to inspect or retry. */
    view: 'running' | 'done';
    /** Called after a successful retry with the refreshed review, so the parent
     *  can update its state and resume polling. */
    onretried?: (review: Review) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { review, view, onretried }: Props = $props();

  const rows = $derived(review.agents);
  // The summarizer is the last entry; it retries via its own endpoint (below).
  const lastRetryable = $derived(review.agents.length - 1);

  // Inline live terminals (multiple may be open at once), keyed by session id.
  let openTerminals = $state<Set<string>>(new Set());
  function toggleTerminal(sessionId: string | null | undefined): void {
    if (!sessionId) return;
    const next = new Set(openTerminals);
    if (next.has(sessionId)) next.delete(sessionId);
    else next.add(sessionId);
    openTerminals = next;
  }

  // Which agent rows are expanded to show their individual findings.
  let agentExpanded: Record<string, boolean> = $state({});
  function toggleAgent(name: string): void {
    agentExpanded = { ...agentExpanded, [name]: !agentExpanded[name] };
  }

  // Re-run a single agent (e.g. one whose prompt never landed). Kills its old
  // session and spawns a fresh one server-side; the parent resumes polling.
  let retrying: Record<number, boolean> = $state({});
  async function retryAgent(index: number): Promise<void> {
    if (retrying[index]) return;
    retrying = { ...retrying, [index]: true };
    try {
      const r = await api.post<Review>(`/reviews/${review.id}/agents/${index}/retry`);
      onretried?.(r);
      toasts.info('Retrying agent…');
    } catch (e) {
      toastError('Couldn’t retry the agent', e);
    } finally {
      retrying = { ...retrying, [index]: false };
    }
  }

  // Re-run ONLY the summarize+persist stage from the stored per-agent findings
  // (cheap — no reviewer re-runs). Replaces the review's unposted draft
  // comments; the parent resumes polling via onretried.
  let retryingSummary = $state(false);
  async function retrySummarizer(): Promise<void> {
    if (retryingSummary) return;
    retryingSummary = true;
    try {
      const r = await api.post<Review>(`/reviews/${review.id}/summarizer/retry`);
      onretried?.(r);
      toasts.info('Re-running the summary…');
    } catch (e) {
      toastError('Couldn’t retry the summary', e);
    } finally {
      retryingSummary = false;
    }
  }

  // Stop a single running/waiting agent (server marks it error/"stopped by
  // user" — one click from a Retry) without cancelling the whole review.
  let stopping: Record<number, boolean> = $state({});
  async function stopAgent(index: number): Promise<void> {
    if (stopping[index]) return;
    stopping = { ...stopping, [index]: true };
    try {
      const r = await api.post<Review>(`/reviews/${review.id}/agents/${index}/stop`);
      onretried?.(r);
      toasts.info('Agent stopped');
    } catch (e) {
      toastError('Couldn’t stop the agent', e);
    } finally {
      stopping = { ...stopping, [index]: false };
    }
  }

  /** Agent finding severity → the shared Badge tone. */
  const SEV_TONE: Record<string, BadgeTone> = { info: 'info', warn: 'warn', bug: 'bad' };
;
async () => {

 { svelteHTML.createElement("div", {  "class":`rp-agents`,});view === 'done';
      for(let agent of __sveltets_2_ensureArray(rows)){let i = 1;agent.name;
     { svelteHTML.createElement("div", { "class":`rp-agent card`,});
       { svelteHTML.createElement("div", { "class":`rp-agent-top`,});
         { svelteHTML.createElement("span", { "class":`rp-agent-name`,});agent.name; }
         { svelteHTML.createElement("span", { "class":`chip rp-agent-chip`,});agent.provider;agent.model ? ' · ' + agent.model : ''; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(agent.session_id){
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => toggleTerminal(agent.session_id),});
            openTerminals.has(agent.session_id) ? 'Hide' : 'Open';
           }
        }
        if(i < lastRetryable && (agent.status === 'running' || agent.status === 'waiting')){
           { svelteHTML.createElement("button", {         "class":`btn small ghost rp-stop-btn`,"disabled":stopping[i],"onclick":() => stopAgent(i),"title":`Stop this agent (it stays retryable)`,});
            stopping[i] ? 'Stopping…' : 'Stop';
           }
        }
        if(i < lastRetryable && (agent.status === 'done' || agent.status === 'error' || agent.status === 'skipped')){
          
           { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":retrying[i] || review.status === 'running',"onclick":() => retryAgent(i),"title":review.status === 'running' ? 'Retry once the review has finished' : 'Re-run this agent',});
            retrying[i] ? 'Retrying…' : 'Retry';
           }
        } else if (view === 'done' || agent.status === 'done' || agent.status === 'error' || agent.status === 'skipped'){
           { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":retryingSummary,"onclick":retrySummarizer,"title":`Re-run only the summary over the agents' stored findings (replaces draft comments; approved/posted ones stay)`,});
            retryingSummary ? 'Retrying…' : 'Retry summary';
           }
        }
        if(agent.fallback){
           { svelteHTML.createElement("span", {     "class":`chip rp-fallback-chip`,"title":summaryFallbackTitle(summaryFallback(review.agents), 'this summary’s findings'),});
            
           }
        }
        if(agent.findings && agent.findings.length > 0){
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => toggleAgent(agent.name),});
            agentExpanded[agent.name]
              ? 'Hide'
              : `${plural(agent.findings.length, 'finding')}`;
           }
        }
         { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":agent.status}),}); { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {  "status":runStatus(agent.status),}});} }
       }
      if(agent.note && (view === 'running' || agent.status !== 'done' || i === lastRetryable)){
        
         { svelteHTML.createElement("p", {   "class":`rp-agent-note`,"title":agent.note,});agent.note; }
      }
      if(agent.status === 'waiting'){
         { svelteHTML.createElement("p", { "class":`rp-agent-waiting`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}         { svelteHTML.createElement("strong", {});  }     
               
         }
      }
      if(agent.session_id && openTerminals.has(agent.session_id)){
         { svelteHTML.createElement("div", { "class":`rp-term`,});
          
           { const $$_lanimreTyzaL3C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL3C({ target: __sveltets_2_any(), props: {    "sessionId":agent.session_id,"preferDom":true,"resumeOnOpen":false,}});}
         }
      }
      if(agentExpanded[agent.name] && agent.findings){
         { svelteHTML.createElement("ul", { "class":`rp-agent-findings`,});
          
              for(let f of __sveltets_2_ensureArray(agent.findings)){let i = 1;`${f.path ?? ''}:${f.line ?? ''}:${i}`;
             { svelteHTML.createElement("li", { "class":`rp-finding`,});
               { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":SEV_TONE[f.severity] ?? 'neutral',"label":severityLabel(f.severity),}});}
              if(f.path){ { svelteHTML.createElement("span", { "class":`mono rp-loc`,});f.path;f.line ? ':' + f.line : ''; }}
               { svelteHTML.createElement("span", { "class":`rp-finding-body`,});f.body; }
             }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReviewAgents__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReviewAgents__SvelteComponent_ = ReturnType<typeof ReviewAgents__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReviewAgents__SvelteComponent_;