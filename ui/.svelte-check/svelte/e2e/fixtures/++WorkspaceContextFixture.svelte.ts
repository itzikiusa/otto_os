///<reference types="svelte" />
;
import ContextSoul from '../../src/modules/settings/ContextSoul.svelte';
import { ws } from '../../src/lib/stores/workspace.svelte';
import { auth } from '../../src/lib/stores/auth.svelte';
function $$render() {

  
  
  
  ws.workspaces = ['one', 'two'].map((id) => ({ id, name: `Workspace ${id}`, root_path: `/fixture/${id}`, settings: {}, archived: false, created_at: '2026-01-01T00:00:00Z', my_role: 'admin' as const }));
  ws.currentId = 'one';
  auth.me = { id: 'u', username: 'user', display_name: 'User', is_root: true, disabled: false, created_at: '2026-01-01T00:00:00Z' };
;
async () => {
 { svelteHTML.createElement("button", { "onclick":() => ws.currentId = 'one',});  }
 { svelteHTML.createElement("button", { "onclick":() => ws.currentId = 'two',});  }
 { const $$_luoStxetnoC0C = __sveltets_2_ensureComponent(ContextSoul); new $$_luoStxetnoC0C({ target: __sveltets_2_any(), props: {}});}
};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const WorkspaceContextFixture__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type WorkspaceContextFixture__SvelteComponent_ = InstanceType<typeof WorkspaceContextFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkspaceContextFixture__SvelteComponent_;