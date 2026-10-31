///<reference types="svelte" />
;
import { radioKey } from '../../lib/radioKey';
import { pollWhileVisible } from '../../lib/poll';
import { awsErrorText } from './util';
import { awsLoadText } from './loadText';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
// CloudWatch metrics for ONE resource (an SQS queue, EC2 instance or RDS
// instance): range picker (1h … 30d), refresh + auto-refresh every 60 s
// while mounted, and a grid of MetricChart cards laid out per
// `metrics-groups.ts`, each with a current / min / max / sum-or-avg stat
// row. Errors follow the module's pattern: AccessDenied → "needs
// cloudwatch:GetMetricData"; expired credentials → Sign in.
import { untrack } from 'svelte';
import { awsApi, isLoginRequired } from '../../lib/api/aws';
import { ApiError, isAbortError } from '../../lib/api/client';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import MetricChart from '../../lib/components/MetricChart.svelte';
import { formatMetric } from '../../lib/metric-format';
import { buildCards } from './metrics-groups';
import type { MetricsNamespace, MetricsRange, MetricsResp } from '../../lib/api/types';
;

  interface Props {
    accountId: string;
    namespace: MetricsNamespace;
    /** Queue name / instance id / DB identifier. */
    dimValue: string;
    region?: string;
    /** EC2 only — lets the daemon skip credit metrics on non-burstable types. */
    instanceType?: string | null;
    onsignin?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { accountId, namespace, dimValue, region, instanceType = null, onsignin }: Props = $props();

  const RANGES: { id: MetricsRange; label: string }[] = [
    { id: '1h', label: '1h' },
    { id: '6h', label: '6h' },
    { id: '24h', label: '24h' },
    { id: '7d', label: '7d' },
    { id: '30d', label: '30d' },
  ];
  const AUTO_MS = 60_000;

  let range = $state<MetricsRange>('1h');
  let resp = $state<MetricsResp | null>(null);
  let loading = $state(false);
  let error = $state('');
  let denied = $state(false);
  let updatedAt = $state<number | null>(null);
  let current: AbortController | null = null;

  async function load(): Promise<void> {
    await resourceAccess.load('aws_account', accountId);
    if (!resourceAccess.can('aws_account', accountId, 'metrics', 'aws', 'view')) {
      resp = null; denied = true; error = 'Metrics access is not granted for this account.'; return;
    }
    current?.abort();
    const ac = new AbortController();
    current = ac;
    loading = true;
    try {
      const r = await awsApi.metrics(accountId, namespace, dimValue, range, {
        region,
        instanceType,
        signal: ac.signal,
      });
      if (ac.signal.aborted) return;
      resp = r;
      error = '';
      denied = false;
      updatedAt = Date.now();
    } catch (e) {
      if (ac.signal.aborted || isAbortError(e)) return;
      denied = e instanceof ApiError && e.status === 403;
      error = awsLoadText(e);
    } finally {
      if (!ac.signal.aborted) loading = false;
    }
  }

  // Reload when the target or range changes; auto-refresh every 60 s while
  // mounted (the panel only exists while its tab is visible).
  $effect(() => {
    void accountId;
    void namespace;
    void dimValue;
    void region;
    void instanceType;
    void range;
    untrack(() => {
      resp = null;
      void load();
    });
    const t = pollWhileVisible(() => untrack(() => load()), { ms: AUTO_MS, immediate: false });
    return () => {
      t.stop();
      current?.abort();
    };
  });

  const cards = $derived(resp ? buildCards(resp) : []);
  const loginNeeded = $derived(isLoginRequired(new Error(error)));
  const periodLabel = $derived.by(() => {
    const p = resp?.period_seconds ?? 0;
    if (!p) return '';
    return p >= 3600 ? `${p / 3600} h` : p >= 60 ? `${p / 60} min` : `${p} s`;
  });

  function fmtUpdated(t: number | null): string {
    if (!t) return '';
    const d = new Date(t);
    const p = (n: number) => String(n).padStart(2, '0');
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`mp`,...__sveltets_2_empty({"data-testid":`aws-metrics`}),});
   { svelteHTML.createElement("div", { "class":`mp-bar`,});
     { svelteHTML.createElement("div", {     "class":`ranges`,"role":`radiogroup`,"aria-label":`Time range`,});
         for(let r of __sveltets_2_ensureArray(RANGES)){r.id;
         { svelteHTML.createElement("button", {            "role":`radio`,"onkeydown":radioKey,"tabindex":range === r.id ? 0 : -1,"aria-checked":range === r.id,"onclick":() => (range = r.id),});range === r.id;r.label; }
      }
     }
     { svelteHTML.createElement("span", { "class":`meta dim`,});
      if(resp){ periodLabel;}
      if(updatedAt){' ';  fmtUpdated(updatedAt);}
         
     }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => void load(),"disabled":loading,"title":`Refresh metrics`,"aria-label":`Refresh metrics`,});
      if(loading){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 13px`,"aria-hidden":`true`,}); }}else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}}
     }
   }

  if(error && !resp){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`metrics`,"error":denied ? `CloudWatch access denied: this account’s IAM identity needs cloudwatch:GetMetricData. ${error}` : awsErrorText(error),"empty":true,"onretry":() => void load(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(!denied && loginNeeded && onsignin){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/ LoadState}
  } else if (!resp){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {      "rows":4,"height":120,"label":`metrics`,}});} }
  }else{
    if(error){
       { svelteHTML.createElement("p", { "class":`stale`,});        error; }
    }
     { svelteHTML.createElement("div", { "class":`grid`,});
         for(let c of __sveltets_2_ensureArray(cards)){c.group.id;
         { svelteHTML.createElement("section", { "class":`card`,});
           { svelteHTML.createElement("h4", {});c.group.title; }
           { const $$_trahCcirteM3C = __sveltets_2_ensureComponent(MetricChart); new $$_trahCcirteM3C({ target: __sveltets_2_any(), props: {      "series":c.chart,"unit":c.series[0].unit,"height":150,}});}
           { svelteHTML.createElement("table", { "class":`stats`,});
             { svelteHTML.createElement("thead", {});
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("th", {}); } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});c.series[0].stat === 'Sum' ? 'Sum' : 'Avg'; }
               }
             }
             { svelteHTML.createElement("tbody", {});
                 for(let s of __sveltets_2_ensureArray(c.series)){s.id;
                 { svelteHTML.createElement("tr", {});
                   { svelteHTML.createElement("td", {   "class":`lbl`,"title":`${s.metric} (${s.stat})`,});s.label; }
                   { svelteHTML.createElement("td", { "class":`mono`,});formatMetric(s.current, s.unit); }
                   { svelteHTML.createElement("td", { "class":`mono`,});formatMetric(s.min, s.unit); }
                   { svelteHTML.createElement("td", { "class":`mono`,});formatMetric(s.max, s.unit); }
                   { svelteHTML.createElement("td", { "class":`mono`,});formatMetric(s.stat === 'Sum' ? s.sum : s.avg, s.unit); }
                 }
              }
             }
           }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MetricsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MetricsPanel__SvelteComponent_ = ReturnType<typeof MetricsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default MetricsPanel__SvelteComponent_;