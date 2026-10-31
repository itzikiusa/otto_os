///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
// The approval queue — dangerous tool calls and `otto.ask_human_approval`
// requests waiting on a human. Shows the redacted args (never the full/secret
// values; the server binds the hash of the FULL args). Approve/Deny with the
// card's one optional note (Deny does not ask a second time). The requester
// cannot approve their own DIRECT request (enforced server-side) — but a request raised by their own agent session /
// MCP client on their behalf is exactly what they're meant to decide, since
// every Otto session authorizes as its owner (mirrors
// `AGENT_REQUESTER_KINDS` in crates/otto-state/src/mcp_control.rs). Polls
// every few seconds and on tab refocus so a new pending request appears
// without a manual reload. "Approve & always allow…" on an `otto.*` tool
// call opens the auto-approve rule form prefilled with that tool and the
// requesting session (else its workspace), then approves this request.
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Icon from '../../lib/components/Icon.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
import ApprovalOutcome from '../../lib/components/ApprovalOutcome.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import { runStateLabel, sentenceCase } from '../../lib/labels';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { mcpCpApi } from '../../lib/api/mcp';
import { liveQuery } from '../../lib/live';
import { toasts } from '../../lib/toast.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { rel } from '../../lib/stores/now.svelte';
import type { McpApproval, McpAutoApproveList } from '../../lib/api/types';
import McpPill from './McpPill.svelte';
import AutoApproveForm from './AutoApproveForm.svelte';
import { mcpCpExtraApi } from './cp-api';
import NeedsYouLink from '../home/LazyNeedsYouLink.svelte';
import { userLabel } from '../../lib/userLabel';
import type { MemberEntry } from '../../lib/api/types';

;type $$ComponentProps =  { ondecided?: () => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let approvals = $state<McpApproval[]>([]);
  const AGENT_REQUESTER_KINDS = ['mcp_server', 'gateway', 'agent'];
  const requesterMayDecide = (a: McpApproval): boolean => AGENT_REQUESTER_KINDS.includes(a.requested_by_kind ?? '');
  const canDecide = (a: McpApproval): boolean => {
    // Not your own direct request (an agent raising it on your behalf is fine)…
    const allowedRequester = a.requested_by !== auth.me?.id || requesterMayDecide(a);
    // …and approve access on the server (or MCP admin for server-less asks).
    const hasAccess = a.server_id
      ? resourceAccess.can('mcp_server', a.server_id, 'approve', 'mcp', 'admin', a.tool ?? undefined)
      : auth.can('mcp', 'admin');
    return allowedRequester && hasAccess;
  };
  $effect(() => {
    for (const approval of approvals) {
      if (approval.server_id) void resourceAccess.load('mcp_server', approval.server_id, approval.tool ?? undefined);
    }
  });
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let busy = $state<Record<string, 'approve' | 'deny'>>({});
  let notes = $state<Record<string, string>>({});
  let showAll = $state(false);
  /** Told after a decision so the page header's pending badge updates now, not on its next poll. */
  let { ondecided }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  /** Why Approve/Deny is greyed out — a disabled button never goes unexplained. */
  const decideBlockedReason = (a: McpApproval): string =>
    a.requested_by === auth.me?.id && !requesterMayDecide(a)
      ? 'You raised this request yourself — another admin has to decide it'
      : 'You don’t have approve access to this server';

  /** The approval whose "always allow" rule form is open, with the catalog it needs. */
  let allowFor = $state<{ approval: McpApproval; catalog: McpAutoApproveList['categories'] } | null>(null);
  let allowLoading = $state<string | null>(null);
  /** Only governed `otto.*` tool calls can be covered by an auto-approve rule. */
  const canAlwaysAllow = (a: McpApproval): boolean =>
    a.kind === 'tool_call' && !a.server_id && !!a.tool?.startsWith('otto.') && canDecide(a);
  const sessionTitle = (id: string): string | null =>
    ws.getSession(id)?.title ?? null;
  /** Members of every workspace an approval came from, fetched once per
   *  workspace: the names behind "requested by" / "decided by" (an id is never
   *  primary text — the full one stays in a `title`). A failed fetch leaves the
   *  shortened id. */
  let members = $state<MemberEntry[]>([]);
  const membersFetched = new Set<string>();
  $effect(() => {
    for (const a of approvals) {
      const w = a.workspace_id;
      if (!w || membersFetched.has(w)) continue;
      membersFetched.add(w);
      mcpCpExtraApi
        .members(w)
        .then((m) => (members = [...members, ...m]))
        .catch(() => {});
    }
  });
  const who = (id: string | null | undefined): string => userLabel(id, auth.me?.id, members);
  const requesterLabel = (a: McpApproval): string => who(a.requested_by);

  async function openAlwaysAllow(a: McpApproval): Promise<void> {
    allowLoading = a.id;
    try {
      const list = await mcpCpExtraApi.autoApproveRules();
      allowFor = { approval: a, catalog: list.categories };
    } catch (e) {
      toastError('Couldn’t load the auto-approve catalog', e);
    } finally {
      allowLoading = null;
    }
  }

  function openSession(id: string): void {
    ws.navigateToSession(id);
  }

  let loadSeq = 0;
  async function load(): Promise<boolean> {
    const seq = ++loadSeq;
    loading = true;
    try {
      const list = await mcpCpApi.cpApprovals(showAll ? undefined : 'pending');
      if (seq !== loadSeq) return true; // a newer load (filter flip) owns the view
      approvals = list;
      loadError = null;
      return true;
    } catch (e) {
      if (seq === loadSeq) loadError = loadErrorText(e);
      return false;
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  // Event-fed: reload when `mcp_approval_changed` says the list changed (plus
  // a safety net / reconnect resync); the old 5 s poll runs only while the
  // event socket is down. Same chain rules (no overlap, paused while hidden,
  // backoff), and immediately whenever the window regains focus.
  $effect(() => {
    void showAll; // re-load when the filter flips
    const p = liveQuery({ run: () => load(), on: ['mcp_approval_changed'], fallbackMs: 5000 });
    const onFocus = (): void => p.now();
    window.addEventListener('focus', onFocus);
    return () => {
      p.stop();
      window.removeEventListener('focus', onFocus);
    };
  });

  async function decide(a: McpApproval, approved: boolean, reason?: string | null): Promise<void> {
    if (!canDecide(a)) return;
    busy = { ...busy, [a.id]: approved ? 'approve' : 'deny' };
    try {
      await mcpCpApi.cpDecide(a.id, { approved, note: reason?.trim() || notes[a.id]?.trim() || null });
      toasts.success(approved ? 'Approved' : 'Denied', a.title);
      ondecided?.();
      await load();
    } catch (e) {
      toastError('Couldn’t record the decision', e);
    } finally {
      const n = { ...busy };
      delete n[a.id];
      busy = n;
    }
  }

  /** Who raised the request, in words (`mcp_server` → “MCP server”). */
  const requesterKindLabel = (kind: string): string =>
    ({ mcp_server: 'MCP server', gateway: 'Gateway', agent: 'Agent', user: 'Person' })[kind] ?? sentenceCase(kind);

  function prettyArgs(json: string): string {
    try {
      return JSON.stringify(JSON.parse(json), null, 2);
    } catch {
      return json;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`appr`,...__sveltets_2_empty({"data-testid":`mcp-approvals`}),});
   { svelteHTML.createElement("div", { "class":`bar`,});
     { svelteHTML.createElement("h2", {});  }
     { svelteHTML.createElement("span", {  "class":`count chip`,});!showAll && approvals.length > 0;approvals.length; showAll ? 'total' : 'pending'; }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    
     { const $$_kniLuoYsdeeN2C = __sveltets_2_ensureComponent(NeedsYouLink); new $$_kniLuoYsdeeN2C({ target: __sveltets_2_any(), props: {}});}
     { svelteHTML.createElement("label", { "class":`check`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showAll,});/*Ωignore_startΩ*/() => showAll = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", {});   }
     }
     { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void load(),"title":`Refresh`,"aria-label":`Refresh approvals`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
   }

  
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`approvals`,loading,"error":loadError,"empty":approvals.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`empty`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":24,}});}
         { svelteHTML.createElement("p", {});showAll ? 'No approvals yet.' : 'Nothing waiting on you — the queue is clear.'; }
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`list`,});
         for(let a of __sveltets_2_ensureArray(approvals)){a.id;
         { svelteHTML.createElement("div", { "class":`card`,});
           { svelteHTML.createElement("div", { "class":`chead`,});
             { svelteHTML.createElement("span", { "class":`chip`,});a.kind === 'human_ask' ? 'Human ask' : 'Tool call'; }
             { svelteHTML.createElement("span", { "class":`title`,});a.title; }
            if(a.risk_label){  { const $$_lliPpcM5C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM5C({ target: __sveltets_2_any(), props: {    "kind":`risk`,"value":a.risk_label,"small":true,}});}}
              { const $$_lliPpcM5C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM5C({ target: __sveltets_2_any(), props: {    "kind":`status`,"value":a.status,"small":true,}});}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("span", {   "class":`when`,"title":new Date(a.created_at).toLocaleString(),});rel(a.created_at); }
           }
           { svelteHTML.createElement("div", { "class":`meta`,});
            if(a.server_name || a.tool){
               { svelteHTML.createElement("span", { "class":`route mono`,});a.server_name ?? '—';a.tool ? ` → ${a.tool}` : ''; }
            }
            if(a.requested_by_session_id){
              const sid = a.requested_by_session_id;
               { svelteHTML.createElement("span", {   "class":`by`,"title":a.requested_by ?? undefined,});
                 
                 { svelteHTML.createElement("button", {         "class":`link`,"type":`button`,"onclick":() => openSession(sid),"title":`Open the requesting session`,...__sveltets_2_empty({"data-testid":`mcp-approval-session`}),});
                  sessionTitle(sid) ?? `${sid.slice(0, 8)}…`;
                 }
                
                if(requesterMayDecide(a)){ { const $$_enilyBtnegA6C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA6C({ target: __sveltets_2_any(), props: {  "name":requesterLabel(a),}});}}else{requesterLabel(a);}
               }
            } else if (a.requested_by){
               { svelteHTML.createElement("span", {   "class":`by`,"title":a.requested_by,});
                 
                if(requesterMayDecide(a)){ { const $$_enilyBtnegA6C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA6C({ target: __sveltets_2_any(), props: {  "name":requesterLabel(a),}});}}else{requesterLabel(a);}
                a.requested_by_kind ? ` (${requesterKindLabel(a.requested_by_kind)})` : '';
               }
            }
            if(a.expires_at){ { svelteHTML.createElement("span", {   "class":`by`,"title":new Date(a.expires_at).toLocaleString(),}); rel(a.expires_at); }}
           }
          if(a.detail){ { svelteHTML.createElement("p", { "class":`detail`,});a.detail; }}
          if(a.args_redacted_json && a.args_redacted_json !== '{}'){
             { svelteHTML.createElement("pre", { "class":`args`,});prettyArgs(a.args_redacted_json); }
          }

          if(a.status === 'pending'){
             { svelteHTML.createElement("div", { "class":`actions`,});
               { svelteHTML.createElement("input", {           "class":`note`,"placeholder":`Checked the arguments, looks safe`,"aria-label":`Decision note (optional)`,"value":notes[a.id] ?? '',"oninput":(e) => (notes = { ...notes, [a.id]: (e.currentTarget as HTMLInputElement).value }),});}
               { const $$_snoitcAlavorppA5C = __sveltets_2_ensureComponent(ApprovalActions); const $$_snoitcAlavorppA5 = new $$_snoitcAlavorppA5C({ target: __sveltets_2_any(), props: {               "busy":busy[a.id] ?? null,"disabled":!canDecide(a),"disabledReason":decideBlockedReason(a),"denyTarget":`this ${a.kind === 'human_ask' ? 'request' : 'tool call'}`,"askReason":false,"onapprove":() => decide(a, true),"ondeny":(reason) => decide(a, false, reason),extra:() => { async ()/*Ωignore_positionΩ*/ => {
                  if(canAlwaysAllow(a)){
                     { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":busy[a.id] !== undefined || allowLoading === a.id,"title":`Approve this call and stop asking for ${a.tool} — opens the auto-approve rule form`,...__sveltets_2_empty({"data-testid":`mcp-approve-always`}),"onclick":() => void openAlwaysAllow(a),});
                      allowLoading === a.id ? '…' : 'Approve & always allow…';
                     }
                  }
                };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {extra} = $$_snoitcAlavorppA5.$$prop_def;/*Ωignore_endΩ*/
                
               ApprovalActions}
             }
          } else if (a.status === 'approved' || a.status === 'denied' || a.status === 'consumed'){
            
             { svelteHTML.createElement("div", {   "class":`decided`,"title":a.decided_by ?? undefined,});
               { const $$_emoctuOlavorppA5C = __sveltets_2_ensureComponent(ApprovalOutcome); new $$_emoctuOlavorppA5C({ target: __sveltets_2_any(), props: {        "outcome":a.status === 'denied' ? 'denied' : 'approved',"by":a.decided_by ? who(a.decided_by) : null,"at":a.decided_at,"note":a.decision_note,}});}
             }
          } else if (a.status === 'expired'){
             { svelteHTML.createElement("div", { "class":`decided`,});
               { const $$_emoctuOlavorppA5C = __sveltets_2_ensureComponent(ApprovalOutcome); new $$_emoctuOlavorppA5C({ target: __sveltets_2_any(), props: {      "outcome":`expired`,"at":a.expires_at ?? a.decided_at,"note":a.decision_note,}});}
             }
          }else{
            
             { svelteHTML.createElement("div", { "class":`decided outcome-line`,});
               { svelteHTML.createElement("span", { "title":a.decided_by ?? undefined,});runStateLabel(a.status);a.decided_by ? ` by ${who(a.decided_by)}` : ''; }
              if(a.decided_at){ { svelteHTML.createElement("span", { "aria-hidden":`true`,});  } { const $$_emiTleR5C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR5C({ target: __sveltets_2_any(), props: {  "iso":a.decided_at,}});}}
              if(a.decision_note){ { svelteHTML.createElement("span", { "aria-hidden":`true`,});  } { svelteHTML.createElement("span", { "class":`note`,}); a.decision_note;  }}
             }
          }
         }
      }
     }
   LoadState}
 }

if(allowFor){
  const a = allowFor.approval;
  const sid = a.requested_by_session_id ?? null;
   { const $$_mroFevorppAotuA0C = __sveltets_2_ensureComponent(AutoApproveForm); new $$_mroFevorppAotuA0C({ target: __sveltets_2_any(), props: {               "catalog":allowFor.catalog,"preset":{ kind: 'tool', target: a.tool ?? '' },"presetScope":sid
      ? { scope: 'session', session_id: sid, session_label: sessionTitle(sid) ?? `Session ${sid.slice(0, 8)}…` }
      : a.workspace_id
        ? { scope: 'workspace', workspace_id: a.workspace_id }
        : null,"title":`Approve & always allow ${a.tool}`,"saveLabel":`Create rule & approve`,"onclose":() => (allowFor = null),"onsaved":() => decide(a, true),}});}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ApprovalsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ApprovalsTab__SvelteComponent_ = ReturnType<typeof ApprovalsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default ApprovalsTab__SvelteComponent_;