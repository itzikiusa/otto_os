///<reference types="svelte" />
;
import Terminal from '../../src/lib/components/Terminal.svelte';
import FileTree from '../../src/modules/panels/FileTree.svelte';
import { ws } from '../../src/lib/stores/workspace.svelte';
import { openFile } from '../../src/lib/stores/openfile.svelte';
import type { Session, WorkspaceWithRole } from '../../src/lib/api/types';
function $$render() {

  
  
  
  
  
  ws.currentId = 'w';
  ws.workspaces = [{ id: 'w', name: 'Fixture', root_path: '/work', role: 'admin' } as WorkspaceWithRole];
  ws.sessions = [{ id: 'links', workspace_id: 'w', kind: 'agent', provider: 'codex', cwd: '/work' } as Session];
  const wide = new URLSearchParams(location.search).has('wide');
  const share = new URLSearchParams(location.search).has('share');
  // desktop-terminal-park-redraw.spec.ts: a parking (keepAlive) terminal that
  // can be hidden and shown again like a tab switch, and the Redraw action.
  const keepAlive = new URLSearchParams(location.search).has('keepalive');
  let shown = $state(true);
  let termRef = $state<Terminal | null>(null);
;
async () => {
 { svelteHTML.createElement("div", { "class":`toolbar`,});
   { svelteHTML.createElement("button", { "onclick":() => openFile.open('/outside/Application Support/report.ts', 2, 3),});  }
   { svelteHTML.createElement("button", { "onclick":() => openFile.open('/slow/first.ts'),});  }
   { svelteHTML.createElement("button", { "onclick":() => openFile.open('/fast/latest.ts'),});  }
   { svelteHTML.createElement("button", { "onclick":() => { ws.currentId = null; ws.workspaces = []; },});  }
  if(keepAlive){
     { svelteHTML.createElement("button", { "onclick":() => (shown = !shown),});  }
     { svelteHTML.createElement("button", { "onclick":() => termRef?.redraw(),});  }
  }
 }
 { svelteHTML.createElement("div", { "class":`layout`,}); { svelteHTML.createElement("div", {  "class":`terminal`,});__sveltets_2_ensureType(String, Number, wide ? '1400px' : '650px');if(shown){ { const $$_lanimreT2C = __sveltets_2_ensureComponent(Terminal); const $$_lanimreT2 = new $$_lanimreT2C({ target: __sveltets_2_any(), props: {         "sessionId":`links`,"preferDom":true,"showToolbar":false,keepAlive,"shareToken":share ? 'guest-fixture' : undefined,}});termRef = $$_lanimreT2;}} } { svelteHTML.createElement("div", { "class":`files`,});  { const $$_eerTeliF2C = __sveltets_2_ensureComponent(FileTree); new $$_eerTeliF2C({ target: __sveltets_2_any(), props: {"primary":true,}});} } }
 { svelteHTML.createElement("output", { "aria-label":`Opened file`,});JSON.stringify(openFile.request); }
 { svelteHTML.createElement("output", { "aria-label":`Workspace state`,});ws.currentId; ws.sessions[0].cwd; }

};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TerminalLinksFixture__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TerminalLinksFixture__SvelteComponent_ = ReturnType<typeof TerminalLinksFixture__SvelteComponent_>;
/*Ωignore_endΩ*/export default TerminalLinksFixture__SvelteComponent_;