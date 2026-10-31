///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { toastError } from '../../lib/toastError';
import ResourceAccess from '../../lib/components/ResourceAccess.svelte';
// Clusters overview: one card per saved cluster (env pill, color dot, server
// version + capability chips), "Add cluster" wizard (Admin), card context menu
// (open / test / refresh capabilities / edit / delete).
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { k8s } from '../../lib/stores/k8s.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { k8sApi } from '../../lib/api/k8s';
import type { K8sCluster } from '../../lib/api/types';
import EmptyState from '../../lib/components/EmptyState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import ClusterWizard from './ClusterWizard.svelte';
import InstallPanel from './InstallPanel.svelte';
import Modal from '../../lib/components/Modal.svelte';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const isAdmin = $derived(auth.isRoot);
  let wizardOpen = $state(false);
  let editing: K8sCluster | null = $state(null);
  $effect(() =>
    registry.register('kubernetes-overview', [
      { id: 'k8s.add', title: 'Add a Kubernetes cluster…', group: 'Kubernetes', keywords: 'k8s kubeconfig context import eks', disabled: !isAdmin, run: () => { editing = null; wizardOpen = true; } },
    ]),
  );
  let k9sSheet = $state(false);
  let testing: Record<string, boolean> = $state({});

  function open(c: K8sCluster): void {
    router.go(`kubernetes/${encodeURIComponent(c.id)}`);
  }

  async function test(c: K8sCluster): Promise<void> {
    testing = { ...testing, [c.id]: true };
    try {
      const r = await k8sApi.testCluster(c.id);
      if (r.ok) toasts.success(`Reached ${c.name}`, `${r.server_version ?? ''} · ${r.latency_ms} ms`.trim());
      else toasts.error(`Couldn’t reach ${c.name}`, r.message);
      void k8s.loadCapabilities(c.id, true);
    } catch (e) {
      toastError(`Couldn’t test ${c.name}`, e);
    } finally {
      testing = { ...testing, [c.id]: false };
    }
  }

  async function remove(c: K8sCluster): Promise<void> {
    const ok = await confirmer.ask(
      `Remove cluster “${c.name}”? Otto only forgets the row${c.source === 'kubeconfig' ? ' (your kubeconfig file is untouched)' : ' and removes the kubeconfig Otto wrote for it'}; nothing in the cluster changes.`,
      { title: 'Remove cluster', confirmLabel: 'Remove' },
    );
    if (!ok) return;
    try {
      await k8s.deleteCluster(c.id);
      toasts.success('Cluster removed', c.name);
    } catch (e) {
      toastError('Couldn’t remove the cluster', e);
    }
  }

  let accessFor = $state<string | null>(null);
  $effect(() => { for (const resource of k8s.clusters) void resourceAccess.load('k8s_cluster', resource.id); });

  function menu(e: MouseEvent | KeyboardEvent, c: K8sCluster): void {
    ctxMenu.show(e, [
      ...(resourceAccess.can('k8s_cluster', c.id, 'manage_access', 'kubernetes', 'admin')
        ? [{ label: 'Manage access…', icon: 'key', action: () => { accessFor = c.id; } }] : []),
      { label: 'Open', icon: 'helm', action: () => open(c) },
      { label: 'Test connection', icon: 'zap', action: () => void test(c) },
      { label: 'Refresh capabilities', icon: 'refresh', action: () => void k8s.loadCapabilities(c.id, true) },
      ...(resourceAccess.can('k8s_cluster', c.id, 'configure', 'kubernetes', 'admin')
        ? [
            { separator: true },
            { label: 'Edit…', icon: 'edit', action: () => { editing = c; wizardOpen = true; } },
            { label: 'Remove cluster…', icon: 'trash', danger: true, action: () => void remove(c) },
          ]
        : []),
    ]);
  }

  function cardKey(e: KeyboardEvent, c: K8sCluster): void {
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      menu(e, c);
    }
  }

  const sourceLabel: Record<string, string> = { kubeconfig: 'kubeconfig', imported: 'pasted', eks: 'EKS' };
;
async () => {

 { svelteHTML.createElement("div", { "class":`k8s-overview`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":`Kubernetes`,"subtitle":k8s.status?.kubectl.version ? `kubectl ${k8s.status.kubectl.version}${k8s.status.k9s.installed && k8s.status.k9s.version ? ` · k9s ${k8s.status.k9s.version}` : ''}` : undefined,actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(!k8s.status?.k9s.installed && isAdmin){
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (k9sSheet = true),});  }
    }
    if(k8s.clusters.length > 0){
     { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => router.go('kubernetes/monitor'),"title":`Monitoring dashboard: pod metrics, restarts, health`,...__sveltets_2_empty({"data-testid":`k8s-monitor-btn`}),});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gauge`,"size":12,}});} 
     }
    }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void k8s.loadClusters(),"title":`Refresh`,"aria-label":`Refresh clusters`,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
     }
    if(isAdmin && k8s.clusters.length > 0){
       { svelteHTML.createElement("button", {     "class":`btn small primary`,...__sveltets_2_empty({"data-testid":`k8s-add-cluster`}),"onclick":() => { editing = null; wizardOpen = true; },});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});

  if(k8s.clustersError && !k8s.clusters.length){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`clusters`,"variant":`page`,"error":k8s.clustersError,"empty":true,"onretry":() => void k8s.loadClusters(),}});}
  } else if (!k8s.clustersLoaded){
     { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":3,"height":96,}});}
  } else if (!k8s.clusters.length){
     { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {               "icon":`helm`,"title":`No clusters yet`,"body":isAdmin
        ? 'Add a cluster from a context in your kubeconfig, paste a kubeconfig, or import one from EKS in the AWS module.'
        : 'No clusters have been added. Ask an Otto admin to add one.',"actionLabel":isAdmin ? 'Add cluster' : undefined,"actionIcon":`plus`,"variant":`page`,"onaction":isAdmin ? () => { editing = null; wizardOpen = true; } : undefined,}});}
  }else{
     { svelteHTML.createElement("div", {   "class":`grid`,...__sveltets_2_empty({"data-testid":`k8s-cluster-grid`}),});
         for(let c of __sveltets_2_ensureArray(k8s.clusters)){c.id;
        const caps = k8s.capabilities[c.id];
        
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {             "role":`group`,"aria-label":c.name,"class":`card cluster`,...__sveltets_2_empty({"data-testid":`k8s-cluster-card`}),"oncontextmenu":(e) => { e.preventDefault(); menu(e, c); },});c.environment === 'prod';
           { svelteHTML.createElement("div", { "class":`row1`,});
             { svelteHTML.createElement("span", {   "class":`dot`,"style":`background:${c.color || 'var(--accent)'}`,}); }
             { svelteHTML.createElement("button", {       "class":`name open-link`,"title":`Open ${c.name}`,"onclick":() => open(c),"onkeydown":(e) => cardKey(e, c),});c.name; }
             { const $$_egdaBvnE5C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE5C({ target: __sveltets_2_any(), props: {  "env":c.environment,}});}
             { svelteHTML.createElement("button", {       "class":`icon-btn more`,"aria-label":`Actions for ${c.name}`,"title":`Cluster actions`,"onclick":(e) => menu(e, c),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
             }
           }
           { svelteHTML.createElement("div", {   "class":`row2 mono`,"title":c.context_name,});
             { svelteHTML.createElement("span", { "class":`dim`,});  } c.context_name;
            if(c.default_namespace){ { svelteHTML.createElement("span", { "class":`dim`,});   } c.default_namespace;}
           }
           { svelteHTML.createElement("div", { "class":`row3`,});
             { svelteHTML.createElement("span", { "class":`chip`,});sourceLabel[c.source] ?? c.source; }
            if(caps?.server_version){ { svelteHTML.createElement("span", { "class":`chip mono`,});caps.server_version; }}
            if(caps){
               { svelteHTML.createElement("span", {    "class":`chip`,"title":`metrics-server`,});caps.metrics_server;  }
              if(caps.argo_rollouts){ { svelteHTML.createElement("span", {   "class":`chip ok`,"title":`Argo Rollouts CRD present`,});  }}
              if(caps.argocd){ { svelteHTML.createElement("span", {   "class":`chip ok`,"title":`ArgoCD Application CRD present`,});  }}
            }else{
               { svelteHTML.createElement("span", {   "class":`chip dim`,"title":`Server version and add-ons appear once the cluster is reached — open it or choose Test connection`,});   }
            }
            if(testing[c.id]){ { svelteHTML.createElement("span", { "class":`chip accent`,});  }}
           }
         }}
      }
     }
  }
 PageBody}
 }

if(wizardOpen){
   { const $$_draziWretsulC0C = __sveltets_2_ensureComponent(ClusterWizard); new $$_draziWretsulC0C({ target: __sveltets_2_any(), props: {    "existing":editing,"onclose":() => { wizardOpen = false; editing = null; },}});}
}

if(k9sSheet){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Install k9s`,"width":520,"onclose":() => (k9sSheet = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (k9sSheet = false),});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
      { const $$_lenaPllatsnI1C = __sveltets_2_ensureComponent(InstallPanel); new $$_lenaPllatsnI1C({ target: __sveltets_2_any(), props: {  "tool":`k9s`,"compact":true,}});}
    
   Modal}
}

if(accessFor){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Manage access`,"width":780,"onclose":() => (accessFor = null),}});
     { const $$_sseccAecruoseR1C = __sveltets_2_ensureComponent(ResourceAccess); new $$_sseccAecruoseR1C({ target: __sveltets_2_any(), props: {    "kind":`k8s_cluster`,"resourceId":accessFor,}});}
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ClustersOverview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ClustersOverview__SvelteComponent_ = ReturnType<typeof ClustersOverview__SvelteComponent_>;
/*Ωignore_endΩ*/export default ClustersOverview__SvelteComponent_;