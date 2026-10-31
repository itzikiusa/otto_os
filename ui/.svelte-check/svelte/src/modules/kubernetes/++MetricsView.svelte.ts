///<reference types="svelte" />
;// Drawer "Metrics" tab: per-container CPU / memory bars for one pod from
// `GET …/metrics?ns=` (metrics-server via `kubectl top`). Bars are relative
// to the pod total (requests/limits aren't in the payload); refreshed every
// 10 s while the tab is open. Below the snapshot, "History (Monitor)" reads
// this pod's memory / request-rate series from the Monitor (K-2) and links
// to the workload's row there.

import { untrack } from 'svelte';
import { router } from '../../lib/router.svelte';
import { k8s } from '../../lib/stores/k8s.svelte';
import Sparkline from './monitor/Sparkline.svelte';
import { monitorPath } from '../../lib/k8s/viewState';
import { pollWhileVisible } from '../../lib/poll';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText, providerErrorText } from '../../lib/loadError';
import { k8sApi } from '../../lib/api/k8s';
import type { K8sMonitorSeries, K8sPodMetrics } from '../../lib/api/types';
import { formatBytes, formatMillicores } from './k8s-util';
;

  interface Props {
    clusterId: string;
    ns: string;
    pod: string;
    /** The pod's owning workload (from its ownerReferences), when known. */
    workload?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { clusterId, ns, pod, workload = '' }: Props = $props();

  // --- History (Monitor) ------------------------------------------------------
  let history = $state<{ mem: K8sMonitorSeries | null; rps: K8sMonitorSeries | null } | null>(null);
  let historyError = $state('');
  let historyLoading = $state(true);
  let historySeq = 0;
  async function loadHistory(): Promise<void> {
    const seq = ++historySeq;
    historyLoading = true;
    try {
      const [ws, sys, rps] = await Promise.all([
        k8sApi.monitorSeries(clusterId, { metric: 'mem_working_set_bytes', pod, ns: ns || undefined, window: '1h' }),
        k8sApi.monitorSeries(clusterId, { metric: 'mem_sys_bytes', pod, ns: ns || undefined, window: '1h' }),
        k8sApi.monitorSeries(clusterId, { metric: 'http_requests_total', pod, ns: ns || undefined, window: '1h' }),
      ]);
      if (seq !== historySeq) return;
      history = { mem: ws.points.length ? ws : sys.points.length ? sys : null, rps: rps.points.length ? rps : null };
      historyError = '';
    } catch (e) {
      if (seq !== historySeq) return;
      historyError = providerErrorText(e);
    } finally {
      if (seq === historySeq) historyLoading = false;
    }
  }
  $effect(() => {
    void clusterId;
    void pod;
    untrack(() => void loadHistory());
    return () => {
      historySeq++;
    };
  });
  function openInMonitor(): void {
    if (workload) k8s.saveMonitorUi(clusterId, { expanded: `${ns}/${workload}`, filter: workload });
    router.go(monitorPath(clusterId));
  }

  let metrics = $state<K8sPodMetrics | null>(null);
  let available = $state(true);
  let loading = $state(true);
  let error = $state('');

  // perf K8s R5: ask for THIS pod only (`?pod=`), abort with the poller and
  // drop an answer that belongs to a pod the drawer has since left.
  let metricsSeq = 0;
  let poller: { now(): void } | null = null;
  async function load(signal?: AbortSignal): Promise<void> {
    const seq = ++metricsSeq;
    const [c, n, p] = [clusterId, ns, pod];
    try {
      const r = await k8sApi.metrics(c, n, signal, p);
      if (seq !== metricsSeq) return;
      available = r.available;
      metrics = r.pods.find((m) => m.name === p && m.namespace === n) ?? null;
      error = '';
    } catch (e) {
      if (seq !== metricsSeq || signal?.aborted) return;
      error = loadErrorText(e);
    } finally {
      if (seq === metricsSeq) loading = false;
    }
  }

  $effect(() => {
    // Re-arm when the pod changes; the fetches themselves are untracked.
    void clusterId;
    void ns;
    void pod;
    loading = true;
    metrics = null;
    const t = untrack(() => pollWhileVisible((signal) => load(signal), { ms: 10_000 }));
    poller = t;
    return () => {
      t.stop();
      metricsSeq++;
      poller = null;
    };
  });

  const maxCpu = $derived(Math.max(1, ...(metrics?.containers.map((c) => c.cpu_millicores) ?? [1])));
  const maxMem = $derived(Math.max(1, ...(metrics?.containers.map((c) => c.mem_bytes) ?? [1])));
;
async () => {

 { svelteHTML.createElement("div", { "class":`metrics`,});
  
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`metrics`,loading,error,"empty":!metrics,"rows":3,"onretry":() => (poller ? void poller.now() : void load()),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
      if(!available){
         { svelteHTML.createElement("div", { "class":`dim`,});        { svelteHTML.createElement("span", { "class":`mono`,});  }     }
      }else{
         { svelteHTML.createElement("div", { "class":`dim`,});                }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
    if(metrics){
       { svelteHTML.createElement("div", { "class":`totals`,});
         { svelteHTML.createElement("div", { "class":`tot`,}); { svelteHTML.createElement("span", { "class":`lbl`,});  } { svelteHTML.createElement("span", { "class":`val mono`,});formatMillicores(metrics.cpu_millicores); } }
         { svelteHTML.createElement("div", { "class":`tot`,}); { svelteHTML.createElement("span", { "class":`lbl`,});  } { svelteHTML.createElement("span", { "class":`val mono`,});formatBytes(metrics.mem_bytes); } }
       }
       { svelteHTML.createElement("div", { "class":`containers`,});
           for(let c of __sveltets_2_ensureArray(metrics.containers)){c.name;
           { svelteHTML.createElement("div", { "class":`ctr`,});
             { svelteHTML.createElement("div", { "class":`ctr-name mono`,});c.name; }
             { svelteHTML.createElement("div", { "class":`bar-row`,});
               { svelteHTML.createElement("span", { "class":`lbl`,});  }
               { svelteHTML.createElement("div", {           "class":`bar`,"role":`meter`,"aria-label":`${c.name} CPU`,"aria-valuemin":0,"aria-valuemax":maxCpu,"aria-valuenow":c.cpu_millicores,});
                 { svelteHTML.createElement("div", {   "class":`fill cpu`,"style":`width:${(100 * c.cpu_millicores) / maxCpu}%`,}); }
               }
               { svelteHTML.createElement("span", { "class":`val mono`,});formatMillicores(c.cpu_millicores); }
             }
             { svelteHTML.createElement("div", { "class":`bar-row`,});
               { svelteHTML.createElement("span", { "class":`lbl`,});  }
               { svelteHTML.createElement("div", {           "class":`bar`,"role":`meter`,"aria-label":`${c.name} memory`,"aria-valuemin":0,"aria-valuemax":maxMem,"aria-valuenow":c.mem_bytes,});
                 { svelteHTML.createElement("div", {   "class":`fill mem`,"style":`width:${(100 * c.mem_bytes) / maxMem}%`,}); }
               }
               { svelteHTML.createElement("span", { "class":`val mono`,});formatBytes(c.mem_bytes); }
             }
           }
        }
       }
       { svelteHTML.createElement("div", { "class":`dim small`,});              }
  
    }
   LoadState}

   { svelteHTML.createElement("section", {     "class":`history`,"aria-label":`History from the Monitor`,...__sveltets_2_empty({"data-testid":`k8s-metrics-history`}),});
     { svelteHTML.createElement("div", { "class":`hist-head`,});
       { svelteHTML.createElement("span", { "class":`hist-title`,});     }
       { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":openInMonitor,"title":workload ? `Open ${workload} in the Monitor` : 'Open the Monitor for this cluster',...__sveltets_2_empty({"data-testid":`k8s-metrics-open-monitor`}),});   }
     }
    if(historyLoading && !history){
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":2,"height":30,}});}
    } else if (historyError){
       { svelteHTML.createElement("div", { "class":`dim`,});        historyError;  { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void loadHistory(),});  } }
    } else if (!history?.mem && !history?.rps){
       { svelteHTML.createElement("div", { "class":`dim`,});                     }
    }else{
      if(history?.mem){
         { svelteHTML.createElement("div", { "class":`hist-row`,}); { svelteHTML.createElement("span", { "class":`lbl`,});  } { const $$_enilkrapS3C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS3C({ target: __sveltets_2_any(), props: {        "points":history.mem.points.map((p) => p.v),"width":260,"height":36,"label":`memory history`,}});} { svelteHTML.createElement("span", { "class":`val mono`,});formatBytes(history.mem.points[history.mem.points.length - 1]?.v ?? 0); } }
      }
      if(history?.rps){
         { svelteHTML.createElement("div", { "class":`hist-row`,}); { svelteHTML.createElement("span", { "class":`lbl`,});  } { const $$_enilkrapS3C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS3C({ target: __sveltets_2_any(), props: {          "points":history.rps.points.map((p) => p.v),"width":260,"height":36,"stroke":`var(--status-working)`,"label":`request-rate history`,}});} { svelteHTML.createElement("span", { "class":`val mono`,});(history.rps.points[history.rps.points.length - 1]?.v ?? 0).toFixed(2); } }
      }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MetricsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MetricsView__SvelteComponent_ = ReturnType<typeof MetricsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default MetricsView__SvelteComponent_;