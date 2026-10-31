///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { formatBytes } from '../../lib/metric-format';
import { pluralNoun } from '../../lib/plural';
import { api } from '../../lib/api/client';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible } from '../../lib/poll';
import { quietCadence } from '../../lib/pollBackoff';
import type { ClusterMetrics, ClusterOverview, Id } from '../../lib/api/types';
;

  interface Props {
    clusterId: Id;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { clusterId }: Props = $props();

  let overview = $state<ClusterOverview | null>(null);
  let metrics = $state<ClusterMetrics | null>(null);
  let loading = $state(true);
  /** Failed overview load — inline with Retry (it used to toast and leave a blank pane). */
  let error = $state<string | null>(null);
  /** The first load has taken long enough to be worth explaining. */
  let slow = $state(false);
  /** Bumped by Retry to re-run the load effect. */
  let attempt = $state(0);
  /** Metrics cadence: 4 s while traffic moves, up to 10 s on an idle cluster. */
  const METRICS_MIN_MS = 4000;
  const METRICS_MAX_MS = 10_000;

  function fmtNum(n: number): string {
    return n.toLocaleString();
  }

  $effect(() => {
    const id = clusterId;
    void attempt;
    loading = true;
    error = null;
    overview = null;
    metrics = null;
    let alive = true;

    void api
      .get<ClusterOverview>(`/brokers/clusters/${id}/overview`)
      .then((o) => {
        if (alive) overview = o;
      })
      .catch((e) => {
        if (alive) error = loadErrorText(e);
      })
      .finally(() => {
        if (alive) loading = false;
      });

    // Chained (never overlapping), paused while hidden, aborted on leave: the
    // metrics sweep is a cluster-wide ListOffsets pass that can outlast a 4 s
    // interval over a tunnel — an interval stacked sweeps and pinned sockets.
    // An idle cluster (total unchanged for 3 samples) stretches the cadence
    // to 10 s — each sample is a watermark sweep plus a Prometheus scrape.
    const cadence = quietCadence({ min: METRICS_MIN_MS, max: METRICS_MAX_MS, quietAfter: 3 });
    let lastTotal: number | null = null;
    const poller = pollWhileVisible(
      (signal) =>
        api
          .get<ClusterMetrics>(`/brokers/clusters/${id}/metrics`, signal)
          .then((m) => {
            if (!alive) return;
            metrics = m;
            cadence.sample(lastTotal !== m.total_messages);
            lastTotal = m.total_messages;
          }),
      {
        get ms() {
          return cadence.ms;
        },
      },
    );
    // An unreachable broker can hang the first metadata call for a long time;
    // say so instead of an open-ended "Connecting…".
    slow = false;
    const slowTimer = setTimeout(() => { if (alive) slow = true; }, 8000);

    return () => {
      alive = false;
      poller.stop();
      clearTimeout(slowTimer);
    };
  });

  const peakRate = $derived(
    Math.max(1, ...(metrics?.throughput.map((p) => p.messages_per_sec) ?? [1])),
  );
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (loading && !overview) announce('Connecting to the cluster…'); });
  $effect(() => { if (slow && loading && !overview) announce('Still waiting on the brokers. Check the bootstrap servers and your network, or try again.'); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`overview`,});
  if(loading && !overview){
     { svelteHTML.createElement("div", { "class":`connecting`,});
       { svelteHTML.createElement("p", { "class":`muted`,});    }
      if(slow){
         { svelteHTML.createElement("p", { "class":`muted slow-note`,});
                      
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => attempt++,});  }
         }
      }
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":2,"height":64,}});}
     }
  } else if (error && !overview){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`the cluster overview`,loading,error,"empty":true,"onretry":() => attempt++,}});}
  } else if (overview){
     { svelteHTML.createElement("div", { "class":`cards`,});
       { svelteHTML.createElement("div", { "class":`card`,});
         { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});overview.brokers.length; }
       }
       { svelteHTML.createElement("div", { "class":`card`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("span", { "class":`v`,});overview.topic_count - overview.internal_topic_count; }
         { svelteHTML.createElement("span", { "class":`sub`,}); overview.internal_topic_count;  }
       }
       { svelteHTML.createElement("div", { "class":`card`,});
         { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});fmtNum(overview.partition_count); }
       }
       { svelteHTML.createElement("div", { "class":`card`,});
         { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`v`,});overview.consumer_group_count; }
       }
       { svelteHTML.createElement("div", { "class":`card`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("span", { "class":`v`,});Math.round(metrics?.messages_per_sec ?? 0); { svelteHTML.createElement("small", {});  } }
         { svelteHTML.createElement("span", { "class":`sub`,});fmtNum(metrics?.total_messages ?? 0);  }
       }
      if(overview.under_replicated_partitions != null){
         { svelteHTML.createElement("div", {  "class":`card`,});overview.under_replicated_partitions > 0;
           { svelteHTML.createElement("span", { "class":`k`,});  }
           { svelteHTML.createElement("span", {  "class":`v`,});overview.under_replicated_partitions > 0;
            overview.under_replicated_partitions;
           }
           { svelteHTML.createElement("span", { "class":`sub`,});pluralNoun(overview.under_replicated_partitions, 'partition'); }
         }
      }
      if(overview.leadership_imbalance != null){
         { svelteHTML.createElement("div", {  "class":`card`,});overview.leadership_imbalance > 0.2;
           { svelteHTML.createElement("span", { "class":`k`,});  }
           { svelteHTML.createElement("span", {  "class":`v`,});overview.leadership_imbalance > 0.2;
            (overview.leadership_imbalance * 100).toFixed(0);
           }
           { svelteHTML.createElement("span", { "class":`sub`,});    }
         }
      }
      if(overview.cluster_id){
         { svelteHTML.createElement("div", { "class":`card wide`,});
           { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`mono`,});overview.cluster_id; }
         }
      }
     }

     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("h4", {});  }
      if(metrics && metrics.throughput.length > 1){
         { svelteHTML.createElement("div", { "class":`spark`,});
             for(let p of __sveltets_2_ensureArray(metrics.throughput.slice(-60))){p.ts_ms;
             { svelteHTML.createElement("div", {       "class":`bar`,"style":`height: ${Math.max(2, (p.messages_per_sec / peakRate) * 100)}%`,"title":`${Math.round(p.messages_per_sec)}/s`,}); }
          }
         }
      }else{
         { svelteHTML.createElement("p", { "class":`muted small`,});        }
      }
     }

     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("h4", {});  }
       { svelteHTML.createElement("div", { "class":`brokers`,});
           for(let b of __sveltets_2_ensureArray(overview.brokers)){b.id;
          const bm = metrics?.brokers.find(
            (m) => m.instance === String(b.id) || m.instance === b.host,
          ) ?? metrics?.brokers[0];
           { svelteHTML.createElement("div", { "class":`broker`,});
             { svelteHTML.createElement("div", { "class":`broker-head`,});
               { svelteHTML.createElement("span", { "class":`mono`,}); b.id; }
               { svelteHTML.createElement("span", { "class":`host`,});b.host; b.port; }
               { svelteHTML.createElement("span", { "class":`leaders`,});b.partition_leaders;  }
             }
            if(metrics?.prometheus_available && bm){
               { svelteHTML.createElement("div", { "class":`metric`,});
                 { svelteHTML.createElement("span", { "class":`ml`,});  }
                 { svelteHTML.createElement("div", { "class":`track`,});
                   { svelteHTML.createElement("div", {     "class":`fill cpu`,"style":`width: ${Math.min(100, bm.cpu_percent ?? 0)}%`,}); }
                 }
                 { svelteHTML.createElement("span", { "class":`mv`,});bm.cpu_percent !== null ? `${bm.cpu_percent.toFixed(0)}%` : '—'; }
               }
               { svelteHTML.createElement("div", { "class":`metric`,});
                 { svelteHTML.createElement("span", { "class":`ml`,});  }
                 { svelteHTML.createElement("div", { "class":`track`,});
                   { svelteHTML.createElement("div", {     "class":`fill ram`,"style":`width: ${bm.memory_total_bytes && bm.memory_used_bytes
                      ? Math.min(100, (bm.memory_used_bytes / bm.memory_total_bytes) * 100)
                      : 0}%`,}); }
                 }
                 { svelteHTML.createElement("span", { "class":`mv`,});
                  formatBytes(bm.memory_used_bytes);bm.memory_total_bytes
                    ? ` / ${formatBytes(bm.memory_total_bytes)}`
                    : '';
                 }
               }
            }
           }
        }
       }
      if(metrics && !metrics.prometheus_available){
         { svelteHTML.createElement("p", { "class":`muted small`,});
              { svelteHTML.createElement("em", {});  }       { svelteHTML.createElement("code", { });  }           
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const OverviewTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OverviewTab__SvelteComponent_ = ReturnType<typeof OverviewTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default OverviewTab__SvelteComponent_;