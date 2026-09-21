<script lang="ts">
  import AccountPicker from '../../src/lib/components/AccountPicker.svelte';
  let selected = $state('');
</script>
<AccountPicker provider="codex" workspaceId="w" value={selected} onchange={(id) => selected = id} />
<output aria-label="Selected account">{selected || 'default'}</output>
