<script lang="ts">
  import Terminal from '../../src/lib/components/Terminal.svelte';
  import { ws } from '../../src/lib/stores/workspace.svelte';
  import type { Session, WorkspaceWithRole } from '../../src/lib/api/types';
  const query = new URLSearchParams(location.search);
  const ids = (query.get('ids') ?? '').split(',').filter(Boolean);
  const workspace = query.get('workspace') ?? '';
  ws.currentId = workspace;
  ws.workspaces = [{ id: workspace, name: 'Owned load fixtures', root_path: '/tmp', role: 'admin' } as WorkspaceWithRole];
  ws.sessions = ids.map((id) => ({ id, workspace_id: workspace, kind: 'agent', provider: 'codex', cwd: '/tmp', meta: {} } as Session));
</script>
<main>
  {#each ids as id (id)}
    <section><Terminal sessionId={id} showToolbar={false} /></section>
  {/each}
</main>
<style>
  main { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4px; height: 96vh; }
  section { min-width: 0; min-height: 0; }
</style>
