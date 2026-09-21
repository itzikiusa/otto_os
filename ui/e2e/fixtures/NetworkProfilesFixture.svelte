<script lang="ts">
  import NetworkProfilePicker from '../../src/modules/connections/NetworkProfilePicker.svelte';
  import SessionNetworkStatus from '../../src/modules/connections/SessionNetworkStatus.svelte';
  let selected = $state('');
  let workspaceId = $state('w');
</script>
<NetworkProfilePicker {workspaceId} value={selected} onchange={(id) => selected = id} editable />
<output data-testid="selected">{selected}</output>
<button onclick={() => { workspaceId = 'other'; selected = ''; }}>Other workspace</button>
<SessionNetworkStatus sessionId="s" workspaceId="w" selectedProfileId="p" editable={false} />
