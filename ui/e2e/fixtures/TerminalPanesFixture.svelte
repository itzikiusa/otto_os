<script lang="ts">
  // desktop-terminal-compact-queue-perf.spec.ts (perf 01 N4): several agent
  // panes (preferDom) sharing ONE window — the window-wide compact queue and
  // the hidden-window ack hold only show with more than one Terminal mounted.
  // p1–p3 sit side by side and follow the viewport WIDTH (fixed height, so a
  // window resize never changes rows); p4 is below a tall spacer inside a
  // scroll box — off-screen for its IntersectionObserver until scrolled in.
  import Terminal from '../../src/lib/components/Terminal.svelte';
  import { ws } from '../../src/lib/stores/workspace.svelte';
  import type { Session, WorkspaceWithRole } from '../../src/lib/api/types';
  const ids = ['p1', 'p2', 'p3', 'p4'];
  ws.currentId = 'w';
  ws.workspaces = [{ id: 'w', name: 'Fixture', root_path: '/work', role: 'admin' } as WorkspaceWithRole];
  ws.sessions = ids.map((id) => ({ id, workspace_id: 'w', kind: 'agent', provider: 'claude', cwd: '/work' }) as Session);
</script>
<div class="row">
  {#each ids.slice(0, 3) as id (id)}
    <div class="pane" data-pane={id}><Terminal sessionId={id} preferDom showToolbar={false} /></div>
  {/each}
</div>
<div class="below" id="below-fold">
  <div class="spacer"></div>
  <div class="pane" data-pane="p4"><Terminal sessionId="p4" preferDom showToolbar={false} /></div>
</div>
<style>
  .row{display:flex;gap:8px;height:320px}
  .pane{flex:1 1 0;min-width:0;height:320px}
  .below{height:340px;overflow-y:auto;margin-top:8px}
  .spacer{height:3000px}
</style>
