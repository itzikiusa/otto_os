<script lang="ts">
  import AgentDocuments from '../../src/modules/personal-agents/AgentDocuments.svelte';
  import PathField from '../../src/lib/components/PathField.svelte';
  let path = $state('~/original');
  let submitted = $state(false);
  const params = new URLSearchParams(location.search);
</script>
<AgentDocuments agentId="a" workspaceId="w" kind={params.get('kind') === 'context' ? 'context' : 'memory'} editable={!params.has('viewer')} sharedWorkspace />
<form onsubmit={(e) => { e.preventDefault(); submitted = true; }}>
  <PathField bind:value={path}><input aria-label="Working directory" bind:value={path} /></PathField>
  <output>{submitted ? 'submitted' : 'not submitted'}</output>
</form>
