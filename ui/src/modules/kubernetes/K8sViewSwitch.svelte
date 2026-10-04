<script lang="ts">
  // The cluster workspace's Resources | Monitor switch (K-2). Both views are
  // one workspace: the same cluster stays selected, the console's row cache
  // and namespace are kept, and each view resumes where it was left (the
  // Monitor's state lives per cluster in the k8s store).
  import { onTabKey } from '../../lib/tabKeys';
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import { monitorPath, resourcesPath } from './viewState';

  interface Props {
    clusterId: string;
    view: 'resources' | 'monitor';
  }
  let { clusterId, view }: Props = $props();

  function go(to: 'resources' | 'monitor'): void {
    if (to === view) return;
    if (to === 'monitor') {
      router.go(monitorPath(clusterId));
      return;
    }
    // Back to the console exactly as it was: kind + selected row.
    const sel = k8s.clusterId === clusterId ? k8s.selected : null;
    router.go(resourcesPath(clusterId, k8s.kind, sel?.ns, sel?.name));
  }
</script>

<div class="segmented k8s-view-switch" role="tablist" aria-label="Cluster view" data-testid="k8s-view-switch">
  <button role="tab" class:active={view === 'resources'} aria-selected={view === 'resources'} tabindex={view === 'resources' ? 0 : -1} onkeydown={onTabKey} onclick={() => go('resources')} data-testid="k8s-view-resources">Resources</button>
  <button role="tab" class:active={view === 'monitor'} aria-selected={view === 'monitor'} tabindex={view === 'monitor' ? 0 : -1} onkeydown={onTabKey} onclick={() => go('monitor')} title="Metrics, restarts and health history for this cluster" data-testid="k8s-view-monitor">Monitor</button>
</div>

<style>
  .k8s-view-switch {
    flex-shrink: 0;
  }
</style>
