///<reference types="svelte" />
;// Activity tab: what the agent is doing NOW (running run, its session's
// state, approvals it waits on) and a merged timeline of its tool calls and
// runs. Event-fed (personal_agent_activity / run updates / approval
// changes); polls only while the event socket is down.

import { onDestroy, untrack } from 'svelte';
import { personalAgentsApi } from '../../lib/api/personalAgents';
import { mcpCpApi } from '../../lib/api/mcp';
import { liveQuery } from '../../lib/live';
import type { Poller } from '../../lib/poll';
import { ws } from '../../lib/stores/workspace.svelte';
import { personalAgents } from '../../lib/stores/personalAgents.svelte';
import { router } from '../../lib/router.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { loadErrorText } from '../../lib/loadError';
import { toastError } from '../../lib/toastError';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus, sessionState } from '../../lib/status';
import { approvalConcernsFeed, mergeActivity } from './activityFeed';
import type {
    PersonalAgentActivity,
    PersonalAgentActivityApproval,
    PersonalAgentActivityItem,
    PersonalAgentRun,
  } from '../../lib/api/types';
;

  interface Props {
    agentId: string;
  };;

  type Entry =
    | { key: string; at: string; kind: 'item'; item: PersonalAgentActivityItem }
    | { key: string; at: string; kind: 'run'; run: PersonalAgentRun };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { agentId }: Props = $props();

  let data = $state<PersonalAgentActivity | null>(null);
  let error = $state<string | null>(null);
  // Incremental refresh (perf W4): a tool-call event fetches only ring
  // entries after the cursor and no run history; run history is re-read when
  // a run changed, on the first load, and at least once a minute.
  let wantRuns = true;
  let lastFull = 0;
  const FULL_EVERY_MS = 60_000;

  async function load(): Promise<boolean> {
    try {
      const full = data === null || wantRuns || Date.now() - lastFull > FULL_EVERY_MS;
      wantRuns = false;
      const next = await personalAgentsApi.activity(
        agentId,
        data === null ? {} : { afterSeq: data.seq ?? 0, epoch: data.epoch, runs: full },
      );
      if (full) lastFull = Date.now();
      data = mergeActivity(data, next);
      error = null;
      return true;
    } catch (e) {
      error = loadErrorText(e);
      return false;
    }
  }

  let poll: Poller | null = null;
  $effect(() => {
    const id = agentId;
    untrack(() => {
      poll?.stop();
      data = null;
      wantRuns = true;
      poll = liveQuery({
        run: () => load(),
        on: ['personal_agent_activity', 'personal_agent_run_updated', 'mcp_approval_changed'],
        match: (ev) => {
          if (ev.type === 'mcp_approval_changed') return approvalConcernsFeed(data, ev.approval_id);
          if (ev.agent_id !== id) return false;
          if (ev.type === 'personal_agent_run_updated') wantRuns = true;
          return true;
        },
        fallbackMs: 10_000,
        debounceMs: 400,
      });
    });
  });
  onDestroy(() => poll?.stop());


  const timeline = $derived.by<Entry[]>(() => {
    if (!data) return [];
    const out: Entry[] = [
      ...data.items
        // Waiting approvals have their own panel above.
        .filter((i) => i.kind !== 'approval_waiting')
        .map((item) => ({ key: `i${item.seq}`, at: item.at, kind: 'item' as const, item })),
      ...(data.runs ?? []).map((run) => ({ key: `r${run.id}`, at: run.started_at, kind: 'run' as const, run })),
    ];
    return out.sort((a, b) => Date.parse(b.at) - Date.parse(a.at)).slice(0, 120);
  });
  const waiting = $derived((data?.approvals ?? []).filter((a) => a.status === 'pending'));

  const ITEM_ICON: Record<PersonalAgentActivityItem['kind'], IconName> = {
    tool_call: 'zap',
    blocked: 'lock',
    approval_required: 'hand',
    approval_waiting: 'clock',
  };

  function modeLabel(r: PersonalAgentRun): string {
    const m = r.mode ?? 'directed';
    if (m === 'proactive') return 'Proactive';
    if (m === 'scheduled') return 'Scheduled';
    return 'Directed';
  }

  function sessionLabel(raw: string): string {
    switch (raw) {
      case 'idle':
      case 'working':
      case 'running':
      case 'exited':
      case 'reconnectable':
        return sessionState(null, raw).label;
      default:
        return runStatus(raw).label;
    }
  }

  const agentName = $derived(personalAgents.agent(agentId)?.name ?? 'This agent');
  let deciding = $state<string | null>(null);
  let decidingAs = $state<'approve' | 'deny' | null>(null);
  let stopping = $state(false);

  function approvalWhat(a: PersonalAgentActivityApproval): string {
    return a.title?.trim() || `otto.${a.tool}`;
  }

  async function stopRun(run: PersonalAgentRun): Promise<void> {
    const when = new Date(run.started_at).toLocaleString();
    const name = run.summary?.trim() || `${modeLabel(run)} run`;
    const ok = await confirmer.ask(
      `Stop “${name}”, started ${when}? Its agent session is stopped and nothing is delivered.`,
      { title: `Stop ${name}?`, danger: true, confirmLabel: 'Stop run' },
    );
    if (!ok) return;
    stopping = true;
    try {
      await personalAgentsApi.cancelRun(run.id);
    } catch (e) {
      toastError('Couldn’t stop the run', e);
    } finally {
      stopping = false;
    }
  }

  async function decide(a: PersonalAgentActivityApproval, approved: boolean): Promise<void> {
    const what = approvalWhat(a);
    if (!approved) {
      const ok = await confirmer.ask(
        `Deny “${what}” from ${agentName}? The tool will not run.`,
        { title: 'Deny this?', confirmLabel: 'Deny', danger: true },
      );
      if (!ok) return;
    }
    deciding = a.approval_id;
    decidingAs = approved ? 'approve' : 'deny';
    try {
      await mcpCpApi.cpDecide(a.approval_id, { approved });
      await load();
    } catch (e) {
      toastError(approved ? 'Couldn’t approve' : 'Couldn’t deny', e);
    } finally {
      deciding = null;
      decidingAs = null;
    }
  }
;
async () => {

 { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`this agent’s activity`,"loading":data === null && !error,"error":data === null ? error : null,"rows":4,"onretry":() => void poll?.now(),}});
  if(data){
     { svelteHTML.createElement("section", {   "class":`pa-panel now`,"aria-labelledby":`now-h`,});
       { svelteHTML.createElement("h2", { "id":`now-h`,});  }
      if(data.now.run){
        const run = data.now.run;
         { svelteHTML.createElement("div", { "class":`now-row`,});
           { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {    "status":runStatus(run.status),"variant":`text`,}});}
           { svelteHTML.createElement("span", { "class":`chip`,});modeLabel(run); }
          if(run.read_only){ { svelteHTML.createElement("span", { "class":`chip pa-ichip`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}  }}
           { svelteHTML.createElement("span", { "class":`meta`,});  { const $$_emiTleR4C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR4C({ target: __sveltets_2_any(), props: {  "iso":run.started_at,}});} }
          if(data.now.session_status){ { svelteHTML.createElement("span", { "class":`meta`,}); sessionLabel(data.now.session_status); }}
          if(run.session_id){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => ws.navigateToSession(run.session_id ?? ''),});  }
          }
           { svelteHTML.createElement("button", {       "class":`btn small danger`,"disabled":stopping,"aria-label":`Stop ${run.summary?.trim() || modeLabel(run)}`,"onclick":() => void stopRun(run),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} stopping ? 'Stopping…' : 'Stop…';
           }
         }
      }else{
         { svelteHTML.createElement("p", { "class":`muted`,});      }
      }
      if(waiting.length > 0){
         { svelteHTML.createElement("h3", {});   }
         { svelteHTML.createElement("ul", { "class":`list`,});
             for(let a of __sveltets_2_ensureArray(waiting)){a.approval_id;
             { svelteHTML.createElement("li", { "class":`wait`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`hand`,"size":14,}});}
               { svelteHTML.createElement("div", { "class":`grow`,});
                 { svelteHTML.createElement("strong", {});approvalWhat(a); }
                 { svelteHTML.createElement("p", { "class":`detail`,});agentName;       a.tool;  }
                if(a.detail){ { svelteHTML.createElement("p", { "class":`detail`,});a.detail; }}
                if(a.risk_label === 'sensitive'){ { svelteHTML.createElement("span", { "class":`chip pa-ichip pa-warn`,});  }} else if (a.risk_label === 'agent_rule'){ { svelteHTML.createElement("span", { "class":`chip`,});  }}
                 { svelteHTML.createElement("div", { "class":`wait-actions`,});
                   { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":deciding === a.approval_id,"aria-label":`Deny ${approvalWhat(a)}`,"onclick":() => void decide(a, false),});deciding === a.approval_id && decidingAs === 'deny' ? 'Denying…' : 'Deny'; }
                   { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":deciding === a.approval_id,"aria-label":`Approve ${approvalWhat(a)}`,"onclick":() => void decide(a, true),});
                    deciding === a.approval_id && decidingAs === 'approve' ? 'Approving…' : 'Approve';
                   }
                   { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go('mcp/activity'),});  }
                 }
               }
               { svelteHTML.createElement("span", { "class":`meta`,}); { const $$_emiTleR5C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR5C({ target: __sveltets_2_any(), props: {  "iso":a.at,}});} }
             }
          }
         }
         { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go('mcp/activity'),});  }
      }
     }

     { svelteHTML.createElement("section", {   "class":`pa-panel`,"aria-labelledby":`tl-h`,});
       { svelteHTML.createElement("h2", { "id":`tl-h`,});  }
      if(timeline.length === 0){
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`radar`,"title":`Nothing yet`,"body":`Tool calls, blocked actions, approvals and runs show up here as they happen.`,}});}
      }else{
         { svelteHTML.createElement("ol", { "class":`list timeline`,});
             for(let e of __sveltets_2_ensureArray(timeline)){e.key;
            if(e.kind === 'item'){
               { svelteHTML.createElement("li", {   "class":`ev`,});e.item.kind === 'blocked';e.item.kind === 'approval_required';
                 { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":ITEM_ICON[e.item.kind],"size":13,}});}
                 { svelteHTML.createElement("span", { "class":`mono`,}); e.item.tool; }
                 { svelteHTML.createElement("span", { "class":`detail grow`,});e.item.kind === 'tool_call' ? 'called' : e.item.detail; }
                 { svelteHTML.createElement("span", { "class":`meta`,}); { const $$_emiTleR5C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR5C({ target: __sveltets_2_any(), props: {  "iso":e.at,}});} }
               }
            }else{
               { svelteHTML.createElement("li", { "class":`ev run`,});
                 { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":13,}});}
                 { svelteHTML.createElement("span", {});modeLabel(e.run);  }
                 { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {    "status":runStatus(e.run.status),"variant":`text`,}});}
                if(e.run.read_only){ { svelteHTML.createElement("span", { "class":`chip pa-ichip`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}  }}
                 { svelteHTML.createElement("span", {   "class":`detail grow`,"title":e.run.summary || e.run.error || undefined,});e.run.summary || e.run.error || ''; }
                 { svelteHTML.createElement("span", { "class":`meta`,}); { const $$_emiTleR5C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR5C({ target: __sveltets_2_any(), props: {  "iso":e.at,}});} }
               }
            }
          }
         }
         { svelteHTML.createElement("p", { "class":`hint`,});                 }
      }
     }
  }
 LoadState}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentActivity__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentActivity__SvelteComponent_ = ReturnType<typeof AgentActivity__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentActivity__SvelteComponent_;