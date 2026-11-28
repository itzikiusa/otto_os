<script lang="ts">
  // The cluster workspace's Resources | Monitor switch (K-2): the page's
  // sections as the shared underline tabs, one row under the header
  // (design.md §7.14). Both views are one workspace: the same cluster stays
  // selected, the console's row cache and namespace are kept, and each view
  // resumes where it was left (the Monitor's state lives per cluster in the
  // k8s store).
  import Tabs from '../../lib/components/Tabs.svelte';
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import { monitorPath, resourcesPath } from '../../lib/k8s/viewState';

  interface Props {
    clusterId: string;
    view: 'resources' | 'monitor';
  }
  let { clusterId, view }: Props = $props();

  const VIEWS = [
    { id: 'resources' as const, label: 'Resources', testid: 'k8s-view-resources' },
    { id: 'monitor' as const, label: 'Monitor', title: 'Metrics, restarts and health history for this cluster', testid: 'k8s-view-monitor' },
  ];

  function go(to: 'resources' | 'monitor'): void {
    if (to === 'monitor') {
      router.go(monitorPath(clusterId));
      return;
    }
    // Back to the console exactly as it was: kind + selected row.
    const sel = k8s.clusterId === clusterId ? k8s.selected : null;
    router.go(resourcesPath(clusterId, k8s.kind, sel?.ns, sel?.name));
  }
</script>

<Tabs label="Cluster view" tabs={VIEWS} value={view} onchange={go} testid="k8s-view-switch" />
