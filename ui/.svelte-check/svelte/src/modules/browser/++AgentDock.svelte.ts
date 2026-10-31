///<reference types="svelte" />
;// The Browser page's agent: the agent's live SHELL (the same Terminal as
// Agents, reused — exactly like Canvas's ConversationPanel, the DB
// Assistant, and the inline PR-review / workflow terminals) docked under
// the page, plus an ask bar that submits page + marks + question turns into
// it. readOnly is FALSE — the user types directly to the agent here (a real
// two-way session), and can also ask through the bar so the agent gets the
// page and marks as context. The bound session is an ordinary agent session
// (it also appears in Agents), remembered per workspace.

import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import StatusDot from '../../lib/components/StatusDot.svelte';
import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
import { sessionState } from '../../lib/status';
import { events } from '../../lib/events.svelte';
import AskBar from './AskBar.svelte';
import { browser } from '../../lib/stores/browser.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { paneResizer, RESIZE_TITLE_VERTICAL } from '../../lib/paneResizer';
import { startMouseDrag } from '../../lib/dragCursor';
import { toasts } from '../../lib/toast.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { viewport } from '../../lib/stores/viewport.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const sessionId = $derived(browser.agentSessionId);
  const session = $derived(sessionId ? (ws.getSession(sessionId) ?? null) : null);
  const status = $derived(sessionId ? (ws.statusMap[sessionId] ?? session?.status ?? null) : null);
  const readOnly = $derived(ws.myRole === 'viewer');
  const open = $derived(ui.browserAgentOpen);
  /** The one shared session state (lib/status.ts) — same dot and words as Agents. */
  const sState = $derived(
    session
      ? sessionState(session, status, ws.needsYou[session.id] === true, { stale: events.state !== 'connected' })
      : null,
  );

  // Drop a stale binding once the session list is loaded: the session was
  // archived/deleted, or isn't an agent session any more.
  $effect(() => {
    if (!sessionId || ws.sessionsLoading) return;
    const s = ws.getSession(sessionId);
    if (!s || s.archived || s.kind !== 'agent') browser.setAgentSession(null);
  });

  // Which agent to start (chosen BEFORE a session exists; same source and
  // defaulting as Canvas / the DB Assistant).
  const providers = $derived(agentProviders());
  let provider = $state('');
  $effect(() => {
    if (!provider && providers.length > 0) provider = defaultAgentProvider();
  });

  let creating = $state(false);
  async function createAgent(): Promise<void> {
    if (creating || readOnly) return;
    creating = true;
    try {
      const s = await ws.createSessionQuiet({
        kind: 'agent',
        provider: provider || defaultAgentProvider(),
        title: 'Browser agent',
        cwd: ws.current?.root_path ?? null,
        // `browser:true` also hands the CLI the browser MCP server (same as
        // the New Session dialog's checkbox) so it can drive pages itself.
        meta: { origin: 'browser', browser: true },
      });
      browser.setAgentSession(s.id);
      ui.setBrowserAgentOpen(true);
    } catch (e) {
      toastError('Couldn’t start agent', e);
    } finally {
      creating = false;
    }
  }

  // Attach an existing agent session (the shared clamped ctxMenu, so a long
  // list stays scrollable — same picker SendToSession uses).
  function pick(e: MouseEvent): void {
    const candidates = ws.agentSessions.filter((s) => s.id !== sessionId);
    if (candidates.length === 0) {
      toasts.warn('No other agent sessions', 'Start a new agent instead.');
      return;
    }
    const items: MenuItem[] = candidates.map((s) => ({
      label: `${s.title} (${s.provider})`,
      icon: 'terminal',
      action: () => {
        browser.setAgentSession(s.id);
        ui.setBrowserAgentOpen(true);
      },
    }));
    ctxMenu.show(e, items, { filter: items.length > 8, filterPlaceholder: 'Filter sessions…' });
  }

  // Drag the dock's top edge to resize it (persisted).
  let resizing = $state(false);
  function startResize(e: MouseEvent): void {
    resizing = true;
    const startY = e.clientY;
    const startH = ui.browserAgentH;
    // Overlay cursor + one write per frame (lib/dragCursor.ts).
    startMouseDrag(e, {
      cursor: 'row-resize',
      onMove: (ev) => ui.setBrowserAgentH(startH + (startY - ev.clientY)),
      onEnd: () => (resizing = false),
    });
  }
;
async () => {

  { svelteHTML.createElement("aside", {        "class":`assistant`,"style":open && sessionId ? `height:${ui.browserAgentH}px` : undefined,"aria-label":`Browser agent`,});open && !!sessionId;resizing;
  if(open && sessionId && !viewport.isPhone){
    
     {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ onDragStart: startResize, value: ui.browserAgentH, min: 160, max: 900, orientation: 'horizontal', invert: true, onChange: (h) => ui.setBrowserAgentH(h), onReset: () => ui.setBrowserAgentH(280), text: (v) => `${Math.round(v)} pixels tall` })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`resize-handle`,"role":`separator`,"aria-label":`Resize the browser agent`,"title":RESIZE_TITLE_VERTICAL,}); }}
  }

   { svelteHTML.createElement("header", { "class":`head`,});
    if(sessionId){
       { svelteHTML.createElement("button", {           "class":`icon-btn collapse`,"onclick":() => ui.setBrowserAgentOpen(!open),"title":open ? 'Collapse agent' : 'Expand agent',"aria-label":open ? 'Collapse agent' : 'Expand agent',"aria-expanded":open,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronUp',"size":12,}});}
       }
    }
     { svelteHTML.createElement("span", { "class":`title`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":14,}});}  }
    if(session){
      if(hasProviderIcon(session.provider)){ { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":session.provider ?? '',"size":13,}});}}
       { svelteHTML.createElement("span", {   "class":`name`,"title":session.title,});session.title; }
      if(sState){ { svelteHTML.createElement("span", { "class":`state`,}); { const $$_toDsutatS3C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS3C({ target: __sveltets_2_any(), props: {  "state":sState,}});} sState.label; }}
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":pick,"title":`Attach a different session`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`swap`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => browser.setAgentSession(null),"aria-label":`Detach session`,"title":`Detach — the session keeps running in Agents`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
       }
    }else{
       { svelteHTML.createElement("span", { "class":`dim no-session`,});   }
      if(!readOnly){
         { svelteHTML.createElement("span", { "class":`spacer`,}); }
        if(providers.length > 1){
           { svelteHTML.createElement("select", {         "class":`provider`,"bind:value":provider,"title":`Which agent to start`,"aria-label":`Agent to start`,"disabled":creating,});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let p of __sveltets_2_ensureArray(providers)){p;
               { svelteHTML.createElement("option", { "value":p,});p; }
            }
           }
        }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":pick,"disabled":creating,"title":`Attach an agent session that’s already running`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});} 
         }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void createAgent(),"disabled":creating,"title":`Start a new agent session for this page`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} creating ? 'Starting…' : 'New agent';
         }
      }
    }
   }

  if(open && sessionId){
    
     { svelteHTML.createElement("div", { "class":`agent-shell otto-force-dark`,});
      sessionId; {
          { const $$_lanimreTyzaL2C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL2C({ target: __sveltets_2_any(), props: {   sessionId,"readOnly":readOnly,"resumable":true,"forceDark":true,"preferDom":true,}});}
      }
     }
  }

   { const $$_raBksA1C = __sveltets_2_ensureComponent(AskBar); new $$_raBksA1C({ target: __sveltets_2_any(), props: {     sessionId,"terminalVisible":open && !!sessionId,"unboundHint":`Attach or start an agent to ask about this page.`,}});}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentDock__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentDock__SvelteComponent_ = ReturnType<typeof AgentDock__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentDock__SvelteComponent_;