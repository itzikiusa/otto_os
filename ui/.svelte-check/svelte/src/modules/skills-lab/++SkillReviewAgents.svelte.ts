///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Per-agent results block for a Skills Lab review — the same embedded-terminal
// pattern as the code-review ReviewAgents: Open mounts a live <Terminal> for
// the agent's session inline (multiple can be open at once), Retry re-runs one
// reviewer, and each agent's own findings expand below its row. The summarizer
// is the trailing row (not retryable; its aggregate renders in the panel).
import type { SkillReview } from '../../lib/api/types';
import { toastError } from '../../lib/toastError';
import { skillReviewApi } from '../../lib/api/skillReview';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus, type BadgeTone } from '../../lib/status';
import { severityLabel } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
;

  interface Props {
    review: SkillReview;
    view: 'running' | 'done';
    onretried?: (review: SkillReview) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { review, view, onretried }: Props = $props();

  // Running: show every agent incl. the summarizer. Done: drop the summarizer
  // (its aggregate renders separately as the summary report).
  const rows = $derived(view === 'done' ? review.agents.slice(0, -1) : review.agents);
  const lastRetryable = $derived(review.agents.length - 1);

  let openTerminals = $state<Set<string>>(new Set());
  function toggleTerminal(sessionId: string | null | undefined): void {
    if (!sessionId) return;
    const next = new Set(openTerminals);
    if (next.has(sessionId)) next.delete(sessionId);
    else next.add(sessionId);
    openTerminals = next;
  }

  let agentExpanded: Record<string, boolean> = $state({});
  function toggleAgent(name: string): void {
    agentExpanded = { ...agentExpanded, [name]: !agentExpanded[name] };
  }

  let retrying: Record<number, boolean> = $state({});
  async function retryAgent(index: number): Promise<void> {
    if (retrying[index]) return;
    retrying = { ...retrying, [index]: true };
    try {
      const r = await skillReviewApi.retryAgent(review.id, index);
      onretried?.(r);
      toasts.info('Retrying agent…');
    } catch (e) {
      toastError('Couldn’t re-run the agent', e);
    } finally {
      retrying = { ...retrying, [index]: false };
    }
  }

  // Review severity → the shared Badge tone + wording (lib/labels severityLabel).
  const SEV_TONE: Record<string, BadgeTone> = { critical: 'bad', high: 'bad', medium: 'warn', low: 'info' };
  function sevTone(sev: string): BadgeTone {
    return SEV_TONE[sev.toLowerCase()] ?? 'neutral';
  }
;
async () => {

 { svelteHTML.createElement("div", {  "class":`rp-agents`,});view === 'done';
      for(let agent of __sveltets_2_ensureArray(rows)){let i = 1;agent.name;
     { svelteHTML.createElement("div", { "class":`rp-agent card`,});
       { svelteHTML.createElement("div", { "class":`rp-agent-top`,});
         { svelteHTML.createElement("span", {   "class":`rp-agent-name`,"title":agent.name,});agent.name; }
         { svelteHTML.createElement("span", { "class":`chip rp-agent-chip`,});agent.provider;agent.model ? ' · ' + agent.model : ''; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(agent.session_id){
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"aria-expanded":openTerminals.has(agent.session_id),"onclick":() => toggleTerminal(agent.session_id),});
            openTerminals.has(agent.session_id) ? 'Hide session' : 'Open session';
           }
        }
        if(i < lastRetryable && agent.name !== 'summarizer'){
           { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":retrying[i],"onclick":() => retryAgent(i),"title":`Re-run this agent`,});
            retrying[i] ? 'Retrying…' : 'Retry';
           }
        }
        if(agent.findings && agent.findings.length > 0){
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"aria-expanded":!!agentExpanded[agent.name],"onclick":() => toggleAgent(agent.name),});
            agentExpanded[agent.name] ? 'Hide findings' : `${plural(agent.findings.length, 'finding')}`;
           }
        }
         { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":agent.status}),}); { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {  "status":runStatus(agent.status),}});} }
       }
      if(agent.note && (view === 'running' || agent.status !== 'done')){
         { svelteHTML.createElement("p", { "class":`rp-agent-note`,});agent.note; }
      }
      if(agent.status === 'waiting'){
         { svelteHTML.createElement("p", { "class":`rp-agent-waiting`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}        { svelteHTML.createElement("strong", {});  }  
         }
      }
      if(agent.session_id && openTerminals.has(agent.session_id)){
         { svelteHTML.createElement("div", { "class":`rp-term`,});
          
           { const $$_lanimreTyzaL3C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL3C({ target: __sveltets_2_any(), props: {    "sessionId":agent.session_id,"preferDom":true,"resumeOnOpen":false,}});}
         }
      }
      if(agentExpanded[agent.name] && agent.findings){
         { svelteHTML.createElement("ul", { "class":`rp-agent-findings`,});
              for(let f of __sveltets_2_ensureArray(agent.findings)){let i = 1;i + f.code;
             { svelteHTML.createElement("li", { "class":`rp-finding`,});
               { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":sevTone(f.severity),"label":severityLabel(f.severity.toLowerCase()),}});}
              if(f.code){ { svelteHTML.createElement("span", { "class":`mono rp-loc`,});f.code; }}
               { svelteHTML.createElement("span", { "class":`rp-finding-body`,}); { svelteHTML.createElement("strong", {});f.title; }f.fix ? ` — ${f.fix}` : ''; }
             }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillReviewAgents__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillReviewAgents__SvelteComponent_ = ReturnType<typeof SkillReviewAgents__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillReviewAgents__SvelteComponent_;