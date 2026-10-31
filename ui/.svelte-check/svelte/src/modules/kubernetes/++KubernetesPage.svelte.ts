///<reference types="svelte" />
;// Kubernetes console module. Routes: `#/kubernetes` (clusters overview),
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
import LoadState from '../../lib/components/LoadState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import InstallPanel from './InstallPanel.svelte';
import ClustersOverview from './ClustersOverview.svelte';
import ClusterWorkspace from './ClusterWorkspace.svelte';
import LazyMount from '../../lib/components/LazyMount.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';
import { isKind } from './k8s-util';
import { monitorPath, parseK8sRoute } from '../../lib/k8s/viewState';
import { registry } from '../../lib/commands.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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

  // ⌘K: the module's verbs, plus one "Open cluster …" per known cluster.
  $effect(() =>
    registry.register('kubernetes', [
      { id: 'k8s.clusters', title: 'Show Kubernetes clusters', group: 'Kubernetes', keywords: 'k8s overview contexts', run: () => router.go('kubernetes') },
      { id: 'k8s.monitor', title: 'Open Kubernetes monitoring', group: 'Kubernetes', keywords: 'k8s metrics restarts health pods', run: () => router.go('kubernetes/monitor') },
      { id: 'k8s.fleet', title: 'Open the Kubernetes fleet dashboard', group: 'Kubernetes', keywords: 'k8s every cluster memory latency req/s', run: () => router.go('kubernetes/monitor/fleet') },
      { id: 'k8s.refresh', title: 'Refresh Kubernetes clusters', group: 'Kubernetes', keywords: 'k8s reload reachability', run: () => void k8s.loadClusters() },
      ...k8s.clusters.map((c) => ({
        id: `k8s.open.${c.id}`,
        title: `Open cluster ${c.name}`,
        group: 'Kubernetes',
        detail: c.environment,
        keywords: 'k8s cluster workloads pods',
        run: () => router.go(`kubernetes/${encodeURIComponent(c.id)}`),
      })),
    ]),
  );

  const needsInstall = $derived(!!k8s.status && !k8s.status.kubectl.installed && !skipInstall);
  const cluster = $derived(
    routeClusterId ? (k8s.clusters.find((c) => c.id === routeClusterId) ?? null) : null,
  );
  const monitorCluster = $derived(
    monitorClusterId ? (k8s.clusters.find((c) => c.id === monitorClusterId) ?? null) : null,
  );
;
async () => {

 { svelteHTML.createElement("div", {   "class":`k8s-page`,...__sveltets_2_empty({"data-testid":`k8s-page`}),});
  if(k8s.unavailable){
     { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {  "title":`Kubernetes`,}});}
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`helm`,"title":`Kubernetes console isn’t available`,"body":`This daemon doesn’t serve /k8s/* yet. Update Otto (or restart the daemon after upgrading) and reopen this page.`,}});}
  } else if (!k8s.status && k8s.statusError){
     { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {  "title":`Kubernetes`,}});}
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "variant":`page`,"what":`Kubernetes`,"error":k8s.statusError,"empty":true,"onretry":() => void k8s.loadStatus(),}});}
  } else if (!k8s.status){
     { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {  "title":`Kubernetes`,}});}
     { svelteHTML.createElement("div", { "class":`k8s-boot`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":4,"height":48,}});} }
  } else if (needsInstall){
     { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {  "title":`Kubernetes`,}});}
     { svelteHTML.createElement("div", { "class":`k8s-scroll`,}); { const $$_lenaPllatsnI2C = __sveltets_2_ensureComponent(InstallPanel); new $$_lenaPllatsnI2C({ target: __sveltets_2_any(), props: {    "tool":`kubectl`,"oncontinue":() => (skipInstall = true),}});} }
  } else if (isFleet){
     { const $$_tnuoMyzaL1C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL1C({ target: __sveltets_2_any(), props: {      "lazy":MonitorFleetLazy,"what":`the Fleet dashboard`,"props":{ tab: fleetTab },}});}
  } else if (isMonitor && monitorClusterId){
    if(monitorCluster){
      monitorCluster.id; { { const $$_tnuoMyzaL1C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL1C({ target: __sveltets_2_any(), props: {      "lazy":MonitorClusterLazy,"what":`the cluster Monitor`,"props":{ cluster: monitorCluster, tab: monitorTab },}});}}
    } else if (k8s.clustersLoaded){
       { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":`Monitor`,"crumbs":[{ label: 'Kubernetes', onclick: () => router.go('kubernetes') }],}});}
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`helm`,"title":`Cluster not found`,"body":`It may have been removed. Pick another cluster from the Monitor overview.`,"actionLabel":`Back to Monitor`,"onaction":() => router.go('kubernetes/monitor'),}});}
    }else{
       { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":`Monitor`,"crumbs":[{ label: 'Kubernetes', onclick: () => router.go('kubernetes') }],}});}
       { svelteHTML.createElement("div", { "class":`k8s-boot`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":6,"height":40,}});} }
    }
  } else if (isMonitor){
     { const $$_tnuoMyzaL1C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL1C({ target: __sveltets_2_any(), props: {    "lazy":MonitorOverviewLazy,"what":`the Monitor overview`,}});}
  } else if (routeClusterId){
    if(cluster){
      `${cluster.id}/${k8s.accessRevision}`; { { const $$_ecapskroWretsulC1C = __sveltets_2_ensureComponent(ClusterWorkspace); new $$_ecapskroWretsulC1C({ target: __sveltets_2_any(), props: { cluster,}});}}
    } else if (k8s.clustersLoaded){
       { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {  "title":`Kubernetes`,}});}
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`helm`,"title":`Cluster not found`,"body":`It may have been removed. Pick another cluster from the overview.`,"actionLabel":`Back to clusters`,"onaction":() => router.go('kubernetes'),}});}
    }else{
       { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {  "title":`Kubernetes`,}});}
       { svelteHTML.createElement("div", { "class":`k8s-boot`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":6,"height":40,}});} }
    }
  }else{
    k8s.accessRevision; { { const $$_weivrevOsretsulC1C = __sveltets_2_ensureComponent(ClustersOverview); new $$_weivrevOsretsulC1C({ target: __sveltets_2_any(), props: {}});}}
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const KubernetesPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type KubernetesPage__SvelteComponent_ = ReturnType<typeof KubernetesPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default KubernetesPage__SvelteComponent_;