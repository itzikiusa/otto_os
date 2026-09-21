<script lang="ts">
  import Terminal from '../../src/lib/components/Terminal.svelte';
  import FileTree from '../../src/modules/panels/FileTree.svelte';
  import { ws } from '../../src/lib/stores/workspace.svelte';
  import { openFile } from '../../src/lib/stores/openfile.svelte';
  import type { Session, WorkspaceWithRole } from '../../src/lib/api/types';
  ws.currentId = 'w';
  ws.workspaces = [{ id: 'w', name: 'Fixture', root_path: '/work', role: 'admin' } as WorkspaceWithRole];
  ws.sessions = [{ id: 'links', workspace_id: 'w', kind: 'agent', provider: 'codex', cwd: '/work' } as Session];
  const wide = new URLSearchParams(location.search).has('wide');
  const share = new URLSearchParams(location.search).has('share');
</script>
<div class="toolbar">
  <button onclick={() => openFile.open('/outside/Application Support/report.ts', 2, 3)}>External file</button>
  <button onclick={() => openFile.open('/slow/first.ts')}>Slow file</button>
  <button onclick={() => openFile.open('/fast/latest.ts')}>Latest file</button>
  <button onclick={() => { ws.currentId = null; ws.workspaces = []; }}>No workspace</button>
</div>
<div class="layout"><div class="terminal" style:width={wide ? '1400px' : '650px'}><Terminal sessionId="links" preferDom showToolbar={false} shareToken={share ? 'guest-fixture' : undefined} /></div><div class="files"><FileTree primary /></div></div>
<output aria-label="Opened file">{JSON.stringify(openFile.request)}</output>
<output aria-label="Workspace state">{ws.currentId}:{ws.sessions[0].cwd}</output>
<style>.layout{display:flex;height:600px;gap:15px}.terminal{width:650px;min-width:0}.files{width:380px;min-width:0;height:100%}.toolbar{display:flex;gap:8px}output{display:block}</style>
