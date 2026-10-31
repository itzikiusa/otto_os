///<reference types="svelte" />
;
import { plural } from '../../../lib/plural';
// Kubernetes box: one row per registered cluster from the Monitor overview
// (health, pods, restarts, memory, rps, error %). Falls back to the plain
// cluster list (name + env + version) when the monitor is not collecting
// (ClickHouse off / monitoring disabled) so the box is never blank for a
// user who has clusters but no metrics.
import { untrack } from 'svelte';
import { radioKey } from '../../../lib/radioKey';
import Icon from '../../../lib/components/Icon.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import { k8s } from '../../../lib/stores/k8s.svelte';
import { k8sApi } from '../../../lib/api/k8s';
import { router } from '../../../lib/router.svelte';
import type { K8sMonitorOverviewRow } from '../../../lib/api/types';
import { formatBytes } from '../../kubernetes/k8s-util';
import EnvBadge from '../../../lib/components/EnvBadge.svelte';
import { WINDOWS, fmtPct, fmtRate, healthLabel, isWindow } from '../../kubernetes/monitor/monitor-util';
import { loadErrorText } from '../../../lib/loadError';
import { home, type HomeBox } from '../home.svelte';
import { freshness, poll, type Poller } from './poll';
;

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
    /** False while this box's Home space is not on screen (or another box
     *  is zoomed): the poller stops, the data stays (review 06 F3). */
    active?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { box, viewId, zoomed: _zoomed, tick, active = true }: Props = $props();

  const win = $derived(isWindow(String(box.config.window ?? '')) ? (box.config.window as (typeof WINDOWS)[number]) : '24h');

  let rows = $state<K8sMonitorOverviewRow[]>([]);
  let loading = $state(true);
  let error = $state('');
  let booted = false;
  let seq = 0;

  async function load(signal?: AbortSignal): Promise<boolean> {
    // perf K8s R5: status, the cluster list and the overview start together
    // (they were three serial round-trips on mount); the overview is aborted
    // with the poller when the box unmounts or leaves the screen.
    const mine = ++seq;
    try {
      const boot = booted ? null : Promise.all([k8s.loadStatus(), k8s.loadClusters()]);
      booted = true;
      const overview = k8sApi.monitorOverview(win, signal);
      if (boot) await boot;
      if (k8s.unavailable) {
        overview.catch(() => undefined);
        return true;
      }
      const next = await overview;
      if (mine !== seq) return true;
      rows = next;
      error = '';
      return true;
    } catch (e) {
      // An aborted read (the space went off screen mid-request) loaded
      // nothing: `false` keeps `freshness` from stamping it fresh, so coming
      // back fetches at once instead of showing empty rows for a minute.
      if (signal?.aborted) return false;
      if (mine !== seq) return true;
      error = loadErrorText(e);
      return false;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  let poller: Poller | null = null;
  const fresh = freshness(load);
  $effect(() => {
    const key = win;
    if (!active) return;
    poller?.stop();
    poller = poll(fresh.run, 60_000, fresh.start(key, 60_000));
    return () => poller?.stop();
  });
  // Manual refresh (frame button) runs interactive; a collection cycle
  // (coalesced by the store, never while hidden) on the background lane.
  // Only CHANGES count — the poller's own first run already covers mount.
  let seenTick = untrack(() => tick);
  let seenCycle = untrack(() => k8s.monitorTick);
  $effect(() => {
    const t = tick;
    if (t === seenTick) return;
    seenTick = t;
    untrack(() => poller?.now());
  });
  $effect(() => {
    const c = k8s.monitorTick;
    if (c === seenCycle) return;
    seenCycle = c;
    untrack(() => poller?.now({ background: true }));
  });

  // Overview rows keyed by cluster id; clusters with no row fall back to the
  // registry entry so every saved cluster shows.
  const byId = $derived(new Map(rows.map((r) => [r.cluster.id, r])));
  const monitored = $derived(rows.some((r) => r.enabled && r.status));
;
async () => {

 { svelteHTML.createElement("div", { "class":`k8s`,});
  
  if(k8s.clusters.length > 0 && !k8s.unavailable){
     { svelteHTML.createElement("div", { "class":`bar`,});
       { svelteHTML.createElement("span", { "class":`dim`,});plural(k8s.clusters.length, 'cluster'); }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("div", {     "class":`seg`,"role":`radiogroup`,"aria-label":`Window`,});
           for(let w of __sveltets_2_ensureArray(WINDOWS)){w;
           { svelteHTML.createElement("button", {           "role":`radio`,"aria-checked":w === win,"tabindex":w === win ? 0 : -1,"onkeydown":radioKey,"onclick":() => home.updateBoxConfig(viewId, box.id, { window: w }),});w === win;w; }
        }
       }
     }
  }
  if(loading && k8s.clusters.length === 0){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {  "rows":3,}});}
  } else if (k8s.unavailable){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`helm`,"title":`Kubernetes console is off`,"body":`Enable it in the daemon to see clusters here.`,}});}
  } else if (k8s.clusters.length === 0 && k8s.clustersError){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`clusters`,"variant":`compact`,"error":k8s.clustersError,"empty":true,"onretry":() => void k8s.loadClusters(),}});}
  } else if (k8s.clusters.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"icon":`helm`,"title":`No clusters yet`,"body":`Add a kubeconfig context on the Kubernetes page.`,}});
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('kubernetes'),});  }
     EmptyState}
  }else{
    if(error && !monitored){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`Kubernetes monitor`,"variant":`compact`,error,"empty":false,"onretry":() => poller?.now(),}});
         { svelteHTML.createElement("span", { "class":`dim`,});    }
       LoadState}
    }
     { svelteHTML.createElement("ul", { "class":`rows`,});
         for(let c of __sveltets_2_ensureArray(k8s.clusters)){c.id;
        const r = byId.get(c.id);
        const h = healthLabel(r?.health ?? 'unknown');
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {     "class":`k8s-row`,"onclick":() => router.go(`kubernetes/${encodeURIComponent(c.id)}`),"title":`Open ${c.name}`,});
             { svelteHTML.createElement("span", {  "class":`cdot`,});__sveltets_2_ensureType(String, Number, c.color ?? 'var(--text-dim)'); }
             { svelteHTML.createElement("span", { "class":`name ellipsis`,});c.name; }
             { const $$_egdaBvnE4C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE4C({ target: __sveltets_2_any(), props: {  "env":c.environment,}});}
             { svelteHTML.createElement("span", { "class":`health ${h.cls}`,});h.label; }
            if(r && r.enabled && r.status){
               { svelteHTML.createElement("span", {   "class":`m`,"title":`Pods running / total`,});
                 { svelteHTML.createElement("b", {});r.pods.running; } r.pods.total;
                if(r.pods.crashloop > 0){ { svelteHTML.createElement("em", { "class":`bad`,});r.pods.crashloop;  }}
                if(r.pods.pending > 0){ { svelteHTML.createElement("em", { "class":`warn`,});r.pods.pending;  }}
               }
               { svelteHTML.createElement("span", {   "class":`m`,"title":`Memory used vs limits`,});
                 { svelteHTML.createElement("span", { "class":`meter`,}); { svelteHTML.createElement("i", {   });__sveltets_2_ensureType(String, Number, `${Math.min(100, r.mem.pct)} `);r.mem.pct >= 80;r.mem.pct >= 95; } }
                fmtPct(r.mem.pct, 0);  formatBytes(r.mem.used);
               }
               { svelteHTML.createElement("span", {   "class":`m`,"title":`Requests / errors`,});fmtRate(r.rps);    { svelteHTML.createElement("span", {  });r.err_pct >= 5;r.err_pct >= 1;fmtPct(r.err_pct);  } }
              if(r.drift.length > 0){ { svelteHTML.createElement("span", {   "class":`m warn`,"title":`Version drift`,});r.drift.length;  }}
            }else{
               { svelteHTML.createElement("span", { "class":`m dim`,});c.capabilities?.server_version ?? c.context_name; }
            }
           }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const K8sBox__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type K8sBox__SvelteComponent_ = ReturnType<typeof K8sBox__SvelteComponent_>;
/*Ωignore_endΩ*/export default K8sBox__SvelteComponent_;