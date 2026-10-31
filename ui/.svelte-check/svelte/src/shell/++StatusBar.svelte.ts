///<reference types="svelte" />
;// Status bar: working agents · needs you · event stream · current branch ·
// network listener · clock. While the events socket is down the live counts
// are stale: the working dot stops pulsing and the stream says
// "Reconnecting…" (patterns.md §1, "Stale is a state").

import Icon from '../lib/components/Icon.svelte';
import { ws } from '../lib/stores/workspace.svelte';
import { auth } from '../lib/stores/auth.svelte';
import { events } from '../lib/events.svelte';
import { git } from '../lib/stores/git.svelte';
import { router } from '../lib/router.svelte';
import { nowMinute } from '../lib/stores/now.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  const live = $derived(events.state === 'connected');

  /** "· N need you" → narrow the sidebar to the sessions waiting on the user. */
  function showNeedsYou(): void {
    ws.needsYouFilter = true;
    if (router.module !== 'agents') router.go('agents');
  }

  // The shared clock's minute tick (skips while the window is hidden) — no
  // interval of our own, and the label turns over on the minute.
  const clock = $derived(
    new Date(nowMinute()).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
  );
;
async () => {

 { svelteHTML.createElement("footer", { "class":`statusbar chrome-material`,});
   { svelteHTML.createElement("div", { "class":`sb-group`,});
     { svelteHTML.createElement("button", {       "class":`sb-item sb-btn`,"onclick":() => router.go('agents'),"title":live ? 'Agents working — open Agents' : 'Agents working (last known — reconnecting) — open Agents',});
       { svelteHTML.createElement("span", {     "class":`working-dot`,"aria-hidden":`true`,});ws.workingCount > 0;!live; }
      ws.workingCount; 
     }
    
     { svelteHTML.createElement("span", {     "class":`sr-only`,"role":`status`,"aria-live":`polite`,});
      ws.needsYouCount > 0 ? `${ws.needsYouCount} ${ws.needsYouCount === 1 ? 'session needs' : 'sessions need'} you` : '';
     }
    if(ws.needsYouCount > 0){
       { svelteHTML.createElement("button", {         "class":`sb-item sb-btn needs-you`,"onclick":showNeedsYou,"title":`Show only the sessions waiting on you`,...__sveltets_2_empty({"data-testid":`statusbar-needs-you`}),});
         ws.needsYouCount; ws.needsYouCount === 1 ? 'needs' : 'need'; 
       }
    }
    if(live){
       { svelteHTML.createElement("span", {   "class":`sb-item dim`,"title":`Event stream: live`,});
         { svelteHTML.createElement("span", {   "class":`conn-dot connected`,"aria-hidden":`true`,}); }
        
       }
    }else{
       { svelteHTML.createElement("button", {       "class":`sb-item sb-btn dim`,"onclick":() => events.reconnectNow(),"title":`Event stream: ${events.state} — live status may be out of date. Click to reconnect now`,});
         { svelteHTML.createElement("span", {   "class":`conn-dot ${events.state}`,"aria-hidden":`true`,}); }
        
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`sb-group`,});
    if(git.primaryStatus){
       { svelteHTML.createElement("button", {       "class":`sb-item sb-btn sb-branch`,"onclick":() => router.go('git'),"title":`Current branch: ${git.primaryStatus.branch} — open Git`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`sb-branch-name`,});git.primaryStatus.branch; }
        if(git.primaryStatus.ahead > 0){ { svelteHTML.createElement("span", { "class":`dim`,}); git.primaryStatus.ahead; }}
        if(git.primaryStatus.behind > 0){ { svelteHTML.createElement("span", { "class":`dim`,}); git.primaryStatus.behind; }}
       }
    }
     { svelteHTML.createElement("span", {   "class":`sb-item dim`,"title":`Network listener`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});}
      auth.meta?.network_listener ? 'network' : 'loopback';
     }
     { svelteHTML.createElement("span", { "class":`sb-item dim`,});clock; }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StatusBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StatusBar__SvelteComponent_ = ReturnType<typeof StatusBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default StatusBar__SvelteComponent_;