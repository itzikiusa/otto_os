<script lang="ts">
  // Kubernetes console module. Routes: `#/kubernetes` (clusters overview),
  // `#/kubernetes/<clusterId>` (workspace, last kind), `#/kubernetes/<clusterId>/<kind>`
  // and `#/kubernetes/<clusterId>/<kind>/<ns>/<name>` (row selected → drawer;
  // `-` stands in for an empty namespace on cluster-scoped kinds), and
  // `#/kubernetes/<clusterId>/monitor[/<tab>]` — the same cluster workspace in
  // its Monitor view (one workspace, a Resources | Monitor switch; the old
  // `kubernetes/monitor/<id>` URL redirects). The URL is the source of truth
  // for cluster / kind / selected row; the store holds the namespace + filter
  // + row cache + per-cluster view state. Leaving for the overview or the
  // Monitor never deselects the cluster, so coming back paints at once.
  // A first-run InstallPanel replaces the module while kubectl is missing
  // (contract §5).
  import { untrack } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import InstallPanel from './InstallPanel.svelte';
  import ClustersOverview from './ClustersOverview.svelte';
  import ClusterWorkspace from './ClusterWorkspace.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { isKind } from './k8s-util';
  import { monitorPath, parseK8sRoute } from './viewState';

  // perf K8s R5: the Monitor views (and MonitorInsights → `marked`) load on
  // first use, so the console chunk stays small; LazyMount designs the
  // loading / failed-load (inline Retry) states.
  const MonitorOverviewLazy = lazyComponent(() => import('./monitor/MonitorOverview.svelte'));
  const MonitorClusterLazy = lazyComponent(() => import('./monitor/MonitorCluster.svelte'));
  const MonitorFleetLazy = lazyComponent(() => import('./monitor/MonitorFleet.svelte'));

  const route = $derived(parseK8sRoute(router.parts));
  // `#/kubernetes/monitor` (overview) and `#/kubernetes/monitor/fleet[/<tab>]`
  // (the cross-cluster ClickHouse dashboard — `fleet` is a reserved segment,
  // never a cluster id) are cluster-less Monitor pages.
  const isMonitor = $derived(route.view === 'monitor' || route.view === 'monitor-overview' || route.view === 'fleet');
  const isFleet = $derived(route.view === 'fleet');
  const fleetTab = $derived(route.view === 'fleet' ? route.tab : 'overview');
  const monitorClusterId = $derived(route.view === 'monitor' ? route.clusterId : null);
  const monitorTab = $derived(route.view === 'monitor' ? route.tab : 'workloads');
  const routeClusterId = $derived(route.view === 'resources' ? route.clusterId : null);
  const routeKind = $derived(route.view === 'resources' ? route.kind : '');
  const routeNs = $derived(route.view === 'resources' ? route.ns : undefined);
  const routeName = $derived(route.view === 'resources' ? route.name : undefined);

  // The old per-cluster Monitor URL canonicalises to the workspace form.
  $effect(() => {
    const r = route;
    if (r.view === 'monitor' && r.legacy) untrack(() => router.replace(monitorPath(r.clusterId, r.tab)));
  });

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
    const monitorId = monitorClusterId;
    const kind = routeKind;
    const ns = routeNs;
    const name = routeName;
    untrack(() => {
      // The Monitor view is the same cluster workspace: entering it selects
      // the cluster (a no-op when it already is) and keeps the console cache.
      if (monitorId) k8s.selectCluster(monitorId);
      // The overview / Monitor overview / Fleet keep the last cluster's state.
      if (!id) return;
      k8s.selectCluster(id);
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
    <PageHeader title="Kubernetes" />
    <EmptyState
      variant="page"
      icon="helm"
      title="Kubernetes console isn't available"
      body="This daemon doesn't serve /k8s/* yet. Update Otto (or restart the daemon after upgrading) and reopen this page."
    />
  {:else if !k8s.status && k8s.statusError}
    <PageHeader title="Kubernetes" />
    <EmptyState actionKind="secondary" variant="page" icon="warning" title="Couldn't reach the daemon" body={k8s.statusError} actionLabel="Retry" onaction={() => void k8s.loadStatus()} />
  {:else if !k8s.status}
    <PageHeader title="Kubernetes" />
    <div class="k8s-boot"><Skeleton rows={4} height={48} /></div>
  {:else if needsInstall}
    <PageHeader title="Kubernetes" />
    <div class="k8s-scroll"><InstallPanel tool="kubectl" oncontinue={() => (skipInstall = true)} /></div>
  {:else if isFleet}
    <LazyMount lazy={MonitorFleetLazy} what="the Fleet dashboard" props={{ tab: fleetTab }} />
  {:else if isMonitor && monitorClusterId}
    {#if monitorCluster}
      {#key monitorCluster.id}<LazyMount lazy={MonitorClusterLazy} what="the cluster Monitor" props={{ cluster: monitorCluster, tab: monitorTab }} />{/key}
    {:else if k8s.clustersLoaded}
      <PageHeader title="Monitor" crumbs={[{ label: 'Kubernetes', onclick: () => router.go('kubernetes') }]} />
      <EmptyState
        variant="page"
        icon="helm"
        title="Cluster not found"
        body="It may have been removed. Pick another cluster from the Monitor overview."
        actionLabel="Back to Monitor"
        onaction={() => router.go('kubernetes/monitor')}
      />
    {:else}
      <PageHeader title="Monitor" crumbs={[{ label: 'Kubernetes', onclick: () => router.go('kubernetes') }]} />
      <div class="k8s-boot"><Skeleton rows={6} height={40} /></div>
    {/if}
  {:else if isMonitor}
    <LazyMount lazy={MonitorOverviewLazy} what="the Monitor overview" />
  {:else if routeClusterId}
    {#if cluster}
      {#key `${cluster.id}/${k8s.accessRevision}`}<ClusterWorkspace {cluster} />{/key}
    {:else if k8s.clustersLoaded}
      <PageHeader title="Kubernetes" />
      <EmptyState
        variant="page"
        icon="helm"
        title="Cluster not found"
        body="It may have been removed. Pick another cluster from the overview."
        actionLabel="Back to clusters"
        onaction={() => router.go('kubernetes')}
      />
    {:else}
      <PageHeader title="Kubernetes" />
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
    padding: 18px 20px;
  }
  .k8s-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>
