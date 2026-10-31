///<reference types="svelte" />
;// Agent UI control: "Claude · ‹session› is driving Connections" — the one
// strip that says an agent is operating THIS document (the side pane beside
// its session, usually). Like a browser's "controlled by automated software"
// infobar: it sits directly under the page's toolbar (the first PageHeader
// hosts it; the shell shows it above pages without one), stays while a
// command runs and for a minute after the last one, then goes by itself.
//
//   [mark] Claude · Refactor billing  is driving Connections  ● Run query…   Recent  ■ Stop  ✕
//
// Stop turns UI control off for that session (the daemon cancels anything
// pending; the agent's next call asks the user again). Recent lists this
// document's last 20 agent actions. Nothing here pulses under reduced motion.

import Icon from './Icon.svelte';
import ProviderIcon from './ProviderIcon.svelte';
import { uiControl, agentName } from '../stores/uiControl.svelte';
import { now, rel } from '../stores/now.svelte';
import { moduleLabel } from '../sidebar';
import { paneKey, routeOf } from '../sidePane';
import { router } from '../router.svelte';
import { providerName } from '../uiCommands/frames';
import { ctxMenu, type MenuItem } from '../contextmenu.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const d = $derived(uiControl.driving);
  const show = $derived(uiControl.visible(now()));
  const last = $derived(uiControl.recent.find((r) => d && r.agent.session_id === d.agent.session_id) ?? null);
  /** The module being driven; a shell command (state/open) names the page. */
  const module = $derived.by(() => {
    const m = d?.module && d.module !== 'shell' ? d.module : paneKey(routeOf(router.parts.join('/')));
    return moduleLabel(m);
  });
  const who = $derived(d ? providerName(d.agent.provider) : '');
  const title = $derived(d?.agent.title.trim() ?? '');
  let stopping = $state(false);

  async function stop(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await uiControl.stop();
    } finally {
      stopping = false;
    }
  }

  function openRecent(e: MouseEvent | KeyboardEvent): void {
    const many = new Set(uiControl.recent.map((r) => r.agent.session_id)).size > 1;
    const items: MenuItem[] = uiControl.recent.map((r) => ({
      label: r.note && r.ok === null ? `${r.label} — ${r.note}` : r.label,
      icon: r.ok === null ? 'clock' : r.ok ? 'check' : 'x',
      hint: many ? `${providerName(r.agent.provider)} · ${rel(r.at)}` : rel(r.at),
      title: r.error ? `${r.label} failed: ${r.error}` : `${agentName(r.agent)} · ${new Date(r.at).toLocaleTimeString()}`,
    }));
    if (items.length === 0) items.push({ label: 'No actions yet', disabled: true });
    ctxMenu.show(e, items);
  }
;
async () => {

if(show && d){
   { svelteHTML.createElement("div", {            "class":`adb`,"role":`region`,"aria-label":`Agent control`,...__sveltets_2_empty({"data-testid":`agent-driving-bar`}),...__sveltets_2_empty({"data-session":d.agent.session_id}),});d.awaitingHuman;
     { svelteHTML.createElement("span", {   "class":`adb-mark`,"aria-hidden":`true`,}); { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":d.agent.provider || 'agent',"size":14,}});} }
     { svelteHTML.createElement("span", {   "class":`adb-who`,"title":agentName(d.agent),});
       { svelteHTML.createElement("strong", {});who; }if(title){ { svelteHTML.createElement("span", { "class":`adb-title`,});` · ${title}`; }}
     }
     { svelteHTML.createElement("span", { "class":`adb-verb`,});  module; }
     { svelteHTML.createElement("span", {     "class":`adb-now`,"role":`status`,"aria-live":`polite`,});
      if(d.awaitingHuman){
         { svelteHTML.createElement("span", {   "class":`adb-dot warn`,"aria-hidden":`true`,}); }    } else if (d.running > 0){
         { svelteHTML.createElement("span", {   "class":`adb-dot live`,"aria-hidden":`true`,}); }d.current ?? 'Working'; } else if (last){
        
         { svelteHTML.createElement("span", { "class":`adb-last`,}); last.label;last.ok === false ? ' (failed)' : ''; { svelteHTML.createElement("span", { "aria-hidden":`true`,});  rel(last.at); } }
      }
     }
     { svelteHTML.createElement("span", { "class":`adb-grow`,}); }
     { svelteHTML.createElement("button", {             "class":`btn small ghost adb-btn`,"onclick":openRecent,"aria-haspopup":`menu`,"aria-label":`Recent agent actions`,"title":`This window’s recent agent actions`,...__sveltets_2_empty({"data-testid":`agent-driving-recent`}),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} { svelteHTML.createElement("span", { "class":`adb-btn-label`,});  } }
     { svelteHTML.createElement("button", {             "class":`btn small adb-btn`,"onclick":stop,"disabled":stopping,"title":`Stop — turn off UI control for “${title || who}”. It asks you again before driving Otto.`,"aria-label":`Stop ${who} driving Otto`,...__sveltets_2_empty({"data-testid":`agent-driving-stop`}),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} { svelteHTML.createElement("span", { "class":`adb-btn-label`,});  } }
    if(d.running === 0){
       { svelteHTML.createElement("button", {         "class":`icon-btn adb-hide`,"onclick":() => uiControl.dismiss(),"title":`Hide until the agent acts again`,"aria-label":`Hide until the agent acts again`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentDrivingBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentDrivingBar__SvelteComponent_ = ReturnType<typeof AgentDrivingBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentDrivingBar__SvelteComponent_;