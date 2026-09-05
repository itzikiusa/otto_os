<script lang="ts">
  // Kubernetes console module. Routes: `#/kubernetes` (clusters overview),
  // `#/kubernetes/<clusterId>` (workspace, last kind), `#/kubernetes/<clusterId>/<kind>`
  // and `#/kubernetes/<clusterId>/<kind>/<ns>/<name>` (row selected → drawer;
  // `-` stands in for an empty namespace on cluster-scoped kinds). The URL is
  // the source of truth for cluster / kind / selected row; the store holds
  // the namespace + filter + cache. A first-run InstallPanel replaces the
  // module while kubectl is missing (contract §5).
  import { untrack } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import InstallPanel from './InstallPanel.svelte';
  import ClustersOverview from './ClustersOverview.svelte';
  import ClusterWorkspace from './ClusterWorkspace.svelte';
  import MonitorOverview from './monitor/MonitorOverview.svelte';
  import MonitorCluster from './monitor/MonitorCluster.svelte';
  import { isKind } from './k8s-util';

  // `#/kubernetes/monitor[/<clusterId>[/<tab>]]` is the Monitor dashboard; it
  // never selects a cluster in the console store.
  const isMonitor = $derived(router.parts[1] === 'monitor');
  const monitorClusterId = $derived(isMonitor ? (router.parts[2] ?? null) : null);
  const monitorTab = $derived(router.parts[3] ?? 'workloads');
  const routeClusterId = $derived(isMonitor ? null : (router.parts[1] ?? null));
  const routeKind = $derived(router.parts[2] ?? '');
  const routeNs = $derived(router.parts[3]);
  const routeName = $derived(router.parts[4]);

  /** Session-only "continue without installing" — lets a viewer who can't
   *  install (or someone with kubectl on a non-standard path) still reach the
   *  cluster list. */
  let skipInstall = $state(false);

  $effect(() => {
    void k8s.accessRevision;
    void k8s.loadStatus();
    void k8s.loadClusters();
    return () => k8s.suspend();
  });

  // Route → store. Only the route is a dependency; every store write is
  // untracked so a store change can never re-trigger this effect.
  $effect(() => {
    void k8s.accessRevision;
    const id = routeClusterId;
    const kind = routeKind;
    const ns = routeNs;
    const name = routeName;
    untrack(() => {
      k8s.selectCluster(id);
      if (!id) return;
      if (isKind(kind)) k8s.setKind(kind);
      if (name !== undefined && ns !== undefined) {
        const sel = { ns: ns === '-' ? '' : ns, name };
        if (k8s.selected?.ns !== sel.ns || k8s.selected?.name !== sel.name) k8s.select(sel);
      } else if (k8s.selected) {
        k8s.select(null);
      }
    });
  });

  const needsInstall = $derived(!!k8s.status && !k8s.status.kubectl.installed && !skipInstall);
  const cluster = $derived(
    routeClusterId ? (k8s.clusters.find((c) => c.id === routeClusterId) ?? null) : null,
  );
  const monitorCluster = $derived(
    monitorClusterId ? (k8s.clusters.find((c) => c.id === monitorClusterId) ?? null) : null,
  );
</script>

<div class="k8s-page" data-testid="k8s-page">
  {#if k8s.unavailable}
    <EmptyState
      icon="helm"
      title="Kubernetes console isn't available"
      body="This daemon doesn't serve /k8s/* yet. Update Otto (or restart the daemon after upgrading) and reopen this page."
    />
  {:else if !k8s.status && k8s.statusError}
    <EmptyState icon="helm" title="Couldn't reach the daemon" body={k8s.statusError} actionLabel="Retry" onaction={() => void k8s.loadStatus()} />
  {:else if !k8s.status}
    <div class="k8s-boot"><Skeleton rows={4} height={48} /></div>
  {:else if needsInstall}
    <InstallPanel tool="kubectl" oncontinue={() => (skipInstall = true)} />
  {:else if isMonitor && monitorClusterId}
    {#if monitorCluster}
      <MonitorCluster cluster={monitorCluster} tab={monitorTab} />
    {:else if k8s.clustersLoaded}
      <EmptyState
        icon="helm"
        title="Cluster not found"
        body="It may have been removed. Pick another cluster from the Monitor overview."
        actionLabel="Back to Monitor"
        onaction={() => router.go('kubernetes/monitor')}
      />
    {:else}
      <div class="k8s-boot"><Skeleton rows={6} height={40} /></div>
    {/if}
  {:else if isMonitor}
    <MonitorOverview />
  {:else if routeClusterId}
    {#if cluster}
      {#key `${cluster.id}/${k8s.accessRevision}`}<ClusterWorkspace {cluster} />{/key}
    {:else if k8s.clustersLoaded}
      <EmptyState
        icon="helm"
        title="Cluster not found"
        body="It may have been removed. Pick another cluster from the overview."
        actionLabel="Back to clusters"
        onaction={() => router.go('kubernetes')}
      />
    {:else}
      <div class="k8s-boot"><Skeleton rows={6} height={40} /></div>
    {/if}
  {:else}
    {#key k8s.accessRevision}<ClustersOverview />{/key}
  {/if}
</div>

<style>
  .k8s-page {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .k8s-boot {
    padding: 24px;
  }
</style>
