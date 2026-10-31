///<reference types="svelte" />
;
import { plural } from '../plural';
import { NO_WORKSPACE } from '../labels';
// Dedicated broadcast composer. Relays a literal message to the selected live
// agent sessions via POST /workspaces/{id}/broadcast — NO AI, no parsing, no
// fallback. Separate from the ⌘K orchestrator on purpose.
import { api } from '../api/client';
import { toastError } from '../../lib/toastError';
import type { BroadcastResp, Id, Session } from '../api/types';
import { ui } from '../stores/ui.svelte';
import { ws } from '../stores/workspace.svelte';
import { toasts } from '../toast.svelte';
import Modal from './Modal.svelte';
import StatusDot from './StatusDot.svelte';
import { events } from '../events.svelte';
import { sessionState } from '../status';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let text = $state('');
  let busy = $state(false);
  let textareaEl: HTMLTextAreaElement | null = $state(null);
  // Sessions the user explicitly unticked. Tracking the *negative* set means
  // freshly-spawned sessions are included by default without extra bookkeeping.
  let deselected = $state<Set<Id>>(new Set());

  // Live FOREGROUND agent sessions are the only valid broadcast targets
  // (connections, dead/suspended sessions, and background engine sessions —
  // workflow steps, review agents, PR drafts — can't/shouldn't receive one).
  const eligible = $derived<Session[]>(
    ws.plainAgentSessions.filter((s) => {
      const st = ws.statusMap[s.id] ?? s.status;
      return st === 'running' || st === 'working' || st === 'idle';
    }),
  );

  const selected = $derived(eligible.filter((s) => !deselected.has(s.id)));
  const allSelected = $derived(eligible.length > 0 && selected.length === eligible.length);

  function toggle(id: Id): void {
    const next = new Set(deselected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    deselected = next;
  }

  function toggleAll(): void {
    deselected = allSelected ? new Set(eligible.map((s) => s.id)) : new Set();
  }

  function close(): void {
    ui.broadcastOpen = false;
    text = '';
    deselected = new Set();
  }

  async function send(): Promise<void> {
    if (busy) return;
    const msg = text.trim();
    if (msg === '') {
      toasts.warn('Nothing to broadcast', 'Type a message first.');
      return;
    }
    if (!ws.currentId) {
      toasts.error(NO_WORKSPACE, 'Broadcast sends to the sessions of one workspace.');
      return;
    }
    const ids = selected.map((s) => s.id);
    if (ids.length === 0) {
      toasts.warn('No sessions selected', 'Pick at least one session to broadcast to.');
      return;
    }
    busy = true;
    try {
      const resp = await api.post<BroadcastResp>(`/workspaces/${ws.currentId}/broadcast`, {
        text: msg,
        session_ids: ids,
      });
      const n = resp.session_ids.length;
      if (n === 0) toasts.warn('Broadcast sent to nobody', 'No selected session was live.');
      else toasts.success('Broadcast sent', `Delivered to ${plural(n, 'session')}`);
      close();
    } catch (e) {
      toastError('Couldn’t send the broadcast', e);
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent): void {
    // Enter sends; Shift+Enter inserts a newline; ⌘/Ctrl+Enter also sends.
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  $effect(() => {
    if (ui.broadcastOpen) queueMicrotask(() => textareaEl?.focus());
  });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Broadcast message`,"width":520,"onclose":close,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":close,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || selected.length === 0 || text.trim() === '',"onclick":send,});
      busy ? 'Sending…' : `Broadcast to ${selected.length}`;
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`bc`,});
     { const $$_textarea2 = svelteHTML.createElement("textarea", {               "dir":`auto`,"aria-label":`Broadcast message`,"bind:value":text,"placeholder":`Message to send to the selected sessions…  (Enter to send, ⇧Enter for newline)`,"rows":3,"spellcheck":`false`,"onkeydown":onKey,});textareaEl = $$_textarea2;/*Ωignore_startΩ*/() => text = __sveltets_2_any(null);/*Ωignore_endΩ*/ }

     { svelteHTML.createElement("div", { "class":`bc-list-head`,});
       { svelteHTML.createElement("span", { "class":`bc-label`,});  }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(eligible.length > 0){
         { svelteHTML.createElement("button", {   "class":`bc-link`,"onclick":toggleAll,});
          allSelected ? 'Select none' : 'Select all';
         }
      }
     }

     { svelteHTML.createElement("div", { "class":`bc-list`,});
         for(let s of __sveltets_2_ensureArray(eligible)){s.id;
         { svelteHTML.createElement("label", {  "class":`bc-row`,});!deselected.has(s.id);
           { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":!deselected.has(s.id),"onchange":() => toggle(s.id),});}
           { const $$_toDsutatS4C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS4C({ target: __sveltets_2_any(), props: {     "state":sessionState(s, ws.statusMap[s.id], ws.needsYou[s.id] === true, { stale: events.state !== 'connected' }),"size":7,}});}
           { svelteHTML.createElement("span", { "class":`bc-title`,});s.title; }
           { svelteHTML.createElement("span", { "class":`bc-provider`,});s.provider; }
         }
      }
         { svelteHTML.createElement("div", { "class":`bc-empty`,});       }
      
     }
   }

  
 Modal}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BroadcastModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BroadcastModal__SvelteComponent_ = ReturnType<typeof BroadcastModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default BroadcastModal__SvelteComponent_;