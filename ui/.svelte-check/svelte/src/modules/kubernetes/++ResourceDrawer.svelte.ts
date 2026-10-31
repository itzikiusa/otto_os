///<reference types="svelte" />
;
  import { plural } from '../../lib/plural';
  /** When the previous drawer instance mounted. The workspace re-keys the
   *  drawer per target, so a mount right after another one is j/k navigation. */
  let lastMountAt = 0;
;;

import DockedDrawer from '../../lib/components/DockedDrawer.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { actionOperation } from './permissions';
// Detail drawer for the selected row: Overview (normalized fields + action
// buttons) / Manifest (YAML, read-only CodeEditor; secrets already redacted
// server-side) / Describe / Events, and for pods Logs / Terminal / Metrics.
// Detail + container list are fetched once per selection (aborted when the
// selection moves on).
import { untrack } from 'svelte';
import { stringify as toYaml } from 'yaml';
import Icon from '../../lib/components/Icon.svelte';
import Tabs from '../../lib/components/Tabs.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import { copyText } from '../../lib/clipboard';
import { toasts } from '../../lib/toast.svelte';
import { isAbortError } from '../../lib/api/client';
import { k8sApi } from '../../lib/api/k8s';
import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
import type { K8sContainer, K8sResourceDetail, K8sResourceKind, K8sRow } from '../../lib/api/types';
import type { ActionDef } from './actions';
import { actionsFor } from './actions';
import { clipLongScalars, formatAge, formatBytes, formatMillicores, healthTone, kindDef, podContainers, rowAge } from './k8s-util';
import LogsView from './LogsView.svelte';
import ExecView from './ExecView.svelte';
import MetricsView from './MetricsView.svelte';
import WorkloadPods from './WorkloadPods.svelte';
import PodHttpPanel from './PodHttpPanel.svelte';
import { workloadKindFor } from './podHttp';
;

  interface Props {
    modal?: boolean;
    /** Desktop column width (the workspace's splitter owns it). */
    width?: string;
    clusterId: string;
    kind: K8sResourceKind;
    ns: string;
    name: string;
    /** The table row when it’s loaded (null while the list is still loading
     *  or the object vanished — the drawer then leans on the manifest). */
    row: K8sRow | null;
    tab: K8sDrawerTab;
    canEdit: boolean;
    /** Open the shell straight away (`s` shortcut). */
    autoExec?: boolean;
    /** The Terminal tab acted on `autoExec` — the owner clears it (one-shot
     *  per open, not per row: re-showing the tab must not open a new shell). */
    onautoexec?: () => void;
    ontab: (t: K8sDrawerTab) => void;
    onclose: () => void;
    onaction: (def: ActionDef, row: K8sRow) => void;
    /** Workloads: jump to one of this object’s pods (its own drawer). */
    onopenpod?: (ns: string, pod: string, tab?: K8sDrawerTab) => void;
    /** Bumped by the workspace after an action on this object (KS-4): the
     *  drawer re-reads its detail quietly. */
    reloadNonce?: number;
    /** Open this workload’s row in the Monitor view (K-2). */
    onmonitor?: (ns: string, workload: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { modal = false, width, clusterId, kind, ns, name, row, tab, canEdit, autoExec = false, onautoexec, ontab, onclose, onaction, onopenpod, reloadNonce = 0, onmonitor }: Props = $props();

  $effect(() => {
    void resourceAccess.load('k8s_cluster', clusterId);
    if (ns) void resourceAccess.load('k8s_cluster', clusterId, `namespace:${ns}`);
  });
  function canOperation(op: string): boolean {
    return resourceAccess.can('k8s_cluster', clusterId, op, 'kubernetes',
      ['exec', 'apply', 'scale', 'restart', 'delete'].includes(op) ? 'edit' : 'view', ns ? `namespace:${ns}` : undefined);
  }
  const canLogs = $derived(canOperation('logs'));
  const canExec = $derived(canOperation('exec'));
  const isPod = $derived(kind === 'pods');
  const def = $derived(kindDef(kind));
  /** The pod-http workload this drawer is, or — for a pod — the workload that
   *  owns it (ReplicaSet → its Deployment by the pod-template-hash suffix). */
  const httpWorkload = $derived.by((): { kind: string; name: string } | null => {
    const wk = workloadKindFor(kind);
    if (wk) return { kind: wk, name };
    if (!isPod) return null;
    const owner = (detail?.manifest as { metadata?: { ownerReferences?: { kind: string; name: string }[] } } | null)?.metadata?.ownerReferences?.[0];
    if (!owner) return null;
    if (owner.kind === 'ReplicaSet') {
      const hash = (detail?.manifest as { metadata?: { labels?: Record<string, string> } }).metadata?.labels?.['pod-template-hash'];
      return hash && owner.name.endsWith(`-${hash}`) ? { kind: 'deployment', name: owner.name.slice(0, -hash.length - 1) } : { kind: 'replicaset', name: owner.name };
    }
    const k = owner.kind.toLowerCase();
    return ['statefulset', 'daemonset', 'job'].includes(k) ? { kind: k, name: owner.name } : null;
  });
  const canHttp = $derived((isPod || !!workloadKindFor(kind)) && canOperation('workloads_view'));
  /** `spec.selector` of a workload (row extra, or the manifest when the row
   *  is gone) — unlocks the Pods + Logs tabs. */
  const selector = $derived.by((): string => {
    if (isPod) return '';
    if (row?.extra?.selector) return row.extra.selector;
    const m = (detail?.manifest as { spec?: { selector?: { matchLabels?: Record<string, string> } } } | null)?.spec?.selector?.matchLabels;
    return m ? Object.entries(m).map(([k, v]) => `${k}=${v}`).join(',') : '';
  });
  const TABS = $derived<{ id: K8sDrawerTab; label: string }[]>([
    { id: 'overview' as const, label: 'Overview' },
    ...(selector
      ? [
          { id: 'pods' as const, label: 'Pods' },
          { id: 'logs' as const, label: 'Logs' },
        ]
      : []),
    { id: 'manifest' as const, label: 'Manifest' },
    { id: 'describe' as const, label: 'Describe' },
    { id: 'events' as const, label: 'Events' },
    ...(isPod
      ? [
          { id: 'logs' as const, label: 'Logs' },
          { id: 'terminal' as const, label: 'Terminal' },
          { id: 'metrics' as const, label: 'Metrics' },
        ]
      : []),
    ...(canHttp ? [{ id: 'http' as const, label: 'HTTP' }] : []),
  ].filter(t => t.id === 'logs' ? canLogs : t.id === 'terminal' ? canExec : t.id === 'metrics' ? canOperation('metrics') : true));
  /** Container names across the workload’s pod template (Logs container filter). */
  const templateContainers = $derived.by((): K8sContainer[] => {
    if (isPod || !detail) return [];
    const spec = (detail.manifest as { spec?: { template?: { spec?: { containers?: { name: string }[]; initContainers?: { name: string }[] } } } }).spec?.template?.spec;
    const out: K8sContainer[] = [];
    for (const c of spec?.initContainers ?? []) out.push({ name: c.name, init: true } as K8sContainer);
    for (const c of spec?.containers ?? []) out.push({ name: c.name, init: false } as K8sContainer);
    return out;
  });

  let current: AbortController | null = null;
  let detail = $state<K8sResourceDetail | null>(null);
  let detailError = $state('');
  let detailLoading = $state(false);
  // Derived from the pod manifest `/resource` already returns (was a second
  // `/containers` call → another `kubectl get pod -o json` per open).
  const containers = $derived<K8sContainer[]>(isPod && detail ? podContainers(detail.manifest) : []);

  async function load(quiet = false): Promise<void> {
    const ac = new AbortController();
    current = ac;
    const sig = ac.signal;
    if (!quiet || !detail) {
      detail = null;
      detailLoading = true;
    }
    detailError = '';
    const cid = clusterId;
    const k = kind;
    const n = ns;
    const nm = name;
    await k8sApi
      .resource(cid, k, n, nm, sig)
      .then((d) => {
        if (!sig.aborted) detail = d;
      })
      .catch((e) => {
        if (!sig.aborted && !isAbortError(e)) detailError = loadErrorText(e);
      })
      .finally(() => {
        if (!sig.aborted) detailLoading = false;
      });
  }

  function retry(): void {
    current?.abort();
    void load();
  }

  /** Header refresh button / `reloadNonce`: re-read without blanking the tab. */
  let refreshing = $state(false);
  async function refresh(): Promise<void> {
    current?.abort();
    refreshing = true;
    try {
      await load(true);
    } finally {
      refreshing = false;
    }
  }
  let seenNonce = untrack(() => reloadNonce);
  $effect(() => {
    const n = reloadNonce;
    if (n === seenNonce) return;
    seenNonce = n;
    untrack(() => void refresh());
  });

  // Rapid target changes (j/k with the drawer open — ~30 Hz on key repeat,
  // each re-keying this drawer) settle for 150 ms before the get + describe +
  // events kubectl calls go out; the overview renders from the row meanwhile.
  // A deliberate open (nothing mounted just before) loads at once.
  $effect(() => {
    void clusterId;
    void kind;
    void ns;
    void name;
    let timer: ReturnType<typeof setTimeout> | undefined;
    untrack(() => {
      current?.abort();
      current = null;
      const now = performance.now();
      const rapid = now - lastMountAt < 400;
      lastMountAt = now;
      if (!rapid) {
        void load();
        return;
      }
      detail = null;
      detailError = '';
      detailLoading = true;
      timer = setTimeout(() => void load(), 150);
    });
    return () => {
      if (timer) clearTimeout(timer);
      current?.abort();
    };
  });

  function manifestText(manifest: unknown): string {
    try {
      return toYaml(manifest, { lineWidth: 0 });
    } catch {
      return JSON.stringify(manifest, null, 2);
    }
  }
  // The view shows scalars clipped at 64 KiB (SC-19: a 1 MB one-line value
  // is one enormous highlighted line) and soft-wraps; Copy re-serializes the
  // full manifest on demand.
  const clippedManifest = $derived(detail ? clipLongScalars(detail.manifest) : { value: null, clipped: 0 });
  const yaml = $derived(detail ? manifestText(clippedManifest.value) : '');
  function copyManifest(): void {
    if (!detail) return;
    void copyOrSay(clippedManifest.clipped ? manifestText(detail.manifest) : yaml, 'manifest');
  }
  /** ux-6-07: a refused copy says what to do next (it was silent). */
  async function copyOrSay(text: string, what: string): Promise<void> {
    if (await copyText(text)) toasts.success(`Copied ${what}`);
    else toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
  }

  const actions = $derived(row ? actionsFor(kind, row).filter(a => a.id.startsWith('argocd_') ? resourceAccess.can('k8s_cluster', clusterId, actionOperation(a.id), 'kubernetes', 'edit') : canOperation(actionOperation(a.id))) : []);

  /** Overview facts: normalized row fields first, then kind-specific extras. */
  const facts = $derived.by(() => {
    const out: [string, string][] = [];
    if (!row) return out;
    out.push(['Status', row.status]);
    if (row.ready) out.push(['Ready', row.ready]);
    if (row.restarts != null) out.push(['Restarts', String(row.restarts)]);
    out.push(['Age', formatAge(rowAge(row))]);
    if (row.node) out.push(['Node', row.node]);
    if (row.ip) out.push(['IP', row.ip]);
    if (row.cpu != null) out.push(['CPU', formatMillicores(row.cpu)]);
    if (row.mem != null) out.push(['Memory', formatBytes(row.mem)]);
    // Extras a fixed fact / section already covers.
    const skip = new Set(['ready', 'selector', 'phase', 'key_count']);
    for (const [k, v] of Object.entries(row.extra ?? {})) if (v && !skip.has(k)) out.push([k.replace(/_/g, ' '), v]);
    return out;
  });

  const statusTone = $derived(row ? healthTone(row.health, row.status) : 'neutral');

;
async () => {




 { const $$_rewarDdekcoD0C = __sveltets_2_ensureComponent(DockedDrawer); const $$_rewarDdekcoD0 = new $$_rewarDdekcoD0C({ target: __sveltets_2_any(), props: {      children:() => { return __sveltets_2_any(0); },"open":true,"title":`${def.singular} details`,onclose,modal,width,"testid":`k8s-drawer`,head:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", { "class":`dr-headrow`,});
       { svelteHTML.createElement("div", { "class":`dr-title`,});
         { svelteHTML.createElement("span", { "class":`dr-kind`,});def.singular; }
         { svelteHTML.createElement("span", {   "class":`dr-name mono`,"title":name,});name; }
        if(ns){ { svelteHTML.createElement("span", { "class":`dr-ns mono`,});ns; }}
        if(row){ { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {      "tone":statusTone,"label":row.status,"dot":true,"live":statusTone === 'info',}});}}
       }
       { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => void refresh(),"disabled":refreshing,"aria-label":`Refresh details`,"title":`Refresh details`,...__sveltets_2_empty({"data-testid":`k8s-drawer-refresh`}),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});} }
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {head} = $$_rewarDdekcoD0.$$prop_def;/*Ωignore_endΩ*/
  

   { const $$_sbaT1C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT1C({ target: __sveltets_2_any(), props: {          "label":`Detail tabs`,"idBase":`k8s`,"tabs":TABS,"value":tab,"onchange":ontab,}});}

   { svelteHTML.createElement("div", {       "class":`dr-body`,"role":`tabpanel`,"id":`k8s-panel-${tab}`,"aria-labelledby":`k8s-tab-${tab}`,});
    if(tab === 'overview'){
       { svelteHTML.createElement("div", { "class":`ov`,});
        if(row){
          if(isPod || selector || (canEdit && actions.length)){
             { svelteHTML.createElement("div", { "class":`ov-actions`,});
              if(isPod){
                 { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!canLogs,"title":canLogs ? undefined : "You don’t have permission to read logs in this namespace","onclick":() => ontab('logs'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}  }
                if(canExec){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => ontab('terminal'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}  }}
              } else if (selector){
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => ontab('pods'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});}  }
                 { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!canLogs,"title":canLogs ? undefined : "You don’t have permission to read logs in this namespace","onclick":() => ontab('logs'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}  }
              }
              if(onmonitor && httpWorkload){
                 { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => onmonitor(ns, httpWorkload.name),"title":`Open ${httpWorkload.name} in the Monitor (history, restarts, req/s)`,...__sveltets_2_empty({"data-testid":`k8s-drawer-monitor`}),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`gauge`,"size":12,}});}  }
              }
              if(canEdit){
                   for(let a of __sveltets_2_ensureArray(actions)){a.id + a.label;
                   { svelteHTML.createElement("button", {    "class":`btn small`,"onclick":() => onaction(a, row),});a.danger;
                    if(a.icon){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":a.icon,"size":12,}});}}a.label;
                   }
                }
              }
             }
          }
           { svelteHTML.createElement("dl", { "class":`facts`,});
               for(let [k, v] of __sveltets_2_ensureArray(facts)){k;
               { svelteHTML.createElement("dt", {});k; }
               { svelteHTML.createElement("dd", {   "title":v,});/ip|node|revision|repo|path|version|image/i.test(k);v; }
            }
           }
          if(row.images?.length){
             { svelteHTML.createElement("div", { "class":`sec`,});
               { svelteHTML.createElement("div", { "class":`sec-title`,});  }
                  for(let im of __sveltets_2_ensureArray(row.images)){let i = 1;i; { svelteHTML.createElement("div", {   "class":`mono small ell`,"title":im,});im; }}
             }
          }
          if(selector){
             { svelteHTML.createElement("div", { "class":`sec`,});
               { svelteHTML.createElement("div", { "class":`sec-title`,});  }
               { svelteHTML.createElement("div", { "class":`labels`,});
                   for(let kv of __sveltets_2_ensureArray((selector.split(',')))){kv; { svelteHTML.createElement("span", {   "class":`chip mono`,"title":kv,});kv; }}
               }
             }
          }
          if(Object.keys(row.labels ?? {}).length){
             { svelteHTML.createElement("div", { "class":`sec`,});
               { svelteHTML.createElement("div", { "class":`sec-title`,});  }
               { svelteHTML.createElement("div", { "class":`labels`,});
                   for(let [k, v] of __sveltets_2_ensureArray(Object.entries(row.labels))){k; { svelteHTML.createElement("span", {   "class":`chip mono`,"title":`${k}=${v}`,});k; v; }}
               }
             }
          }
        } else if (detailLoading){
           { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"height":22,}});}
        } else if (detailError){
           { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          "what":`this object`,"variant":`compact`,"error":detailError,"empty":true,"onretry":retry,}});}
        }else{
           { svelteHTML.createElement("div", { "class":`dim`,});                         }
        }
       }
    } else if (tab === 'manifest'){
      if(detailLoading){ { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"height":16,}});} }
      } else if (detailError){ { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          "what":`this object`,"variant":`compact`,"error":detailError,"empty":true,"onretry":retry,}});} }
      }else{
         { svelteHTML.createElement("div", { "class":`code-tools`,});
           { svelteHTML.createElement("span", { "class":`dim`,});kind === 'secrets' ? 'Secret values are redacted by the daemon.' : 'managedFields stripped.';clippedManifest.clipped ? ` ${plural(clippedManifest.clipped, 'long value')} shortened — Copy has the full manifest.` : ''; }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":copyManifest,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  }
         }
         { svelteHTML.createElement("div", { "class":`code`,});
          `${clusterId}/${kind}/${ns}/${name}`; {
              { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {    "path":`manifest.yaml`,"root":"","content":yaml,"readOnly":true,"minimal":true,"wrap":true,}});}
          }
         }
      }
    } else if (tab === 'describe'){
      if(detailLoading){ { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"height":16,}});} }
      } else if (detailError){ { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          "what":`this object`,"variant":`compact`,"error":detailError,"empty":true,"onretry":retry,}});} }
      }else{
         { svelteHTML.createElement("div", { "class":`code-tools`,});
           { svelteHTML.createElement("span", { "class":`dim mono`,});  def.singular.toLowerCase(); name; }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copyOrSay(detail?.describe ?? '', 'describe output'),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  }
         }
         { svelteHTML.createElement("pre", { "class":`describe mono`,});detail?.describe ?? ''; }
      }
    } else if (tab === 'events'){
      if(detailLoading){ { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"height":22,}});} }
      } else if (detailError){ { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          "what":`this object`,"variant":`compact`,"error":detailError,"empty":true,"onretry":retry,}});} }
      } else if (!detail?.events.length){ { svelteHTML.createElement("div", { "class":`dim pad`,});     }
      }else{
         { svelteHTML.createElement("table", { "class":`events`,});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`num`,});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
           { svelteHTML.createElement("tbody", {});
                for(let ev of __sveltets_2_ensureArray(detail.events)){let i = 1;i;
               { svelteHTML.createElement("tr", { });ev.type !== 'Normal';
                 { svelteHTML.createElement("td", {});ev.type; }
                 { svelteHTML.createElement("td", { "class":`mono`,});ev.reason; }
                 { svelteHTML.createElement("td", { "class":`num mono`,});ev.count; }
                 { svelteHTML.createElement("td", { "class":`mono nowrap`,});ev.last_seen; }
                 { svelteHTML.createElement("td", { "class":`msg`,});ev.message; }
               }
            }
           }
         }
      }
    } else if (tab === 'pods'){
       { const $$_sdoPdaolkroW2C = __sveltets_2_ensureComponent(WorkloadPods); new $$_sdoPdaolkroW2C({ target: __sveltets_2_any(), props: {       clusterId,ns,selector,"canEdit":canExec,"onopenpod":(pod, t) => onopenpod?.(ns, pod, t),}});}
    } else if (tab === 'logs' && !isPod && canLogs){
       { const $$_weiVsgoL2C = __sveltets_2_ensureComponent(LogsView); new $$_weiVsgoL2C({ target: __sveltets_2_any(), props: {         clusterId,ns,selector,"title":name,"containers":templateContainers,"onopenpod":(pod) => onopenpod?.(ns, pod, 'logs'),}});}
    } else if (tab === 'logs' && canLogs){
       { const $$_weiVsgoL2C = __sveltets_2_ensureComponent(LogsView); new $$_weiVsgoL2C({ target: __sveltets_2_any(), props: {     clusterId,ns,"pod":name,containers,}});}
    } else if (tab === 'terminal' && canExec){
       { const $$_weiVcexE2C = __sveltets_2_ensureComponent(ExecView); new $$_weiVcexE2C({ target: __sveltets_2_any(), props: {         clusterId,ns,"pod":name,containers,"autoOpen":autoExec,"onautoopened":onautoexec,}});}
    } else if (tab === 'metrics' && canOperation('metrics')){
       { const $$_weiVscirteM2C = __sveltets_2_ensureComponent(MetricsView); new $$_weiVscirteM2C({ target: __sveltets_2_any(), props: {      clusterId,ns,"pod":name,"workload":httpWorkload?.name ?? '',}});}
    } else if (tab === 'http' && canHttp){
      if(detailLoading && !detail){ { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"height":22,}});} }
      }else{
         { const $$_lenaPpttHdoP2C = __sveltets_2_ensureComponent(PodHttpPanel); new $$_lenaPpttHdoP2C({ target: __sveltets_2_any(), props: {          clusterId,ns,"pod":isPod ? name : undefined,"workload":httpWorkload,"manifest":detail?.manifest ?? null,"canMutate":canEdit && canExec,}});}
      }
    }
   }
 DockedDrawer}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ResourceDrawer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ResourceDrawer__SvelteComponent_ = ReturnType<typeof ResourceDrawer__SvelteComponent_>;
/*Ωignore_endΩ*/export default ResourceDrawer__SvelteComponent_;