///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { api } from '../../lib/api/client';
import { toastError } from '../../lib/toastError';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible } from '../../lib/poll';
import TopicDetail from './TopicDetail.svelte';
import { brokersTopicsPort } from '../../lib/uiCommands/brokers';
import type {
    BrokerCluster,
    CreateTopicReq,
    TopicStats,
    TopicSummary,
  } from '../../lib/api/types';
;

  interface Props {
    cluster: BrokerCluster;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { cluster }: Props = $props();

  const PAGE_SIZE = 50;
  const STATS_CONCURRENCY = 4;

  let topics = $state<TopicSummary[]>([]);
  // Lazily-filled per-topic stats (count + cleanup policy), keyed by name.
  // null = in-flight, 'err' = failed (retry available), TopicStats = loaded.
  let stats = $state<Record<string, TopicStats | null | 'err'>>({});
  let loading = $state(true);
  /** Failed topic-list load — inline with Retry, never "No topics.". */
  let loadError = $state<string | null>(null);
  let query = $state('');
  let showInternal = $state(false);
  let cleanupFilter = $state('');
  let page = $state(1);
  let selected = $state<string | null>(null);
  // Agent UI control (lib/uiCommands/brokers.ts) opens a topic here.
  $effect(() => brokersTopicsPort.bind({ clusterId: cluster.id, open: (name) => (selected = name) }));

  let creating = $state(false);
  let newName = $state('');
  let newParts = $state(1);
  let newRf = $state(1);
  let createBusy = $state(false);

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  const filtered = $derived(
    topics
      .filter((t) => showInternal || !t.internal)
      .filter((t) => t.name.toLowerCase().includes(query.toLowerCase()))
      .filter((t) => {
        if (!cleanupFilter) return true;
        const s = stats[t.name];
        return s != null && s !== 'err' && s.cleanup_policy === cleanupFilter;
      })
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const pageCount = $derived(Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));
  const pageStart = $derived((Math.min(page, pageCount) - 1) * PAGE_SIZE);
  const visible = $derived(filtered.slice(pageStart, pageStart + PAGE_SIZE));
  // Cleanup-policy values discovered so far (for the filter dropdown).
  const cleanupOptions = $derived(
    [...new Set(
      Object.values(stats)
        .filter((s): s is TopicStats => s != null && s !== 'err')
        .map((s) => s.cleanup_policy)
        .filter(Boolean)
    )] as string[],
  );

  function load() {
    loading = true;
    stats = {};
    api
      .get<TopicSummary[]>(`/brokers/clusters/${cluster.id}/topics`)
      .then((t) => {
        topics = t;
        loadError = null;
        if (selected && !t.some((x) => x.name === selected)) selected = null;
      })
      .catch((e) => (loadError = loadErrorText(e)))
      .finally(() => (loading = false));
  }

  // Background-fill stats for the topics currently on screen via the batch
  // endpoint (one HTTP call vs N×1). Bounded by STATS_CONCURRENCY batches of
  // PAGE_SIZE so very large topic lists don't hammer the server with huge
  // request bodies.
  let statsToken = 0;
  async function fillStats(names: string[], retry = false) {
    // Skip already-loaded (TopicStats) and in-flight (null) entries unless retrying errors.
    const pending = names.filter((n) => {
      const v = stats[n];
      if (v === undefined) return true;
      if (retry && v === 'err') return true;
      return false;
    });
    if (pending.length === 0) return;
    const token = ++statsToken;
    // Mark as in-flight (null) so parallel calls don't duplicate fetches.
    stats = { ...stats, ...Object.fromEntries(pending.map((n) => [n, null as null])) };
    try {
      // Single batch call to replace per-topic N×1 round-trips.
      const result = await api.post<Record<string, TopicStats>>(
        `/brokers/clusters/${cluster.id}/topics/stats`,
        { names: pending },
      );
      if (token !== statsToken) return;
      const update: Record<string, TopicStats | 'err'> = {};
      for (const name of pending) {
        update[name] = result[name] ?? ('err' as const);
      }
      stats = { ...stats, ...update };
    } catch {
      // Batch endpoint unavailable (older server); fall back to per-topic calls.
      const queue = [...pending];
      const worker = async () => {
        while (queue.length) {
          const name = queue.shift()!;
          try {
            const s = await api.get<TopicStats>(
              `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(name)}/stats`,
            );
            if (token === statsToken) stats = { ...stats, [name]: s };
          } catch {
            if (token === statsToken) stats = { ...stats, [name]: 'err' as const };
          }
        }
      };
      await Promise.all(Array.from({ length: STATS_CONCURRENCY }, worker));
    }
  }

  function retryStats() {
    void fillStats(visible.map((t) => t.name), true);
  }

  $effect(() => {
    void cluster.id;
    selected = null;
    page = 1;
    // Another cluster's topics are not "stale data" for this one.
    topics = [];
    loadError = null;
    load();
  });

  // Lazily load stats for the visible page (and refetch as you paginate/filter).
  $effect(() => {
    if (loading || selected) return;
    void fillStats(visible.map((t) => t.name));
  });

  // Reset to page 1 when the filters change.
  $effect(() => {
    void query;
    void showInternal;
    void cleanupFilter;
    page = 1;
  });

  function countText(name: string): string {
    const s = stats[name];
    if (s === undefined) return '';
    if (s === null) return '…';
    if (s === 'err') return '—';
    return s.message_count.toLocaleString();
  }

  // Per-topic production rate (msg/s), derived server-side from the high-watermark
  // delta between consecutive `topics/stats` polls. `—` until a second sample
  // lands (or when the count is unavailable).
  function rateText(name: string): string {
    const s = stats[name];
    if (s == null || s === 'err' || s.msg_per_sec == null) return '—';
    const r = s.msg_per_sec;
    if (r < 0.05) return '0';
    if (r < 10) return `${r.toFixed(1)}/s`;
    return `${Math.round(r).toLocaleString()}/s`;
  }

  // Periodically re-fetch the visible topics' stats so the msg/s rate refreshes
  // (the backend needs two samples to compute it). One batch call per tick.
  async function refreshRates(signal?: AbortSignal) {
    if (loading || selected) return;
    const names = visible.map((t) => t.name).filter((n) => {
      const v = stats[n];
      return v != null && v !== 'err';
    });
    if (names.length === 0) return;
    try {
      const result = await api.post<Record<string, TopicStats>>(
        `/brokers/clusters/${cluster.id}/topics/stats`,
        { names },
        signal,
      );
      if (!signal?.aborted) stats = { ...stats, ...result };
    } catch {
      // Transient — keep the last values; the next tick retries.
    }
  }

  // Chained (a slow tunnel never stacks ticks), paused while hidden, aborted
  // on leave. 10 s: each tick is a stats fan-out over every visible topic, and
  // a rate column doesn't need sub-10 s freshness.
  $effect(() => {
    void cluster.id;
    const poller = pollWhileVisible((signal) => refreshRates(signal), {
      ms: 10_000,
      immediate: false,
    });
    return () => poller.stop();
  });

  // True when any visible topic's stats failed — show the retry button.
  const hasStatErrors = $derived(visible.some((t) => stats[t.name] === 'err'));

  async function createTopic() {
    if (!newName.trim()) return;
    // Same guard rail as produce / config edits in TopicDetail: a write to a
    // guarded (prod or read-only) cluster asks first instead of silently
    // sending `confirm: true`.
    if (guarded) {
      const ok = await confirmer.ask(
        `Create topic "${newName.trim()}" on guarded cluster "${cluster.name}"?`,
        { title: 'Create topic on guarded cluster', confirmLabel: 'Create', danger: true },
      );
      if (!ok) return;
    }
    const req: CreateTopicReq = {
      name: newName.trim(),
      partitions: Number(newParts),
      replication_factor: Number(newRf),
      confirm: guarded,
    };
    createBusy = true;
    try {
      await api.post(`/brokers/clusters/${cluster.id}/topics`, req);
      toasts.success(`Created ${req.name}`);
      creating = false;
      newName = '';
      load();
      selected = req.name;
    } catch (e) {
      toastError('Couldn’t create the topic', e);
    } finally {
      createBusy = false;
    }
  }
;
async () => {

if(selected){
   { svelteHTML.createElement("div", { "class":`detail-wrap`,});
     { svelteHTML.createElement("button", {   "class":`crumb`,"onclick":() => (selected = null),});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":13,}});} 
       { svelteHTML.createElement("span", { "class":`sep`,});  }
       { svelteHTML.createElement("span", {   "class":`cur`,"title":selected,});selected; }
     }
    selected; {
       { const $$_liateDcipoT1C = __sveltets_2_ensureComponent(TopicDetail); new $$_liateDcipoT1C({ target: __sveltets_2_any(), props: {      cluster,"topic":selected,"ondeleted":() => {
          selected = null;
          load();
        },}});}
    }
   }
}else{
   { svelteHTML.createElement("div", { "class":`topics`,});
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`search`,"bind:value":query,"placeholder":`Filter topics…`,"aria-label":`Filter topics`,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showInternal,});/*Ωignore_startΩ*/() => showInternal = __sveltets_2_any(null);/*Ωignore_endΩ*/}   }
      if(cleanupOptions.length){
         { svelteHTML.createElement("select", {   "bind:value":cleanupFilter,"title":`Cleanup policy`,});/*Ωignore_startΩ*/() => cleanupFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});   }
             for(let p of __sveltets_2_ensureArray(cleanupOptions)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
         }
      }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("span", { "class":`count`,});plural(filtered.length, 'topic'); }
      if(hasStatErrors){
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":retryStats,"title":`Retry failed message-count fetches`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}  
         }
      }
       { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":load,"disabled":loading,"aria-label":`Refresh topics`,"title":`Refresh topics`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
       }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => (creating = !creating),"title":`New topic`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});} 
       }
     }

    if(creating){
       { svelteHTML.createElement("div", { "class":`create`,});
         { svelteHTML.createElement("input", {        "dir":`ltr`,"bind:value":newName,"placeholder":`orders.events`,"aria-label":`Topic name`,});/*Ωignore_startΩ*/() => newName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("label", { "title":`Partitions`,});  { svelteHTML.createElement("input", {      "type":`number`,"min":`1`,"bind:value":newParts,});/*Ωignore_startΩ*/() => newParts = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", { "title":`Replication factor`,});  { svelteHTML.createElement("input", {      "type":`number`,"min":`1`,"bind:value":newRf,});/*Ωignore_startΩ*/() => newRf = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("button", {         "class":`btn primary small`,"onclick":createTopic,"disabled":!newName.trim() || createBusy,"title":newName.trim() ? 'Create topic' : 'Enter a topic name first',});createBusy ? 'Creating…' : 'Create'; }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => (creating = false),"disabled":createBusy,});  }
       }
    }

     { svelteHTML.createElement("div", { "class":`grid-wrap`,});
      
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`topics`,loading,"error":loadError,"empty":topics.length === 0,"onretry":load,"rows":6,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {            "icon":`layers`,"title":`No topics on this cluster yet`,"actionLabel":`New topic…`,"actionIcon":`plus`,"actionKind":`secondary`,"onaction":() => (creating = true),}});}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
        
      if(filtered.length === 0){
        
         { svelteHTML.createElement("p", { "class":`muted pad`,});
               
           { svelteHTML.createElement("button", {    "class":`btn small ghost`,"onclick":() => {
              query = '';
              cleanupFilter = '';
              showInternal = true;
            },});  }
         }
      }else{
         { svelteHTML.createElement("table", { "class":`grid`,});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("th", { "class":`tname`,});  }
               { svelteHTML.createElement("th", { "class":`num`,});  }
               { svelteHTML.createElement("th", { "class":`num`,});  }
               { svelteHTML.createElement("th", { "class":`num`,});  }
               { svelteHTML.createElement("th", {   "class":`num`,"title":`Production rate (messages/second), from the high-watermark delta between polls`,});  }
               { svelteHTML.createElement("th", { "class":`num`,});  }
             }
           }
           { svelteHTML.createElement("tbody", {});
               for(let t of __sveltets_2_ensureArray(visible)){t.name;
               { svelteHTML.createElement("tr", {         "tabindex":0,"aria-label":`Open topic ${t.name}`,"onclick":() => (selected = t.name),"onkeydown":(e) => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); selected = t.name; } },});
                 { svelteHTML.createElement("td", {    "class":`tname`,"title":t.internal ? `${t.name} (internal)` : undefined,});t.internal;t.name; }
                 { svelteHTML.createElement("td", { "class":`num`,});t.partitions; }
                 { svelteHTML.createElement("td", { "class":`num`,});t.replication_factor; }
                 { svelteHTML.createElement("td", {      "class":`num`,"title":stats[t.name] === 'err' ? 'Count unavailable — click "Retry counts" to try again' : undefined,});stats[t.name] === 'err';countText(t.name); }
                 { svelteHTML.createElement("td", {   "class":`num`,"title":`Production rate (msg/s) — high-watermark delta between polls`,});rateText(t.name); }
                 { svelteHTML.createElement("td", {   "class":`num muted`,"title":`On-disk size isn’t exposed by this Kafka client`,});  }
               }
            }
           }
         }
      }
       LoadState}
     }

    if(topics.length > 0 && pageCount > 1){
       { svelteHTML.createElement("div", { "class":`pager`,});
         { svelteHTML.createElement("span", {  "class":`muted`,});pageStart + 1; Math.min(pageStart + PAGE_SIZE, filtered.length);  filtered.length; }
         { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":page <= 1,"onclick":() => (page = Math.max(1, page - 1)),"aria-label":`Previous page`,"title":`Previous page`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});}
         }
         { svelteHTML.createElement("span", { "class":`muted`,});Math.min(page, pageCount);  pageCount; }
         { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":page >= pageCount,"onclick":() => (page = Math.min(pageCount, page + 1)),"aria-label":`Next page`,"title":`Next page`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
         }
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TopicsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TopicsTab__SvelteComponent_ = ReturnType<typeof TopicsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default TopicsTab__SvelteComponent_;