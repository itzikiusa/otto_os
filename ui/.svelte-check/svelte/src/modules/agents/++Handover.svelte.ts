///<reference types="svelte" />
;// Hand the current agent's working context to another agent in the same
// workspace — a freshly spawned CLI or an existing running one. Otto gathers
// the source agent's recent work (+ git state), summarizes it, and types a
// handover brief into the target. Optionally review/edit the brief first.

import Modal from '../../lib/components/Modal.svelte';
import Badge from '../../lib/components/Badge.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { toastError } from '../../lib/toastError';
import Icon from '../../lib/components/Icon.svelte';
import StatusDot from '../../lib/components/StatusDot.svelte';
import { events } from '../../lib/events.svelte';
import { sessionState } from '../../lib/status';
import { ws, isForeground } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { api } from '../../lib/api/client';
import type {
    Session,
    HandoverReq,
    HandoverTarget,
    HandoverBriefResp,
  } from '../../lib/api/types';
import { agentProviders, defaultAgentProvider, providerReadiness } from '../../lib/providers';
;

  interface Props {
    sessionId: string;
    onclose: () => void;
  };;

  type Mode = 'new' | 'existing';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { sessionId, onclose }: Props = $props();

  const source = $derived(ws.getSession(sessionId) ?? null);
  // A handover target must be able to reason — shell is not a valid target, so
  // the list is registry-sourced AGENT providers only (never plain shell).
  const providers = $derived(agentProviders());
  // Other agent sessions in the SOURCE's workspace, eligible as existing
  // targets (the daemon rejects cross-workspace handovers). Drawn from
  // `ws.sessions`, which holds the current workspace's AND the scratch
  // workspace's sessions — so a workspace-less source lists only scratch peers.
  const otherAgents = $derived(
    ws.sessions.filter(
      (s) =>
        s.id !== sessionId &&
        !s.archived &&
        s.kind === 'agent' &&
        s.provider !== 'shell' &&
        isForeground(s) &&
        s.workspace_id === source?.workspace_id,
    ),
  );

  let mode = $state<Mode>('new');
  let provider = $state('');
  let existingId = $state('');
  let focus = $state('');

  // Options.
  let includeGit = $state(true);
  let fast = $state(false);
  let reviewBrief = $state(false);
  let archiveSource = $state(false);

  // Review flow.
  let phase = $state<'compose' | 'review'>('compose');
  let brief = $state('');
  let briefNote = $state('');
  let briefLoading = $state(false);
  let busy = $state(false);

  async function requestClose(): Promise<void> {
    if (busy || briefLoading) return;
    if (focus.trim() !== '' || brief.trim() !== '') {
      const ok = await confirmer.ask('Discard your handover note and close?', {
        title: 'Discard handover?',
        confirmLabel: 'Discard',
        danger: true,
      });
      if (!ok) return;
    }
    onclose();
  }

  function toolFound(p: string): boolean {
    return providerReadiness(p).available;
  }

  function providerDesc(p: string): string {
    return p === 'claude'
      ? 'Claude Code CLI'
      : p === 'codex'
        ? 'Codex CLI'
        : p === 'shell'
          ? 'Plain shell'
          : 'Custom provider';
  }

  // Default to the configured default agent; only if it equals the source (or
  // isn't available) fall back to an available provider that differs from it.
  $effect(() => {
    if (provider === '' && providers.length > 0) {
      const avail = providers.filter(toolFound);
      const def = defaultAgentProvider();
      provider =
        def !== source?.provider && avail.includes(def)
          ? def
          : (avail.find((p) => p !== source?.provider) ?? avail[0] ?? providers[0]);
    }
  });

  const canSubmit = $derived(
    mode === 'new' ? provider !== '' && toolFound(provider) : existingId !== '',
  );

  function buildTarget(): HandoverTarget {
    return mode === 'new'
      ? { kind: 'new_agent', provider }
      : { kind: 'existing_session', session_id: existingId };
  }

  async function generateBrief(): Promise<void> {
    if (briefLoading || !canSubmit) return;
    briefLoading = true;
    briefNote = '';
    try {
      const resp = await api.post<HandoverBriefResp>(`/sessions/${sessionId}/handover/brief`, {
        focus: focus.trim() === '' ? null : focus.trim(),
        include_git: includeGit,
        fast,
      });
      brief = resp.brief;
      briefNote = !resp.had_context
        ? 'No prior context was found — the agent will receive only your focus note.'
        : resp.fallback
          ? 'Automatic summary was unavailable; showing raw recent context. Edit as needed.'
          : '';
      phase = 'review';
    } catch (e) {
      toastError('Couldn’t generate brief', e);
    } finally {
      briefLoading = false;
    }
  }

  async function send(briefText: string | null): Promise<void> {
    if (busy || !canSubmit) return;
    busy = true;
    try {
      const req: HandoverReq = {
        target: buildTarget(),
        focus: focus.trim() === '' ? null : focus.trim(),
        brief: briefText && briefText.trim() !== '' ? briefText.trim() : null,
        include_git: includeGit,
        fast,
        archive_source: archiveSource,
      };
      const session = await api.post<Session>(`/sessions/${sessionId}/handover`, req);
      ws.addSession(session); // adds (if new) + navigates to the target session
      onclose();
      const where = mode === 'new' ? `new ${provider} agent` : (session.title ?? 'the agent');
      toasts.success(
        'Handover started',
        briefText
          ? `Delivering your brief to ${where}.`
          : `Preparing the brief for ${where} — it’ll arrive shortly.`,
      );
    } catch (e) {
      toastError('Couldn’t hand over the session', e);
    } finally {
      busy = false;
    }
  }

  async function primary(): Promise<void> {
    const sending = phase === 'review' || !reviewBrief;
    if (sending && archiveSource) {
      const title = source?.title ?? 'this session';
      const ok = await confirmer.ask(
        `Hand over and archive “${title}”? The source session closes after the brief is delivered.`,
        { title: 'Hand over and archive', confirmLabel: 'Hand over and archive', danger: true },
      );
      if (!ok) return;
    }
    if (phase === 'review') {
      void send(brief);
    } else if (reviewBrief) {
      void generateBrief();
    } else {
      void send(null);
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Hand over to another agent`,"width":600,"onclose":() => void requestClose(),footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(phase === 'review'){
       { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => (phase = 'compose'),"disabled":busy,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});}  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || !canSubmit,"onclick":primary,});
        busy ? 'Sending…' : archiveSource ? 'Send handover and archive' : 'Send handover';
       }
    }else{
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => void requestClose(),});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || briefLoading || !canSubmit,"onclick":primary,});
        if(briefLoading){ } else if (busy){ } else if (reviewBrief){  } else if (archiveSource){   }else{ }
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  if(phase === 'compose'){
     { svelteHTML.createElement("p", { "class":`lead`,});
        { svelteHTML.createElement("strong", {});source?.title ?? 'this agent'; }     
         { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":source?.provider ?? 'the agent',}});}   
               
     }

    
     { svelteHTML.createElement("div", {     "class":`seg`,"role":`group`,"aria-label":`Hand over to`,});
       { svelteHTML.createElement("button", {      "class":`seg-btn`,"aria-pressed":mode === 'new',"onclick":() => (mode = 'new'),});mode === 'new';
         
       }
       { svelteHTML.createElement("button", {            "class":`seg-btn`,"aria-pressed":mode === 'existing',"disabled":otherAgents.length === 0,"title":otherAgents.length === 0 ? 'No other agents in this workspace' : '',"onclick":() => (mode = 'existing'),});mode === 'existing';
          otherAgents.length > 0 ? `(${otherAgents.length})` : '';
       }
     }

    if(mode === 'new'){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("div", { "class":`provider-grid`,});
             for(let p of __sveltets_2_ensureArray(providers)){p;
            const avail = toolFound(p);
             { svelteHTML.createElement("button", {             "class":`provider-card`,"aria-pressed":provider === p,"disabled":!avail,"title":providerReadiness(p).message,"onclick":() => (provider = p),});provider === p;!avail;
               { svelteHTML.createElement("span", { "class":`provider-name`,});
                 { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":p,"size":14,}});}
                p;
                if(p === source?.provider){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {  "label":`Same`,}});}}
                if(!avail){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {  "label":`Not found`,}});}}
               }
               { svelteHTML.createElement("span", { "class":`provider-desc`,});providerDesc(p); }
             }
          }
         }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("ul", { "class":`agent-list`,});
             for(let a of __sveltets_2_ensureArray(otherAgents)){a.id;
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("button", {        "class":`agent-row`,"aria-pressed":existingId === a.id,"onclick":() => (existingId = a.id),});existingId === a.id;
                 { const $$_toDsutatS5C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS5C({ target: __sveltets_2_any(), props: {  "state":sessionState(a, ws.statusMap[a.id], ws.needsYou[a.id] === true, { stale: events.state !== 'connected' }),}});}
                 { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":a.provider,"size":14,}});}
                 { svelteHTML.createElement("span", { "class":`agent-title`,});a.title; }
                 { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":a.provider,}});}
               }
             }
          }
         }
       }
    }

     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ho-focus`,});       { svelteHTML.createElement("span", { "class":`dim`,});  } }
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`ho-focus`,"class":`input`,"bind:value":focus,"rows":3,"placeholder":`e.g. The auth refactor is done — focus on wiring the new endpoint into the UI; don’t touch the DB layer.`,});/*Ωignore_startΩ*/() => focus = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }

     { svelteHTML.createElement("div", { "class":`opts`,});
       { svelteHTML.createElement("label", { "class":`toggle-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":includeGit,});/*Ωignore_startΩ*/() => includeGit = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`toggle-text`,});
           { svelteHTML.createElement("span", { "class":`toggle-title`,});   }
           { svelteHTML.createElement("span", { "class":`hint`,});           }
         }
       }
       { svelteHTML.createElement("label", { "class":`toggle-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":reviewBrief,});/*Ωignore_startΩ*/() => reviewBrief = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`toggle-text`,});
           { svelteHTML.createElement("span", { "class":`toggle-title`,});    }
           { svelteHTML.createElement("span", { "class":`hint`,});           }
         }
       }
       { svelteHTML.createElement("label", { "class":`toggle-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fast,});/*Ωignore_startΩ*/() => fast = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`toggle-text`,});
           { svelteHTML.createElement("span", { "class":`toggle-title`,});  }
           { svelteHTML.createElement("span", { "class":`hint`,});       }
         }
       }
       { svelteHTML.createElement("label", { "class":`toggle-row`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":archiveSource,});/*Ωignore_startΩ*/() => archiveSource = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`toggle-text`,});
           { svelteHTML.createElement("span", { "class":`toggle-title`,}); source?.provider ?? 'source';    }
           { svelteHTML.createElement("span", { "class":`hint`,});          }
         }
       }
     }
  }else{
    
     { svelteHTML.createElement("p", { "class":`lead`,});             }
    if(briefNote){ { svelteHTML.createElement("p", { "class":`note`,});briefNote; }}
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ho-brief`,});  }
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`ho-brief`,"class":`input mono brief`,"bind:value":brief,"rows":14,"placeholder":`The brief is empty — type what the agent should know, or just rely on your focus note.`,});/*Ωignore_startΩ*/() => brief = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }
  }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Handover__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Handover__SvelteComponent_ = ReturnType<typeof Handover__SvelteComponent_>;
/*Ωignore_endΩ*/export default Handover__SvelteComponent_;