///<reference types="svelte" />
;
import Badge from '../../lib/components/Badge.svelte';
import { CLUSTER_VIEWS, type ClusterView } from './types';
import Tabs from '../../lib/components/Tabs.svelte';
import { toastError } from '../../lib/toastError';
// Embeddable Kafka cluster viewer: the header + per-cluster tab strip
// (Overview / Topics / Consumer Groups / Schema Registry / Replay / Lag Alerts)
// that the standalone Message Brokers page renders, factored out so the unified
// Connections workbench can open a cluster as a first-class tab (next to the DB
// connections) instead of navigating away. Self-contained: it owns its sub-tab
// + warm-tunnel state and does the Test call itself; Edit/Remove are delegated
// to the host via callbacks so each page can wire them to its own flow.
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import { api } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import type { BrokerCluster, TestClusterResp } from '../../lib/api/types';
import OverviewTab from './OverviewTab.svelte';
import TopicsTab from './TopicsTab.svelte';
import GroupsTab from './GroupsTab.svelte';
import SchemaTab from './SchemaTab.svelte';
import ReplayPanel from './ReplayPanel.svelte';
import LagAlertsPanel from './LagAlertsPanel.svelte';
;

  interface Props {
    cluster: BrokerCluster;
    onEdit: (c: BrokerCluster) => void;
    onRemove: (c: BrokerCluster) => void;
  };;

  type Tab = ClusterView;;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { cluster, onEdit, onRemove }: Props = $props();

  let tab = $state<Tab>('overview');
  let testing = $state(false);

  // Reset to Overview whenever the shown cluster changes (tab switch in the host).
  $effect(() => {
    void cluster.id;
    tab = 'overview';
  });

  // Warm the SSH tunnel (if any) in the background so the pill flips to "Tunnel"
  // quickly — mirrors the standalone Message Brokers page.
  let warmingId = $state<string | null>(null);
  let tunnelReady = $state(false);
  $effect(() => {
    const c = cluster;
    tunnelReady = false;
    if (!c.ssh) return;
    warmingId = c.id;
    const id = c.id;
    void api
      .post<TestClusterResp>(`/brokers/clusters/${id}/test`, {})
      .then((r) => {
        if (warmingId === id) tunnelReady = r.ok;
      })
      .catch(() => {
        // silent — the pill stays grey until a real op succeeds
      });
  });

  async function testConn(): Promise<void> {
    testing = true;
    try {
      const r = await api.post<TestClusterResp>(`/brokers/clusters/${cluster.id}/test`, {});
      if (r.ok) toasts.success('Connected', `${r.message} · ${r.latency_ms}ms`);
      else toasts.error('Couldn’t connect', r.message);
    } catch (e) {
      toastError('Couldn’t test the connection', e);
    } finally {
      testing = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`cv`,});
   { svelteHTML.createElement("header", { "class":`cv-head`,});
     { svelteHTML.createElement("div", { "class":`cv-title`,});
       { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${cluster.color || 'var(--accent)'}`,}); }
       { svelteHTML.createElement("span", {   "class":`name ellipsis`,"title":cluster.name,});cluster.name; }
       { const $$_egdaBvnE3C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE3C({ target: __sveltets_2_any(), props: {  "env":cluster.environment,}});}
      if(cluster.read_only){ { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":`Read-only`,}});}}
      if(cluster.ssh){
         { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {        "tone":tunnelReady ? 'ok' : 'neutral',"dot":true,"live":!tunnelReady,"title":tunnelReady ? 'SSH tunnel connected' : 'SSH tunnel warming…',"label":tunnelReady ? 'Tunnel' : 'Connecting…',}});}
      }
       { svelteHTML.createElement("span", {   "class":`boot mono ellipsis`,"title":cluster.bootstrap_servers,});cluster.bootstrap_servers; }
     }
     { svelteHTML.createElement("div", { "class":`cv-actions`,});
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":testConn,"disabled":testing,});
        testing ? 'Testing…' : 'Test';
       }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => onEdit(cluster),});  }
       { svelteHTML.createElement("button", {   "class":`btn small danger`,"onclick":() => onRemove(cluster),});  }
     }
   }

   { const $$_sbaT1C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT1C({ target: __sveltets_2_any(), props: {          "label":`Kafka cluster views`,"tabs":CLUSTER_VIEWS,"value":tab,"onchange":(id) => (tab = id),"idBase":`kafka-cv`,}});}

   { svelteHTML.createElement("div", {       "class":`cv-body`,"role":`tabpanel`,"id":`kafka-cv-panel-${tab}`,"aria-labelledby":`kafka-cv-tab-${tab}`,});
    cluster.id; {
      if(tab === 'overview'){
         { const $$_baTweivrevO2C = __sveltets_2_ensureComponent(OverviewTab); new $$_baTweivrevO2C({ target: __sveltets_2_any(), props: {  "clusterId":cluster.id,}});}
      } else if (tab === 'topics'){
         { const $$_baTscipoT2C = __sveltets_2_ensureComponent(TopicsTab); new $$_baTscipoT2C({ target: __sveltets_2_any(), props: { cluster,}});}
      } else if (tab === 'groups'){
         { const $$_baTspuorG2C = __sveltets_2_ensureComponent(GroupsTab); new $$_baTspuorG2C({ target: __sveltets_2_any(), props: { cluster,}});}
      } else if (tab === 'schema'){
         { const $$_baTamehcS2C = __sveltets_2_ensureComponent(SchemaTab); new $$_baTamehcS2C({ target: __sveltets_2_any(), props: { cluster,}});}
      } else if (tab === 'replay'){
         { const $$_lenaPyalpeR2C = __sveltets_2_ensureComponent(ReplayPanel); new $$_lenaPyalpeR2C({ target: __sveltets_2_any(), props: { cluster,}});}
      } else if (tab === 'alerts'){
         { const $$_lenaPstrelAgaL2C = __sveltets_2_ensureComponent(LagAlertsPanel); new $$_lenaPstrelAgaL2C({ target: __sveltets_2_any(), props: { cluster,}});}
      }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ClusterViewer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ClusterViewer__SvelteComponent_ = ReturnType<typeof ClusterViewer__SvelteComponent_>;
/*Ωignore_endΩ*/export default ClusterViewer__SvelteComponent_;