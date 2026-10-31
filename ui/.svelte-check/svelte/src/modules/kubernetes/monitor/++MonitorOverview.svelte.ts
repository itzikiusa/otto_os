///<reference types="svelte" />
;
import { rel } from '../../../lib/stores/now.svelte';
import { plural } from '../../../lib/plural';
// Monitor overview: one card per registered cluster (health badge, pods,
// restarts by class, memory vs limits, rps / error %, version drift, and the
// collector status line with the exact metrics-server RBAC message when it
// is denied). Refreshes on WS `k8s_monitor_cycle` and on a window change.
import { loadErrorText } from '../../../lib/loadError';
import { untrack } from 'svelte';
import { radioKey } from '../../../lib/radioKey';
import { router } from '../../../lib/router.svelte';
import { k8s } from '../../../lib/stores/k8s.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { copyTextOrThrow } from '../../../lib/clipboard';
import { k8sApi } from '../../../lib/api/k8s';
import type { K8sMonitorOverviewRow } from '../../../lib/api/types';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import PageHeader from '../../../lib/components/PageHeader.svelte';
import PageBody from '../../../lib/components/PageBody.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { formatBytes } from '../k8s-util';
import EnvBadge from '../../../lib/components/EnvBadge.svelte';
import { WINDOWS, classColor, classLabel, collectorLine, fmtPct, fmtRate, healthLabel, isWindow, rbacMessage } from './monitor-util';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let window = $state<(typeof WINDOWS)[number]>('24h');
  let rows = $state<K8sMonitorOverviewRow[]>([]);
  let loading = $state(true);
  let error = $state('');
  let abort: AbortController | null = null;

  try {
    const saved = localStorage.getItem('otto_k8s_monitor_window');
    if (isWindow(saved ?? undefined)) window = saved as (typeof WINDOWS)[number];
  } catch {
    /* storage unavailable */
  }

  async function load(quiet = false): Promise<void> {
    abort?.abort();
    abort = new AbortController();
    if (!quiet) loading = true;
    try {
      rows = await k8sApi.monitorOverview(window, abort.signal);
      error = '';
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return;
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const w = window;
    try {
      localStorage.setItem('otto_k8s_monitor_window', w);
    } catch {
      /* ignore */
    }
    untrack(() => void load());
  });

  // Live refresh after collection cycles (coalesced by the store, paused
  // while hidden). Only a NEW tick — the window effect covers mount.
  let seenTick = untrack(() => k8s.monitorTick);
  $effect(() => {
    const t = k8s.monitorTick;
    if (t === seenTick) return;
    seenTick = t;
    untrack(() => void load(true));
  });

  function open(row: K8sMonitorOverviewRow, tab = 'workloads'): void {
    router.go(`kubernetes/${encodeURIComponent(row.cluster.id)}/monitor/${tab}`);
  }

  async function copy(text: string): Promise<void> {
    try {
      // The shared helper falls back to execCommand when the async Clipboard
      // API is refused (unfocused / insecure webview) — the raw call failed there.
      await copyTextOrThrow(text);
      toasts.success('Copied', 'RBAC message copied to the clipboard.');
    } catch {
      toasts.error('Couldn’t copy', `Select the message and copy it manually: ${text}`);
    }
  }

  function restartTotal(r: K8sMonitorOverviewRow): number {
    return r.restarts.oom + r.restarts.crash + r.restarts.probe + r.restarts.unknown;
  }
  const CLASSES = ['oom', 'crash', 'probe', 'unknown'] as const;
;
async () => {

 { svelteHTML.createElement("div", {   "class":`mon-ov`,...__sveltets_2_empty({"data-testid":`k8s-monitor-overview`}),});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {      "title":`Monitor`,"crumbs":[{ label: 'Kubernetes', onclick: () => router.go('kubernetes') }],"subtitle":`Pod metrics, restarts and a health digest per cluster`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", {      "class":`segmented`,"role":`radiogroup`,"aria-label":`Window`,...__sveltets_2_empty({"data-keep":true}),});
         for(let w of __sveltets_2_ensureArray(WINDOWS)){w;
         { svelteHTML.createElement("button", {         "role":`radio`,"onkeydown":radioKey,"aria-checked":window === w,"tabindex":window === w ? 0 : -1,"onclick":() => (window = w),});w; }
      }
     }
     { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => router.go('kubernetes/monitor/fleet'),"title":`One dashboard over every cluster — restarts, memory, req/s, latency — read from ClickHouse only`,...__sveltets_2_empty({"data-testid":`k8s-monitor-fleet-link`}),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chart`,"size":12,}});}   }
     { svelteHTML.createElement("button", {           "class":`icon-btn`,...__sveltets_2_empty({"data-icon":`refresh`}),...__sveltets_2_empty({"data-label":`Refresh overview`}),"onclick":() => void load(),"title":`Refresh`,"aria-label":`Refresh overview`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});} }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
 { svelteHTML.createElement("div", { "class":`mon-ov-body`,});

  if(loading && !rows.length){
     { svelteHTML.createElement("div", { "class":`grid`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {    "rows":3,"height":140,}});} }
  } else if (error && !rows.length){
     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {     "what":`the overview`,error,"empty":true,"onretry":() => void load(),}});}
  } else if (!rows.length){
     { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {          "icon":`helm`,"title":`No clusters yet`,"body":`Add a cluster in the Kubernetes console first, then enable monitoring on it here. Pod-level metrics come from your services’ own endpoints.`,"actionLabel":`Open clusters`,"onaction":() => router.go('kubernetes'),}});}
  }else{
     { svelteHTML.createElement("div", {   "class":`grid`,...__sveltets_2_empty({"data-testid":`k8s-monitor-grid`}),});
         for(let r of __sveltets_2_ensureArray(rows)){r.cluster.id;
        const h = healthLabel(r.health);
        const total = restartTotal(r);
        const rbac = rbacMessage(r.status?.metrics_server);
        
         { svelteHTML.createElement("div", {      "class":`card cluster`,...__sveltets_2_empty({"data-testid":`k8s-monitor-card`}),...__sveltets_2_empty({"data-health":r.health}),});!r.enabled;
           { svelteHTML.createElement("div", { "class":`row1`,});
             { svelteHTML.createElement("span", {   "class":`dot`,"style":`background: ${r.cluster.color ?? 'var(--accent)'}`,}); }
             { svelteHTML.createElement("button", {       "type":`button`,"class":`name card-open`,"onclick":() => open(r),...__sveltets_2_empty({"data-testid":`k8s-monitor-card-open`}),});r.cluster.name; }
             { const $$_egdaBvnE6C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE6C({ target: __sveltets_2_any(), props: {  "env":r.cluster.environment,}});}
             { svelteHTML.createElement("span", {   "class":`health ${h.cls}`,...__sveltets_2_empty({"data-testid":`k8s-monitor-health`}),});h.label; }
           }

          if(!r.enabled){
             { svelteHTML.createElement("div", { "class":`off-body`,});
               { svelteHTML.createElement("span", { "class":`dim`,});      }
               { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => open(r, 'settings'),});  }
             }
          }else{
             { svelteHTML.createElement("div", { "class":`stats`,});
               { svelteHTML.createElement("div", { "class":`stat`,});
                 { svelteHTML.createElement("span", { "class":`k`,});  }
                 { svelteHTML.createElement("span", { "class":`v mono`,});r.pods.running; { svelteHTML.createElement("span", { "class":`dim`,}); r.pods.total; } }
                if(r.pods.pending || r.pods.failed || r.pods.crashloop){
                   { svelteHTML.createElement("span", { "class":`note`,});
                    if(r.pods.pending){r.pods.pending; }
                    if(r.pods.failed){r.pods.pending ? ' · ' : '';r.pods.failed; }
                    if(r.pods.crashloop){r.pods.pending || r.pods.failed ? ' · ' : '';r.pods.crashloop; }
                   }
                }
               }
               { svelteHTML.createElement("div", { "class":`stat`,});
                 { svelteHTML.createElement("span", { "class":`k`,});  { svelteHTML.createElement("span", { "class":`dim`,}); r.window;  } }
                 { svelteHTML.createElement("span", { "class":`v mono`,});total; { svelteHTML.createElement("span", { "class":`dim`,});' ';  } }
                 { svelteHTML.createElement("span", { "class":`note`,});plural(r.churn, 'planned replacement'); }
               }
               { svelteHTML.createElement("div", { "class":`stat`,});
                 { svelteHTML.createElement("span", { "class":`k`,});  }
                 { svelteHTML.createElement("span", { "class":`v mono`,});formatBytes(r.mem.used);if(r.mem.limit > 0){ { svelteHTML.createElement("span", { "class":`dim`,});' / ';formatBytes(r.mem.limit); }} }
                if(r.mem.limit > 0){
                   { svelteHTML.createElement("div", {   "class":`bar`,"title":`${fmtPct(r.mem.pct)} of limits`,}); { svelteHTML.createElement("div", {    "class":`fill`,"style":`width: ${Math.min(100, r.mem.pct)}%`,});r.mem.pct >= 85; } }
                }
               }
               { svelteHTML.createElement("div", { "class":`stat`,});
                 { svelteHTML.createElement("span", { "class":`k`,});  }
                 { svelteHTML.createElement("span", { "class":`v mono`,});fmtRate(r.rps); }
                 { svelteHTML.createElement("span", {  "class":`note`,});r.err_pct >= 1;fmtPct(r.err_pct, 2);  }
               }
             }

            if(total > 0){
               { svelteHTML.createElement("div", {   "class":`stack`,"aria-label":`Restarts by class`,});
                   for(let c of __sveltets_2_ensureArray(CLASSES)){c;
                  if(r.restarts[c] > 0){
                     { svelteHTML.createElement("div", {     "class":`seg-bar`,"style":`width: ${(100 * r.restarts[c]) / total}%; background: ${classColor(c)}`,"title":`${classLabel(c)}: ${r.restarts[c]}`,}); }
                  }
                }
               }
               { svelteHTML.createElement("div", { "class":`legend`,});
                   for(let c of __sveltets_2_ensureArray(CLASSES)){c;
                  if(r.restarts[c] > 0){
                     { svelteHTML.createElement("span", { "class":`lg`,}); { svelteHTML.createElement("i", { "style":`background: ${classColor(c)}`,}); }classLabel(c); r.restarts[c]; }
                  }
                }
               }
            }

            if(r.drift.length){
               { svelteHTML.createElement("div", {   "class":`drift`,"title":r.drift.map((d) => `${d.workload}: ${d.versions.join(', ')}`).join('\n'),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});} plural(r.drift.length, 'workload');   
               }
            }

             { svelteHTML.createElement("div", {   "class":`status dim`,"title":r.status?.last_error || '',});collectorLine(r.status, r.enabled, rel); }
            if(rbac){
               { svelteHTML.createElement("div", { "class":`rbac`,});
                 { svelteHTML.createElement("span", {});            }
                 { svelteHTML.createElement("code", {});rbac; }
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copy(rbac),});  }
               }
            } else if (r.status?.metrics_server === 'ok'){
               { svelteHTML.createElement("div", { "class":`dim tiny`,});       }
            } else if (r.status?.metrics_server === 'disabled'){
               { svelteHTML.createElement("div", { "class":`dim tiny`,});     }
            }
          }
         }
      }
     }
  }
 }
 PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MonitorOverview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MonitorOverview__SvelteComponent_ = ReturnType<typeof MonitorOverview__SvelteComponent_>;
/*Ωignore_endΩ*/export default MonitorOverview__SvelteComponent_;