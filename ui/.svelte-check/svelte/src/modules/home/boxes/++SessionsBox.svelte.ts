///<reference types="svelte" />
;// Agents box: the live session roster straight from the workspace store (no
// fetch of its own — `ws` is already kept current by the daemon WS), with
// working / needs-you / idle counts and a click-to-open row per session.

import Icon from '../../../lib/components/Icon.svelte';
import Badge from '../../../lib/components/Badge.svelte';
import StatusDot from '../../../lib/components/StatusDot.svelte';
import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { ui } from '../../../lib/stores/ui.svelte';
import { router } from '../../../lib/router.svelte';
import { events } from '../../../lib/events.svelte';
import { sessionState, type SessionStateInfo } from '../../../lib/status';
import type { Session } from '../../../lib/api/types';
import { rel } from '../../../lib/stores/now.svelte';
import type { HomeBox } from '../home.svelte';
;

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { box: _box, zoomed: _zoomed, tick: _tick }: Props = $props();

  // The one session state (lib/status.ts) — the same dot/label the sidebar,
  // tabs and tiles show, incl. suspended (resumable) and stale (socket down).
  const stateOf = (s: Session): SessionStateInfo =>
    sessionState(s, ws.statusMap[s.id], ws.needsYou[s.id] === true, { stale: events.state !== 'connected' });
  const sessions = $derived(
    [...ws.plainAgentSessions].sort((a, b) => {
      // Needs-you first (blocked on you), then working, then most recently active.
      const rank = (s: Session): number => {
        const k = stateOf(s).key;
        return k === 'needs-you' ? 0 : k === 'working' ? 1 : 2;
      };
      return rank(a) - rank(b) || Date.parse(b.last_active_at) - Date.parse(a.last_active_at);
    }),
  );
  const idle = $derived(sessions.filter((s) => (ws.statusMap[s.id] ?? s.status) === 'idle').length);
  const working = $derived(ws.workingCount);
  const needsYou = $derived(ws.needsYouCount);
  // Nothing listed yet: the first session load is still running (or has not
  // settled), or it failed. Neither may read as "No open agent sessions" with
  // 0 / 0 / 0 / 0 stats — the box shows the load state and "—" counts.
  const loadingList = $derived(ws.sessions.length === 0 && (ws.sessionsLoading || !ws.listSettled));
  const unknown = $derived(ws.sessions.length === 0 && (loadingList || !!ws.sessionsError));
  const count = (n: number): string => (unknown ? '—' : String(n));
;
async () => {

 { svelteHTML.createElement("div", { "class":`sessions`,});
   { svelteHTML.createElement("div", { "class":`stats`,});
     { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", {  "class":`n`,});!unknown && working > 0;count(working); } { svelteHTML.createElement("span", { "class":`l`,});  } }
     { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", {  "class":`n`,});!unknown && needsYou > 0;count(needsYou); } { svelteHTML.createElement("span", { "class":`l`,});  } }
     { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});count(idle); } { svelteHTML.createElement("span", { "class":`l`,});  } }
     { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`n`,});count(sessions.length); } { svelteHTML.createElement("span", { "class":`l`,});  } }
   }
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`sessions`,"variant":`compact`,"loading":loadingList,"error":ws.sessionsError,"empty":sessions.length === 0,"onretry":() => void ws.refreshSessions().catch(() => {}),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
      
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"icon":`terminal`,"title":`No open agent sessions`,"body":`Start Claude, Codex or a shell in this workspace.`,}});
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (ui.newSessionOpen = true),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
       EmptyState}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("ul", { "class":`rows`,});
         for(let s of __sveltets_2_ensureArray(sessions)){s.id;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {     "class":`srow`,"onclick":() => ws.navigateToSession(s.id),"title":`Open ${s.title}`,});
             { const $$_toDsutatS5C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS5C({ target: __sveltets_2_any(), props: {  "state":stateOf(s),}});}
             { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":s.provider,"size":12,}});}
             { svelteHTML.createElement("span", { "class":`title ellipsis`,});s.title; }
            if(ws.needsYou[s.id]){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Needs you`,}});}}
             { svelteHTML.createElement("span", {   "class":`ago`,"title":new Date(s.last_active_at).toLocaleString(),});rel(s.last_active_at, { compact: true }); }
             { svelteHTML.createElement("span", {   "class":`go`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});} }
           }
         }
      }
     }
    
     { svelteHTML.createElement("button", {   "class":`foot`,"onclick":() => router.go('agents'),});
        { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
     }
   LoadState}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SessionsBox__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SessionsBox__SvelteComponent_ = ReturnType<typeof SessionsBox__SvelteComponent_>;
/*Ωignore_endΩ*/export default SessionsBox__SvelteComponent_;