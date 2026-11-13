<script lang="ts">
  // desktop-terminal-path-links.spec.ts: an agent session started in /work/a
  // of a workspace (/work) whose second repo is /work/b — printed relative
  // paths must resolve to the folder that really has them.
  import Terminal from '../../src/lib/components/Terminal.svelte';
  import FileTree from '../../src/modules/panels/FileTree.svelte';
  import Toasts from '../../src/lib/components/Toasts.svelte';
  import ContextMenu from '../../src/lib/components/ContextMenu.svelte';
  import { ws } from '../../src/lib/stores/workspace.svelte';
  import { openFile } from '../../src/lib/stores/openfile.svelte';
  import type { Session, WorkspaceWithRole } from '../../src/lib/api/types';
  ws.currentId = 'w';
  ws.workspaces = [{ id: 'w', name: 'Fixture', root_path: '/work', role: 'admin' } as WorkspaceWithRole];
  ws.sessions = [{ id: 'links', workspace_id: 'w', kind: 'agent', provider: 'claude', cwd: '/work/a', meta: {} } as Session];
</script>
<div class="layout"><div class="terminal"><Terminal sessionId="links" preferDom showToolbar={false} /></div><div class="files"><FileTree primary /></div></div>
<output aria-label="Opened file">{JSON.stringify(openFile.request)}</output>
<Toasts />
<ContextMenu />
<style>.layout{display:flex;height:600px;gap:15px}.terminal{width:650px;min-width:0}.files{width:380px;min-width:0;height:100%}output{display:block}</style>
