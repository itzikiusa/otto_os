///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
// All runs/iterations as a filterable list (per assignee / project / status).
import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus, sentenceCase } from '../../lib/status';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import RunInspector from './RunInspector.svelte';
import VirtualList from '../../lib/components/VirtualList.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { formatCount } from '../../lib/metric-format';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { RecruitedAgent, RunStatus, SwarmRun } from './types';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { canOpenRunSession, RUN_SESSION_NOT_YOURS } from './access';

;type $$ComponentProps =  { onhire?: (p: RecruitedAgent, runId: string) => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // `onhire` lets a completed recruit run open the Recruiter wizard pre-filled
  // with its proposal — so a candidate isn't lost if the modal was closed.
  let { onhire }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let agentFilter = $state<string>('');
  let projectFilter = $state<string>('');
  let statusFilter = $state<string>('');
  let inspecting = $state<SwarmRun | null>(null);

  // Keep the open inspector in sync with live WS run updates (tokens land on the
  // terminal patch, after the inspector may already be open).
  const live = $derived(inspecting ? (swarm.runs.find((r) => r.id === inspecting!.id) ?? inspecting) : null);

  // VirtualList assumes uniform rows, so the phone card row has a fixed height
  // (see the ≤640px block below).
  const ROW_H_PHONE = 104;

  // access-control-6-04: a run's session is the acting user's; offer "Open"
  // only for one in my (owner-scoped) store, or to root / a ws admin.
  const myRole = $derived(ws.workspaces.find((w) => w.id === swarm.detail?.workspace_id)?.my_role);
  const canOpen = (sid: string) =>
    canOpenRunSession({ isRoot: auth.isRoot, myRole }, !!ws.getSession(sid));

  const STATUSES: RunStatus[] = ['queued', 'running', 'waiting', 'done', 'error', 'stopped'];

  const filtered = $derived(
    swarm.runs.filter(
      (r) =>
        (!agentFilter || r.agent_id === agentFilter) &&
        (!projectFilter || r.project_id === projectFilter) &&
        (!statusFilter || r.status === statusFilter),
    ),
  );

  const filtering = $derived(!!(agentFilter || projectFilter || statusFilter));

  // Stopping can't be undone (the agent's session is closed and its work in
  // progress is lost; a new run starts over), so it asks first — the same rule
  // as Abort all, a workflow's Cancel run and a goal loop's Stop. A failed stop
  // must surface instead of rejecting silently (the row would just keep
  // showing Running with no explanation).
  async function stop(r: SwarmRun) {
    const who = swarm.agentById(r.agent_id)?.name ?? 'this agent';
    const ok = await confirmer.ask(
      `Stop ${who}'s ${sentenceCase(r.kind).toLowerCase()} run? Its session is closed and work in progress is lost.`,
      { title: 'Stop run', confirmLabel: 'Stop run', danger: true },
    );
    if (!ok) return;
    try {
      await swarm.stopRun(r.id);
    } catch (e) {
      toastError("Couldn’t stop the run", e);
    }
  }

  const reload = () => swarm.detail && swarm.loadRuns({ swarm_id: swarm.detail.id });

  function tokenTitle(r: SwarmRun): string {
    return `${(r.tokens_input ?? 0).toLocaleString()} input · ${(r.tokens_output ?? 0).toLocaleString()} output tokens`;
  }

  function active(r: SwarmRun): boolean {
    return r.status === 'queued' || r.status === 'running' || r.status === 'waiting';
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`runs`,});
   { svelteHTML.createElement("div", { "class":`filters`,});
     { svelteHTML.createElement("select", {     "class":`input small`,"aria-label":`Filter by assignee`,"bind:value":agentFilter,});/*Ωignore_startΩ*/() => agentFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/
       { svelteHTML.createElement("option", {"value":"",});  }
         for(let a of __sveltets_2_ensureArray(swarm.detail?.agents ?? [])){a.id;
         { svelteHTML.createElement("option", { "value":a.id,});a.name; }
      }
     }
     { svelteHTML.createElement("select", {     "class":`input small`,"aria-label":`Filter by project`,"bind:value":projectFilter,});/*Ωignore_startΩ*/() => projectFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/
       { svelteHTML.createElement("option", {"value":"",});  }
         for(let p of __sveltets_2_ensureArray(swarm.detail?.projects ?? [])){p.id;
         { svelteHTML.createElement("option", { "value":p.id,});p.name; }
      }
     }
     { svelteHTML.createElement("div", {     "class":`chips`,"role":`group`,"aria-label":`Filter by status`,});
       { svelteHTML.createElement("button", {      "class":`chip`,"aria-pressed":statusFilter === '',"onclick":() => (statusFilter = ''),});statusFilter === '';  }
         for(let s of __sveltets_2_ensureArray(STATUSES)){s;
         { svelteHTML.createElement("button", {      "class":`chip`,"aria-pressed":statusFilter === s,"onclick":() => (statusFilter = s),});statusFilter === s;runStatus(s).label; }
      }
     }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":reload,"aria-label":`Refresh runs`,"title":`Refresh runs`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
     }
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`runs`,"loading":swarm.runsLoading,"error":swarm.runsError,"empty":swarm.runs.length === 0,"onretry":reload,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":`No runs yet`,"body":`Runs appear here as agents work tasks. Start one from the Board (Run now) or an agent’s menu in Org.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
    if(filtered.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {           "icon":`clock`,"title":`No runs match these filters`,"body":`Clear the assignee, project or status filter to see all ${swarm.runs.length} runs.`,"actionLabel":`Clear filters`,"onaction":() => { agentFilter = ''; projectFilter = ''; statusFilter = ''; },}});}
    }else{
       { svelteHTML.createElement("div", { "class":`table`,});
         { svelteHTML.createElement("div", { "class":`thead`,});
           { svelteHTML.createElement("span", { "class":`c-agent`,});  }
           { svelteHTML.createElement("span", { "class":`c-kind`,});  }
           { svelteHTML.createElement("span", { "class":`c-status`,});  }
           { svelteHTML.createElement("span", { "class":`c-time`,});  }
           { svelteHTML.createElement("span", {   "class":`c-tok`,"title":`Input / output tokens`,});  }
           { svelteHTML.createElement("span", { "class":`c-act`,}); }
         }
         { const $$_tsiLlautriV3C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV3 = new $$_tsiLlautriV3C({ target: __sveltets_2_any(), props: {     "items":filtered,"estimateHeight":viewport.isPhone ? ROW_H_PHONE : 37,"class":`vlist-runs`,row:(r: SwarmRun) => { async ()/*Ωignore_positionΩ*/ => {
            const agent = swarm.agentById(r.agent_id);
            
             { svelteHTML.createElement("div", { "class":`trow`,});
               { svelteHTML.createElement("span", {   "class":`c-agent`,"title":agent ? `${agent.name}${agent.title ? ` — ${agent.title}` : ''}` : r.agent_id,});
                 { svelteHTML.createElement("span", { "class":`agent-name`,});agent?.name ?? r.agent_id.slice(0, 6); }
                if(agent?.title){ { svelteHTML.createElement("span", { "class":`agent-title dim`,});agent.title; }}
               }
               { svelteHTML.createElement("button", {       "class":`c-kind dim run-open`,"title":r.summary ? `${sentenceCase(r.kind)} · ${r.summary}` : sentenceCase(r.kind),"onclick":() => (inspecting = r),"aria-label":`Inspect ${agent?.name ?? 'agent'}'s ${sentenceCase(r.kind).toLowerCase()} run`,});sentenceCase(r.kind);r.summary ? ` · ${r.summary}` : ''; }
               { svelteHTML.createElement("span", { "class":`c-status`,}); { const $$_egdaBsutatS2C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS2C({ target: __sveltets_2_any(), props: {  "status":runStatus(r.status),}});} }
               { svelteHTML.createElement("span", { "class":`c-time dim`,});rel(r.started_at ?? r.enqueued_at ?? ''); }
               { svelteHTML.createElement("span", {   "class":`c-tok mono dim`,"title":r.tokens_input != null || r.tokens_output != null ? tokenTitle(r) : 'No usage recorded',});
                r.tokens_input != null || r.tokens_output != null
                  ? `${formatCount(r.tokens_input ?? 0)}/${formatCount(r.tokens_output ?? 0)}`
                  : '—';
               }
               { svelteHTML.createElement("span", { "class":`c-act`,});
                if(r.session_id && canOpen(r.session_id)){
                   { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Open ${agent?.name ?? 'this agent'}’s session beside the view`,"aria-label":`Open ${agent?.name ?? 'this agent'}’s ${sentenceCase(r.kind).toLowerCase()} session`,"onclick":() => (swarm.selectedSessionId = r.session_id!),});
                     { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":14,}});}
                   }
                } else if (r.session_id){
                   { svelteHTML.createElement("button", {     "class":`icon-btn`,"disabled":true,"title":RUN_SESSION_NOT_YOURS,"aria-label":`${agent?.name ?? 'This agent'}’s session: ${RUN_SESSION_NOT_YOURS}`,});
                     { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":14,}});}
                   }
                }
                if(r.kind === 'recruit' && r.status === 'done' && r.result && onhire && !swarm.recruitHired.has(r.id)){
                   { svelteHTML.createElement("button", {       "class":`btn small primary`,"title":`Review and hire ${(r.result as { name?: string }).name || agent?.name || 'this candidate'}`,"aria-label":`Hire ${(r.result as { name?: string }).name || agent?.name || 'this candidate'}`,"onclick":() => onhire?.(r.result as unknown as RecruitedAgent, r.id),});  }
                }
                if(active(r)){
                   { svelteHTML.createElement("button", {       "class":`btn small danger`,"title":`Stop ${agent?.name ?? 'this agent'}’s ${sentenceCase(r.kind).toLowerCase()} run`,"aria-label":`Stop ${agent?.name ?? 'this agent'}’s ${sentenceCase(r.kind).toLowerCase()} run`,"onclick":() => void stop(r),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  }
                }
               }
             }
          };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV3.$$prop_def;/*Ωignore_endΩ*/
          
         VirtualList}
       }
    }
   LoadState}
 }

if(live){
   { const $$_rotcepsnInuR0C = __sveltets_2_ensureComponent(RunInspector); new $$_rotcepsnInuR0C({ target: __sveltets_2_any(), props: {    "run":live,"onclose":() => (inspecting = null),}});}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunsList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunsList__SvelteComponent_ = ReturnType<typeof RunsList__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunsList__SvelteComponent_;