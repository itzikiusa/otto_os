<script lang="ts">
  // JsonTree moved to lib/components/JsonTree.svelte (architecture-1-11): it is
  // shared by the AWS, API-client and workbench views, not a database-only
  // component. This thin wrapper keeps the old import path working.
  import type { ComponentProps } from 'svelte';
  import JsonTree from '../../lib/components/JsonTree.svelte';

  let props: ComponentProps<typeof JsonTree> = $props();
</script>

<JsonTree {...props} />
