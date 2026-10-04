<script lang="ts">
  // Mounts the personal-agent Autonomy form and Memory inspector against
  // routed fixture APIs (see desktop-personal-autonomy.spec.ts).
  import AgentAutonomy from '../../src/modules/personal-agents/AgentAutonomy.svelte';
  import AgentMemoryInspector from '../../src/modules/personal-agents/AgentMemoryInspector.svelte';
  import { router } from '../../src/lib/router.svelte';
  import ConfirmDialog from '../../src/lib/components/ConfirmDialog.svelte';
  const params = new URLSearchParams(location.search);
</script>
{#if params.get('view') === 'memory'}
  <AgentMemoryInspector agentId="a" editable={!params.has('viewer')} />
{:else}
  <AgentAutonomy agentId="a" editable={!params.has('viewer')} />
{/if}
<ConfirmDialog />

<button onclick={() => router.go("fixture-away")}>Leave autonomy</button>
