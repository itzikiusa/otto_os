<script lang="ts">
  import {onMount, tick} from 'svelte';
  import DiffView from '../../src/lib/components/DiffView.svelte';
  let before = $state(''), after = $state('');
  onMount(() => {
    const host = window as unknown as {reviewDiff?: (a: string, b: string) => Promise<number>};
    host.reviewDiff = async (a, b) => {
      const started = performance.now();
      before = a; after = b;
      await tick();
      await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      return performance.now() - started;
    };
    return () => { delete host.reviewDiff; };
  });
</script>
<DiffView {before} {after} mode="split" contextLines={4} />
