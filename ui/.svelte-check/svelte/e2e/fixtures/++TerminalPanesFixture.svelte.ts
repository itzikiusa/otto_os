///<reference types="svelte" />
;// desktop-terminal-compact-queue-perf.spec.ts (perf 01 N4): several agent
// panes (preferDom) sharing ONE window — the window-wide compact queue and
// the hidden-window ack hold only show with more than one Terminal mounted.
// p1–p3 sit side by side and follow the viewport WIDTH (fixed height, so a
// window resize never changes rows); p4 is below a tall spacer inside a
// scroll box — off-screen for its IntersectionObserver until scrolled in.

import Terminal from '../../src/lib/components/Terminal.svelte';
import { ws } from '../../src/lib/stores/workspace.svelte';
import type { Session, WorkspaceWithRole } from '../../src/lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  const ids = ['p1', 'p2', 'p3', 'p4'];
  ws.currentId = 'w';
  ws.workspaces = [{ id: 'w', name: 'Fixture', root_path: '/work', role: 'admin' } as WorkspaceWithRole];
  ws.sessions = ids.map((id) => ({ id, workspace_id: 'w', kind: 'agent', provider: 'claude', cwd: '/work' }) as Session);
;
async () => {
 { svelteHTML.createElement("div", { "class":`row`,});
     for(let id of __sveltets_2_ensureArray((ids.slice(0, 3)))){id;
     { svelteHTML.createElement("div", {   "class":`pane`,...__sveltets_2_empty({"data-pane":id}),}); { const $$_lanimreT2C = __sveltets_2_ensureComponent(Terminal); new $$_lanimreT2C({ target: __sveltets_2_any(), props: {    "sessionId":id,"preferDom":true,"showToolbar":false,}});} }
  }
 }
 { svelteHTML.createElement("div", {   "class":`below`,"id":`below-fold`,});
   { svelteHTML.createElement("div", { "class":`spacer`,}); }
   { svelteHTML.createElement("div", {   "class":`pane`,...__sveltets_2_empty({"data-pane":`p4`}),}); { const $$_lanimreT2C = __sveltets_2_ensureComponent(Terminal); new $$_lanimreT2C({ target: __sveltets_2_any(), props: {    "sessionId":`p4`,"preferDom":true,"showToolbar":false,}});} }
 }

};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const TerminalPanesFixture__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type TerminalPanesFixture__SvelteComponent_ = InstanceType<typeof TerminalPanesFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default TerminalPanesFixture__SvelteComponent_;