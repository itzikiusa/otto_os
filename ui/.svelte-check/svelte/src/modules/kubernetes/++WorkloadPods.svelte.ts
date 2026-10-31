///<reference types="svelte" />
;// Drawer "Pods" tab for a workload (Deployment / StatefulSet / DaemonSet /
// ReplicaSet / Job / Rollout): the pods its `spec.selector` matches, with
// ready / status / restarts / CPU / MEM (metrics-server, when it answers) /
// age, refreshed every 10 s while visible. Each row jumps to that pod's own
// drawer; Logs / Shell open it straight on those tabs.

import { untrack } from 'svelte';
import { pollWhileVisible } from '../../lib/poll';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { isAbortError } from '../../lib/api/client';
import { k8sApi } from '../../lib/api/k8s';
import type { K8sRow } from '../../lib/api/types';
import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
import { formatAge, formatBytes, formatMillicores, healthTone, rowAge } from './k8s-util';
;

  interface Props {
    clusterId: string;
    ns: string;
    selector: string;
    canEdit: boolean;
    onopenpod: (pod: string, tab?: K8sDrawerTab) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { clusterId, ns, selector, canEdit, onopenpod }: Props = $props();

  const REFRESH_MS = 10_000;
  let pods: K8sRow[] = $state([]);
  let hasMetrics = $state(false);
  let loading = $state(true);
  let error = $state('');
  let abort: AbortController | null = null;

  async function load(quiet = false): Promise<void> {
    abort?.abort();
    const ac = new AbortController();
    abort = ac;
    if (!quiet) loading = true;
    try {
      const r = await k8sApi.resources(clusterId, 'pods', { ns, label: selector }, ac.signal);
      if (ac.signal.aborted) return;
      pods = [...r.items].sort((a, b) => a.name.localeCompare(b.name));
      hasMetrics = r.has_metrics;
      error = '';
    } catch (e) {
      if (ac.signal.aborted || isAbortError(e)) return;
      error = loadErrorText(e);
    } finally {
      if (abort === ac) loading = false;
    }
  }

  $effect(() => {
    void clusterId;
    void ns;
    void selector;
    // A different cluster/namespace/selector must not show the previous one's pods.
    untrack(() => {
      pods = [];
      void load();
    });
    const t = pollWhileVisible(() => load(true), { ms: REFRESH_MS, immediate: false });
    return () => {
      t.stop();
      abort?.abort();
    };
  });

  const totals = $derived.by(() => {
    let ready = 0;
    let restarts = 0;
    let cpu = 0;
    let mem = 0;
    for (const p of pods) {
      const [r, t] = (p.ready ?? '0/0').split('/').map(Number);
      if (r === t && t > 0) ready++;
      restarts += p.restarts ?? 0;
      cpu += p.cpu ?? 0;
      mem += p.mem ?? 0;
    }
    return { ready, restarts, cpu, mem };
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`wp`,});
   { svelteHTML.createElement("div", { "class":`wp-sum`,});
     { svelteHTML.createElement("span", {}); { svelteHTML.createElement("b", {});pods.length; }  }
     { svelteHTML.createElement("span", {}); { svelteHTML.createElement("b", {});totals.ready; }  }
     { svelteHTML.createElement("span", { });totals.restarts > 0; { svelteHTML.createElement("b", {});totals.restarts; }  }
    if(hasMetrics){ { svelteHTML.createElement("span", { "class":`mono`,});formatMillicores(totals.cpu);  formatBytes(totals.mem); }}
     { svelteHTML.createElement("span", { "class":`spacer`,}); }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void load(true),"title":`Refresh`,"aria-label":`Refresh pods`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
   }
  
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`pods`,loading,error,"empty":!pods.length,"onretry":() => void load(),"variant":`panel`,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`dim pad`,});    { svelteHTML.createElement("code", { "class":`mono`,});selector; }  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
       { svelteHTML.createElement("div", {  "class":`wp-head`,});hasMetrics;
         { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("span", { "class":`num`,});  } { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("span", {   "class":`num`,"title":`Restarts`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
        if(hasMetrics){ { svelteHTML.createElement("span", { "class":`num`,});  } { svelteHTML.createElement("span", { "class":`num`,});  }}
         { svelteHTML.createElement("span", { "class":`num`,});  } { svelteHTML.createElement("span", {}); }
       }
         for(let p of __sveltets_2_ensureArray(pods)){p.name;
        const tone = healthTone(p.health, p.status);
        
         { svelteHTML.createElement("div", {  "class":`wp-row`,});hasMetrics;
           { svelteHTML.createElement("button", {       "type":`button`,"class":`wp-name mono ell`,"title":`Open ${p.name}`,"onclick":() => onopenpod(p.name),});p.name; }
           { svelteHTML.createElement("span", { "class":`num mono`,});p.ready ?? ''; }
           { svelteHTML.createElement("span", { "class":`wp-status`,}); { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },tone,"dot":true,"live":tone === 'info',}}); { svelteHTML.createElement("span", { "class":`ell`,});p.status; } Badge} }
           { svelteHTML.createElement("span", {  "class":`num mono`,});(p.restarts ?? 0) > 0;p.restarts ?? ''; }
          if(hasMetrics){
             { svelteHTML.createElement("span", { "class":`num mono`,});p.cpu == null ? '' : formatMillicores(p.cpu); }
             { svelteHTML.createElement("span", { "class":`num mono`,});p.mem == null ? '' : formatBytes(p.mem); }
          }
           { svelteHTML.createElement("span", { "class":`num mono`,});formatAge(rowAge(p)); }
           { svelteHTML.createElement("span", { "class":`acts`,});
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => onopenpod(p.name, 'logs'),"title":`Logs of ${p.name}`,"aria-label":`Logs of ${p.name}`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} }
            if(canEdit){ { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => onopenpod(p.name, 'terminal'),"title":`Shell into ${p.name}`,"aria-label":`Shell into ${p.name}`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});} }}
           }
         }
      }
  
   LoadState}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WorkloadPods__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkloadPods__SvelteComponent_ = ReturnType<typeof WorkloadPods__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkloadPods__SvelteComponent_;