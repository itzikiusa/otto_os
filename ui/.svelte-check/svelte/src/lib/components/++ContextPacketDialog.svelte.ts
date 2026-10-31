///<reference types="svelte" />
;
import { plural } from '../plural';
// Shared "Send to agent" dialog — B2a.
//
// Workflow:
//   1. On mount (or when open becomes true) → POST /preview → show the
//      operator the redacted packet + a "Redacted: N secrets, M PII" badge.
//   2. If the operator clicks Send → POST /send → closes the dialog with a
//      toast confirmation.
//
// Props:
//   workspaceId  — workspace that owns both the source data and the target session.
//   sessionId    — the agent session to inject into (null = let user pick).
//   kind         — 'api' | 'db' | 'broker' (label + icon only; payload shape is free).
//   payload      — the raw JSON value to redact and inject.
//   onclose      — called when the dialog should be dismissed (cancel or after send).
import { untrack } from 'svelte';
import { api } from '../api/client';
import { ws } from '../stores/workspace.svelte';
import { toasts } from '../toast.svelte';
import { toastError } from '../toastError';
import Modal from './Modal.svelte';
import Badge from './Badge.svelte';
import LoadState from './LoadState.svelte';
import { loadErrorText } from '../loadError';
import type {
    ContextPacketKind,
    ContextPacketPreviewResp,
    RedactionHit,
    Session,
  } from '../api/types';
;

  interface Props {
    workspaceId: string;
    sessionId: string | null;
    kind: ContextPacketKind;
    payload: unknown;
    onclose: () => void;
  };;

  // Preview result (null = not yet loaded, 'loading' = in-flight).
  type PreviewState = 'loading' | ContextPacketPreviewResp | null;;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { workspaceId, sessionId, kind, payload, onclose }: Props = $props();

  // ── State ─────────────────────────────────────────────────────────────────
  // The session the operator wants to target. Defaults to the prop, falls back
  // to the currently-focused workspace session, or lets the user pick.
  let pickedSessionId = $state<string | null>(untrack(() => sessionId));

  let preview = $state<PreviewState>(null);
  let previewErr = $state<string | null>(null);

  let sending = $state(false);

  // Derive the running agent sessions so the operator can pick one.
  const agentSessions = $derived(
    ws.plainAgentSessions.filter((s: Session) => s.status !== 'exited'),
  );

  // ── Preview logic ─────────────────────────────────────────────────────────
  async function loadPreview(sid: string) {
    preview = 'loading';
    previewErr = null;
    try {
      const resp = await api.post<ContextPacketPreviewResp>(
        `/workspaces/${workspaceId}/agents/${sid}/context-packet/preview`,
        { kind, payload },
      );
      preview = resp;
    } catch (e) {
      previewErr = loadErrorText(e);
      preview = null;
    }
  }

  // Trigger a preview whenever the target session resolves.
  $effect(() => {
    const sid = pickedSessionId;
    if (sid) void loadPreview(sid);
    else { preview = null; previewErr = null; }
  });

  // ── Send ──────────────────────────────────────────────────────────────────
  async function sendPacket() {
    if (!pickedSessionId) return;
    sending = true;
    try {
      const resp = await api.post<{ ok: boolean; size_bytes: number; redactions: RedactionHit[] }>(
        `/workspaces/${workspaceId}/agents/${pickedSessionId}/context-packet/send`,
        { kind, payload },
      );
      const total = resp.redactions.reduce((s, h) => s + h.count, 0);
      const label = kindLabel(kind);
      toasts.success(
        total > 0
          ? `${label} sent — ${plural(total, 'secret')} redacted`
          : `${label} sent to agent`,
      );
      onclose();
    } catch (e) {
      toastError('Couldn’t send the context to the agent', e);
    } finally {
      sending = false;
    }
  }

  // ── Helpers ───────────────────────────────────────────────────────────────
  function kindLabel(k: ContextPacketKind): string {
    return k === 'api' ? 'API response' : k === 'db' ? 'DB result' : 'Broker message';
  }

  function redactionSummary(hits: RedactionHit[]): string {
    if (hits.length === 0) return 'No secrets detected';
    const parts = hits.map((h) => `${h.count} ${h.kind}`);
    return `Redacted: ${parts.join(', ')}`;
  }

  function previewJson(value: unknown): string {
    try { return JSON.stringify(value, null, 2); }
    catch { return String(value); }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    "title":`Send ${kindLabel(kind)} to agent`,"width":560,onclose,children:() => { async ()/*Ωignore_positionΩ*/ => {
    
    if(agentSessions.length > 0){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cp-session`,});   }
         { svelteHTML.createElement("select", {       "id":`cp-session`,"bind:value":pickedSessionId,"disabled":sending,});/*Ωignore_startΩ*/() => pickedSessionId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":null,});     }
             for(let s of __sveltets_2_ensureArray(agentSessions)){s.id;
             { svelteHTML.createElement("option", { "value":s.id,});s.title || s.id.slice(0, 8); s.status;  }
          }
         }
       }
    }else{
       { svelteHTML.createElement("p", { "class":`no-sessions`,});       }
    }

    
    if(pickedSessionId){
      if(preview === 'loading'){
         { svelteHTML.createElement("div", { "class":`preview-loading`,});  }
      } else if (previewErr){
         { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the preview`,"error":previewErr,"empty":true,"onretry":() => pickedSessionId && void loadPreview(pickedSessionId),}});}
      } else if (preview){
        
        const total = (preview as ContextPacketPreviewResp).redactions.reduce((s, h) => s + h.count, 0);
         { svelteHTML.createElement("div", { "class":`redactions`,});
           { const $$_egdaB1C = __sveltets_2_ensureComponent(Badge); new $$_egdaB1C({ target: __sveltets_2_any(), props: {    "tone":total > 0 ? 'warn' : 'ok',"label":redactionSummary((preview as ContextPacketPreviewResp).redactions),}});}
         }
        
         { svelteHTML.createElement("div", { "class":`preview-wrap`,});
           { svelteHTML.createElement("pre", { "class":`preview-code`,});previewJson((preview as ContextPacketPreviewResp).redacted); }
         }
         { svelteHTML.createElement("div", { "class":`size-hint`,});(preview as ContextPacketPreviewResp).size_bytes;     }
      }
    }
  };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn-ghost`,"onclick":onclose,"disabled":sending,});  }
     { svelteHTML.createElement("button", {       "class":`btn-primary`,"onclick":sendPacket,"disabled":sending || !pickedSessionId || preview === 'loading' || preview === null,});
      sending ? 'Sending…' : 'Send to agent';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ContextPacketDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ContextPacketDialog__SvelteComponent_ = ReturnType<typeof ContextPacketDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default ContextPacketDialog__SvelteComponent_;