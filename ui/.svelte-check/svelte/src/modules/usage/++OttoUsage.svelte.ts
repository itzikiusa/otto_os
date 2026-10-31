///<reference types="svelte" />
;
import { pollWhileVisible } from '../../lib/poll';
import { onMount } from 'svelte';
import { api } from '../../lib/api/client';
import type { TelemetryConfig, TelemetryOverview, TelemetryStatus, TelemetrySuggestion, TelemetrySpan, TelemetryProfile } from '../../lib/api/types';
import { telemetrySettingsChanged } from '../../lib/telemetryBoot';
import LoadState from '../../lib/components/LoadState.svelte';
import Modal from '../../lib/components/Modal.svelte';
import MetricChart from '../../lib/components/MetricChart.svelte';
import { formatCount as fmt, formatBytes, formatSeconds, type MetricChartPoint } from '../../lib/metric-format';
import { loadErrorText } from '../../lib/loadError';
import { announce } from '../../lib/announce.svelte';

;type $$ComponentProps =  { dirty?: boolean };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  let { dirty = $bindable(false) }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props()/*Ωignore_startΩ*/;dirty;/*Ωignore_endΩ*/;
  let savedConfig = $state('');
  $effect(() => { dirty = !!config && JSON.stringify(config) !== savedConfig; });

  let config = $state<TelemetryConfig | null>(null);
  let status = $state<TelemetryStatus | null>(null);
  let overview = $state<TelemetryOverview | null>(null);
  let suggestions = $state<TelemetrySuggestion[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let loadError = $state('');
  let message = $state('');
  // a11y-6-01: the save result and errors are announced where they change;
  // the visible lines are inserted with their text, so they are not live regions.
  $effect(() => { if (message) announce(message); });
  $effect(() => { if (error) announce(error, { assertive: true }); });
  let trace = $state<TelemetrySpan[] | null>(null);
  let traceBusy = $state(false);
  let traceError = $state('');
  let traceId = $state<string | null>(null);
  let profile = $state<TelemetryProfile | null>(null);
  let hours = $state(24);
  let operationPage = $state(0);
  const operationPages = $derived(Math.max(1, Math.ceil((overview?.operations.length ?? 0) / 50)));
  const operations = $derived((overview?.operations ?? []).slice(Math.min(operationPage, operationPages - 1) * 50, (Math.min(operationPage, operationPages - 1) + 1) * 50));
  function scrollFocus(node: HTMLElement) {
    const update = () => { if (node.scrollWidth > node.clientWidth + 1) node.tabIndex = 0; else node.removeAttribute('tabindex'); };
    const observer = new ResizeObserver(update);
    observer.observe(node);
    if (node.firstElementChild) observer.observe(node.firstElementChild);
    return { destroy: () => observer.disconnect() };
  }
  let disposed = false;
  let loadSeq = 0;
  let detailSeq = 0;
  const controller = new AbortController();
  const date = (n: number | null | undefined) => n ? new Date(n * 1000).toLocaleString() : 'Not yet';
  const active = $derived(suggestions.filter((s) => !s.dismissed));
  const resources = $derived.by(() => {
    const groups = new Map<string, { cpu: number | null; rss: number | null; peakCpu: number | null; peakRss: number | null; load: number | null; points: MetricChartPoint[]; memory: MetricChartPoint[] }>();
    for (const sample of overview?.resources ?? []) {
      const group = groups.get(sample.process) ?? { cpu: null, rss: null, peakCpu: null, peakRss: null, load: null, points: [], memory: [] };
      if (sample.cpu_percent !== null) { group.cpu = sample.cpu_percent; group.peakCpu = Math.max(group.peakCpu ?? 0, sample.cpu_percent);  }
      if (sample.rss_mb !== null) { group.rss = sample.rss_mb; group.peakRss = Math.max(group.peakRss ?? 0, sample.rss_mb); }
      if (sample.host_load !== null) group.load = sample.host_load;
      group.points.push({ t: sample.timestamp * 1000, v: sample.cpu_percent });
      group.memory.push({ t: sample.timestamp * 1000, v: sample.rss_mb === null ? null : sample.rss_mb * 1048576 });
      groups.set(sample.process, group);
    }
    return Array.from(groups, ([name, data]) => ({ name, ...data }));
  });
  async function load(initial = false): Promise<void> {
    const seq = ++loadSeq;
    if (initial) loading = true;
    loadError = '';
    try {
      const [cfg, state] = await Promise.all([
        api.get<TelemetryConfig>('/telemetry/config', controller.signal),
        api.get<TelemetryStatus>('/telemetry/status', controller.signal),
      ]);
      if (disposed || seq !== loadSeq) return;
      if (!config || initial) { config = cfg; savedConfig = JSON.stringify(cfg); }
      status = state;
      if (state.collector_ready || !state.enabled) {
        const [data, items] = await Promise.all([
          api.get<TelemetryOverview>(`/telemetry/overview?hours=${hours}`, controller.signal),
          api.get<TelemetrySuggestion[]>('/telemetry/suggestions', controller.signal),
        ]);
        if (disposed || seq !== loadSeq) return;
        overview = data; suggestions = items;
      }
    } catch (e) {
      if (!disposed && seq === loadSeq) loadError = loadErrorText(e);
    } finally {
      if (!disposed && seq === loadSeq) {
        loading = false;
      }
    }
  }
  onMount(() => {
    void load(true);
    const poller = pollWhileVisible(() => { if (status?.enabled && !busy && !loading) return load(); }, { ms: 15000, immediate: false });
    return () => { disposed = true; controller.abort(); loadSeq++; detailSeq++; poller.stop(); };
  });
  async function save(): Promise<void> {
    if (!config || busy) return;
    busy = true; error = ''; message = '';
    try {
      config = await api.put<TelemetryConfig>('/telemetry/config', { ...config });
      savedConfig = JSON.stringify(config);
      telemetrySettingsChanged();
      message = config.enabled ? 'Settings saved. The local collector starts automatically.' : 'Collection stopped. Retained history expires on its schedule.';
      await load();
    } catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
  async function analyze(): Promise<void> {
    if (busy) return;
    busy = true; error = '';
    try { suggestions = await api.post<TelemetrySuggestion[]>('/telemetry/analyze'); await load(); }
    catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
  async function dismiss(id: string): Promise<void> {
    if (busy) return;
    busy = true;
    try { await api.post(`/telemetry/suggestions/${encodeURIComponent(id)}/dismiss`); suggestions = suggestions.map((s) => s.id === id ? { ...s, dismissed: true } : s); }
    catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
  async function openTrace(id: string): Promise<void> {
    const seq = ++detailSeq;
    traceId = id; trace = []; traceBusy = true; traceError = '';
    try { const spans = await api.get<TelemetrySpan[]>(`/telemetry/traces/${encodeURIComponent(id)}`, controller.signal); if (!disposed && seq === detailSeq) trace = spans; }
    catch (e) { if (!disposed && seq === detailSeq) traceError = loadErrorText(e); }
    finally { if (!disposed && seq === detailSeq) traceBusy = false; }
  }
  async function capture(): Promise<void> {
    busy = true; error = '';
    try { profile = await api.post<TelemetryProfile>('/telemetry/profile'); }
    catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`otto-usage`,"aria-label":`Otto application usage`,});
   { svelteHTML.createElement("p", { "class":`intro`,});                     }
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`application telemetry`,loading,"empty":!config,"error":loadError,"onretry":() => void load(),}});
    if(config){
       { svelteHTML.createElement("form", {   "class":`settings`,"onsubmit":(event) => { event.preventDefault(); void save(); },});
         { svelteHTML.createElement("label", { "class":`toggle`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":config.enabled,"disabled":busy,});/*Ωignore_startΩ*/() => config.enabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}       }
         { svelteHTML.createElement("p", { "class":`hint`,});                                }
         { svelteHTML.createElement("details", {});
           { svelteHTML.createElement("summary", {});    }
           { svelteHTML.createElement("div", { "class":`fields`,});
             { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`1`,"max":`30`,"step":`1`,"required":true,"bind:value":config.traces_days,"disabled":busy,});/*Ωignore_startΩ*/() => config.traces_days = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`1`,"max":`30`,"step":`1`,"required":true,"bind:value":config.logs_days,"disabled":busy,});/*Ωignore_startΩ*/() => config.logs_days = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`1`,"max":`90`,"step":`1`,"required":true,"bind:value":config.metrics_days,"disabled":busy,});/*Ωignore_startΩ*/() => config.metrics_days = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`1`,"max":`168`,"step":`1`,"required":true,"bind:value":config.analysis_interval_hours,"disabled":busy,});/*Ωignore_startΩ*/() => config.analysis_interval_hours = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`1`,"max":config.traces_days * 24,"step":`1`,"required":true,"bind:value":config.analysis_window_hours,"disabled":busy,});/*Ωignore_startΩ*/() => config.analysis_window_hours = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});     { svelteHTML.createElement("input", {            "class":`input`,"type":`number`,"min":`1`,"max":`60000`,"required":true,"bind:value":config.slow_threshold_ms,"disabled":busy,});/*Ωignore_startΩ*/() => config.slow_threshold_ms = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`1`,"max":`10000`,"step":`1`,"required":true,"bind:value":config.min_samples,"disabled":busy,});/*Ωignore_startΩ*/() => config.min_samples = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`1`,"max":`100`,"step":`1`,"required":true,"bind:value":config.suggestion_limit,"disabled":busy,});/*Ωignore_startΩ*/() => config.suggestion_limit = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {              "class":`input`,"type":`number`,"min":`2`,"max":`60`,"step":`1`,"required":true,"bind:value":config.sample_interval_secs,"disabled":busy,});/*Ωignore_startΩ*/() => config.sample_interval_secs = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});    { svelteHTML.createElement("input", {            "class":`input`,"type":`number`,"max":`10000`,"min":`1`,"required":true,"bind:value":config.cpu_spike_percent,"disabled":busy,});/*Ωignore_startΩ*/() => config.cpu_spike_percent = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
             { svelteHTML.createElement("label", {});     { svelteHTML.createElement("input", {            "class":`input`,"type":`number`,"min":`16`,"max":`65536`,"required":true,"bind:value":config.rss_spike_mb,"disabled":busy,});/*Ωignore_startΩ*/() => config.rss_spike_mb = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
           }
           { svelteHTML.createElement("label", { "class":`toggle`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":config.native_profiling,"disabled":busy || !status?.native_profiling_supported,});/*Ωignore_startΩ*/() => config.native_profiling = __sveltets_2_any(null);/*Ωignore_endΩ*/}        }
           { svelteHTML.createElement("p", { "class":`hint`,});                         }
         }
         { svelteHTML.createElement("button", {     "class":`btn primary`,"type":`submit`,"disabled":busy,});busy ? 'Working…' : 'Save telemetry settings'; }
        if(message){ { svelteHTML.createElement("p", { "class":`saved`,});message; }}
       }
    }
   LoadState}
  if(error){ { svelteHTML.createElement("div", { "class":`notice`,});error;  { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => error = '',});  } }}
  if(status){
     { svelteHTML.createElement("div", { "class":`status`,});
       { svelteHTML.createElement("span", { "class":`chip`,});!status.enabled ? 'Collection off' : status.collector_ready ? 'Collecting locally' : 'Collector starting or unavailable'; }
       { svelteHTML.createElement("span", {});fmt(status.exported);    fmt(status.queued);   fmt(status.dropped);    }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void load(),"disabled":loading || busy,});  }
     }
    if(status.last_error){ { svelteHTML.createElement("p", {   "class":`notice`,"role":`alert`,});                  { svelteHTML.createElement("span", { "class":`dim`,});status.last_error; } }}
  }
  if(overview){
     { svelteHTML.createElement("div", { "class":`section-heading`,}); { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("label", { "class":`toggle`,});  { svelteHTML.createElement("select", {     "class":`input`,"bind:value":hours,"onchange":() => void load(),});/*Ωignore_startΩ*/() => hours = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":1,});  } { svelteHTML.createElement("option", { "value":6,});   } { svelteHTML.createElement("option", { "value":24,});  } { svelteHTML.createElement("option", { "value":168,});  } } } }
    if(resources.length){
       { svelteHTML.createElement("div", { "class":`resource-grid`,});
           for(let resource of __sveltets_2_ensureArray(resources)){resource.name;
           { svelteHTML.createElement("article", { "class":`resource`,}); { svelteHTML.createElement("h3", {});resource.name; }
             { svelteHTML.createElement("div", {});resource.cpu === null ? 'CPU unavailable' : `${fmt(resource.cpu)}% CPU`;  resource.rss === null ? 'Memory unavailable' : `${formatBytes(resource.rss * 1048576)} RAM`; }
            if(resource.load !== null){ { svelteHTML.createElement("p", {});    fmt(resource.load); }}
            if(resource.peakCpu !== null){ { const $$_trahCcirteM3C = __sveltets_2_ensureComponent(MetricChart); new $$_trahCcirteM3C({ target: __sveltets_2_any(), props: {      "series":[{ label: `${resource.name} CPU`, points: resource.points }],"unit":`percent`,"height":120,}});}}
            if(resource.peakRss !== null){ { const $$_trahCcirteM3C = __sveltets_2_ensureComponent(MetricChart); new $$_trahCcirteM3C({ target: __sveltets_2_any(), props: {      "series":[{ label: `${resource.name} RAM`, points: resource.memory }],"unit":`bytes`,"height":120,}});}}
             { svelteHTML.createElement("p", { "class":`hint`,});resource.peakCpu === null ? 'CPU peak unavailable' : `Peak CPU ${fmt(resource.peakCpu)}%`;  resource.peakRss === null ? 'Memory peak unavailable' : `Peak RAM ${formatBytes(resource.peakRss * 1048576)}`; }
           }
        }
       }
    }else{ { svelteHTML.createElement("p", { "class":`hint`,});            }}
     { svelteHTML.createElement("p", { "class":`hint`,});                  }
     { svelteHTML.createElement("div", { "class":`section-heading`,}); { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":analyze,"disabled":busy || !status?.collector_ready,});  } }
     { svelteHTML.createElement("p", { "class":`hint`,});  config?.suggestion_limit ?? 100;                     date(status?.last_analysis_at);  date(status?.next_analysis_at);  }
    if(active.length){
       { svelteHTML.createElement("div", { "class":`suggestions`,});   for(let item of __sveltets_2_ensureArray(active)){item.id;
         { svelteHTML.createElement("article", { "class":`suggestion`,}); { svelteHTML.createElement("h3", {});item.component;  item.name; } { svelteHTML.createElement("p", {});item.action; }
          if(item.kind === 'resource'){ { svelteHTML.createElement("p", { "class":`hint`,});fmt(item.count);    item.peak_cpu_percent === null ? 'unavailable' : `${fmt(item.peak_cpu_percent)}%`;   item.peak_rss_mb === null ? 'unavailable' : formatBytes(item.peak_rss_mb * 1048576);  date(item.observed_at); }}else{ { svelteHTML.createElement("p", { "class":`hint`,});fmt(item.count);    formatSeconds(item.p95_ms / 1000);   formatSeconds(item.max_ms / 1000);  date(item.observed_at); }}
           { svelteHTML.createElement("div", { "class":`actions`,});if(item.trace_id){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void openTrace(item.trace_id),});  }} { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void dismiss(item.id),"disabled":busy,});  } }
         }
      } }
    }else{ { svelteHTML.createElement("p", { "class":`hint`,});                 }}
     { svelteHTML.createElement("h2", {});  }
    if(overview.operations.length){
       {const $$action_0 = __sveltets_2_ensureAction(scrollFocus(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`table-scroll`,"role":`region`,"aria-label":`Operation latency table`,});
         { svelteHTML.createElement("table", {}); { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } } }
           { svelteHTML.createElement("tbody", {});   for(let op of __sveltets_2_ensureArray(operations)){`${op.component}:${op.name}`; { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`row`,}); { svelteHTML.createElement("span", {});op.component; } { svelteHTML.createElement("code", {});op.name; } } { svelteHTML.createElement("td", {});fmt(op.count); } { svelteHTML.createElement("td", {});formatSeconds(op.p50_ms / 1000); } { svelteHTML.createElement("td", {});formatSeconds(op.p95_ms / 1000); } { svelteHTML.createElement("td", {});formatSeconds(op.max_ms / 1000); } { svelteHTML.createElement("td", {});fmt(op.errors); } { svelteHTML.createElement("td", {});if(op.trace_id){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void openTrace(op.trace_id),});  }}else{ } } }} }
         }
       }}
      if(operationPages > 1){ { svelteHTML.createElement("nav", {   "class":`actions`,"aria-label":`Operation pages`,}); { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":operationPage === 0,"onclick":() => operationPage--,});  } { svelteHTML.createElement("span", {}); Math.min(operationPage + 1, operationPages);  operationPages; } { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":operationPage >= operationPages - 1,"onclick":() => operationPage++,});  } }}
    }else{ { svelteHTML.createElement("p", { "class":`hint`,});               }}
  }
   { svelteHTML.createElement("div", { "class":`section-heading`,}); { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":async () => { try { profile = await api.get<TelemetryProfile | null>('/telemetry/profile'); message = profile ? '' : 'No retained native profile yet.'; } catch (e) { error = loadErrorText(e); } },});   } { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":capture,"disabled":busy || !status?.enabled || !config?.native_profiling || !status?.native_profiling_supported,});busy ? 'Working…' : 'Capture short profile'; } }
   { svelteHTML.createElement("p", { "class":`hint`,});                          }
 }

if(trace !== null){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Trace detail`,"width":680,"onclose":() => { detailSeq++; traceId = null; trace = null; },}});
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`trace`,"loading":traceBusy,"empty":!trace.length,"error":traceError,"onretry":() => { if (traceId) void openTrace(traceId); },emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", {});           }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ol", { "class":`trace-list`,});   for(let span of __sveltets_2_ensureArray(trace)){span.span_id; { svelteHTML.createElement("li", {}); { svelteHTML.createElement("code", {});span.name; } { svelteHTML.createElement("span", {});span.component;  formatSeconds(span.duration_ms / 1000);  span.status; } { svelteHTML.createElement("small", {}); span.span_id;span.parent_span_id ? ` · parent ${span.parent_span_id}` : ' · root'; } }} }
     LoadState}
   Modal}
}
if(profile){ { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Native stack profile`,"width":680,"onclose":() => profile = null,}}); { svelteHTML.createElement("p", {});date(profile.captured_at);  profile.duration_seconds;   profile.format; } { svelteHTML.createElement("pre", { "class":`profile`,});profile.frames.join('\n'); } Modal}}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings('dirty'), slots: {}, events: {} }}
const OttoUsage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OttoUsage__SvelteComponent_ = ReturnType<typeof OttoUsage__SvelteComponent_>;
/*Ωignore_endΩ*/export default OttoUsage__SvelteComponent_;