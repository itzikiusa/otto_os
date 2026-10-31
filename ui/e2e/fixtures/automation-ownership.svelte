<script lang="ts">
  import { onMount } from 'svelte';
  import LoopDetail from '../../src/modules/loops/LoopDetail.svelte';
  import ConfirmDialog from '../../src/lib/components/ConfirmDialog.svelte';
  import ContextMenu from '../../src/lib/components/ContextMenu.svelte';
  let target = $state('A');
  onMount(() => {
    const change = () => { target = 'B'; };
    window.addEventListener('review-target-change', change);
    return () => window.removeEventListener('review-target-change', change);
  });
</script>
<LoopDetail id={target} onback={() => {}} />
<ConfirmDialog /><ContextMenu />
