<script lang="ts">
  import ContextSoul from '../../src/modules/settings/ContextSoul.svelte';
  import { ws } from '../../src/lib/stores/workspace.svelte';
  import { auth } from '../../src/lib/stores/auth.svelte';
  ws.workspaces = ['one', 'two'].map((id) => ({ id, name: `Workspace ${id}`, root_path: `/fixture/${id}`, settings: {}, archived: false, created_at: '2026-01-01T00:00:00Z', my_role: 'admin' as const }));
  ws.currentId = 'one';
  auth.me = { id: 'u', username: 'user', display_name: 'User', is_root: true, disabled: false, created_at: '2026-01-01T00:00:00Z' };
</script>
<button onclick={() => ws.currentId = 'one'}>Workspace one</button>
<button onclick={() => ws.currentId = 'two'}>Workspace two</button>
<ContextSoul />
