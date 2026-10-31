///<reference types="svelte" />
;
import Terminal from '../../src/lib/components/Terminal.svelte';
import { ws } from '../../src/lib/stores/workspace.svelte';
import type { Session, WorkspaceWithRole } from '../../src/lib/api/types';
function $$render() {

  
  
  
  const query = new URLSearchParams(location.search);
  const ids = (query.get('ids') ?? '').split(',').filter(Boolean);
  const workspace = query.get('workspace') ?? '';
  ws.currentId = workspace;
  ws.workspaces = [{ id: workspace, name: 'Owned load fixtures', root_path: '/tmp', role: 'admin' } as WorkspaceWithRole];
  ws.sessions = ids.map((id) => ({ id, workspace_id: workspace, kind: 'agent', provider: 'codex', cwd: '/tmp', meta: {} } as Session));
;
async () => {
 { svelteHTML.createElement("main", {});
     for(let id of __sveltets_2_ensureArray(ids)){id;
     { svelteHTML.createElement("section", {}); { const $$_lanimreT2C = __sveltets_2_ensureComponent(Terminal); new $$_lanimreT2C({ target: __sveltets_2_any(), props: {    "sessionId":id,"showToolbar":false,}});} }
  }
 }

};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const TerminalLoadFixture__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type TerminalLoadFixture__SvelteComponent_ = InstanceType<typeof TerminalLoadFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default TerminalLoadFixture__SvelteComponent_;