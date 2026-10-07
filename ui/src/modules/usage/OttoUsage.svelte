<script lang="ts">
  import { pollWhileVisible } from '../../lib/poll';
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { TelemetryConfig, TelemetryOverview, TelemetryStatus, TelemetrySuggestion, TelemetrySpan, TelemetryProfile } from '../../lib/api/types';
  import { telemetrySettingsChanged } from '../../lib/telemetryBoot';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import MetricChart from '../../lib/components/MetricChart.svelte';
  import { formatCount as fmt, formatBytes, formatSeconds, type MetricChartPoint } from '../../lib/metric-format';
  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();
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
      if (!disposed && seq === loadSeq) loadError = e instanceof Error ? e.message : 'Could not load application telemetry';
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
    } catch (e) { error = e instanceof Error ? e.message : 'Could not save telemetry settings'; }
    finally { busy = false; }
  }
  async function analyze(): Promise<void> {
    if (busy) return;
    busy = true; error = '';
    try { suggestions = await api.post<TelemetrySuggestion[]>('/telemetry/analyze'); await load(); }
    catch (e) { error = e instanceof Error ? e.message : 'Could not analyze performance'; }
    finally { busy = false; }
  }
  async function dismiss(id: string): Promise<void> {
    if (busy) return;
    busy = true;
    try { await api.post(`/telemetry/suggestions/${encodeURIComponent(id)}/dismiss`); suggestions = suggestions.map((s) => s.id === id ? { ...s, dismissed: true } : s); }
    catch (e) { error = e instanceof Error ? e.message : 'Could not dismiss suggestion'; }
    finally { busy = false; }
  }
  async function openTrace(id: string): Promise<void> {
    const seq = ++detailSeq;
    traceId = id; trace = []; traceBusy = true; traceError = '';
    try { const spans = await api.get<TelemetrySpan[]>(`/telemetry/traces/${encodeURIComponent(id)}`, controller.signal); if (!disposed && seq === detailSeq) trace = spans; }
    catch (e) { if (!disposed && seq === detailSeq) traceError = e instanceof Error ? e.message : 'Could not load trace'; }
    finally { if (!disposed && seq === detailSeq) traceBusy = false; }
  }
  async function capture(): Promise<void> {
    busy = true; error = '';
    try { profile = await api.post<TelemetryProfile>('/telemetry/profile'); }
    catch (e) { error = e instanceof Error ? e.message : 'Could not capture profile'; }
    finally { busy = false; }
  }
</script>

<section class="otto-usage" aria-label="Otto application usage">
  <p class="intro">Application performance on this Mac: page rendering, server operations, CPU and memory. Agent tokens and cost remain in Overview and Report.</p>
  <LoadState what="application telemetry" {loading} empty={!config} error={loadError} onretry={() => void load()}>
    {#if config}
      <form class="settings" onsubmit={(event) => { event.preventDefault(); void save(); }}>
        <label class="toggle"><input type="checkbox" bind:checked={config.enabled} disabled={busy} /> Collect application telemetry on this Mac</label>
        <p class="hint">Opt-in. Otto manages a local OpenTelemetry Collector and stores measurements in its local ClickHouse. Enabling downloads the collector from GitHub. No prompts, query text, file paths, request bodies or credentials are collected.</p>
        <details>
          <summary>Retention, analysis and profiling</summary>
          <div class="fields">
            <label>Trace retention (days)<input type="number" min="1" max="30" step="1" required bind:value={config.traces_days} disabled={busy} /></label>
            <label>Log retention (days)<input type="number" min="1" max="30" step="1" required bind:value={config.logs_days} disabled={busy} /></label>
            <label>Metric retention (days)<input type="number" min="1" max="90" step="1" required bind:value={config.metrics_days} disabled={busy} /></label>
            <label>Analyze every (hours)<input type="number" min="1" max="168" step="1" required bind:value={config.analysis_interval_hours} disabled={busy} /></label>
            <label>Analysis window (hours)<input type="number" min="1" max={config.traces_days * 24} step="1" required bind:value={config.analysis_window_hours} disabled={busy} /></label>
            <label>Slow threshold (ms per call)<input type="number" min="1" max="60000" required bind:value={config.slow_threshold_ms} disabled={busy} /></label>
            <label>Minimum calls<input type="number" min="1" max="10000" step="1" required bind:value={config.min_samples} disabled={busy} /></label>
            <label>Maximum suggestions<input type="number" min="1" max="100" step="1" required bind:value={config.suggestion_limit} disabled={busy} /></label>
            <label>Sample every (seconds)<input type="number" min="2" max="60" step="1" required bind:value={config.sample_interval_secs} disabled={busy} /></label>
            <label>Sustained CPU threshold (%)<input type="number" min="1" max="10000" required bind:value={config.cpu_spike_percent} disabled={busy} /></label>
            <label>Memory increase between samples (MB)<input type="number" min="16" max="65536" required bind:value={config.rss_spike_mb} disabled={busy} /></label>
          </div>
          <label class="toggle"><input type="checkbox" bind:checked={config.native_profiling} disabled={busy || !status?.native_profiling_supported} /> Allow short native stack profiles of Otto</label>
          <p class="hint">Profiles sample the daemon only, on supported macOS systems. They can briefly add CPU load. Browser timings and process metrics do not require native profiling.</p>
        </details>
        <button class="btn primary" type="submit" disabled={busy}>{busy ? 'Working…' : 'Save telemetry settings'}</button>
        {#if message}<p role="status">{message}</p>{/if}
      </form>
    {/if}
  </LoadState>
  {#if error}<div class="notice" role="alert">{error} <button class="btn small" onclick={() => error = ''}>Dismiss error</button></div>{/if}
  {#if status}
    <div class="status" role="status">
      <span class="chip">{!status.enabled ? 'Collection off' : status.collector_ready ? 'Collecting locally' : 'Collector starting or unavailable'}</span>
      <span>{fmt(status.exported)} collector accepted · {fmt(status.queued)} queued · {fmt(status.dropped)} discarded or unacknowledged</span>
      <button class="btn small" onclick={() => void load()} disabled={loading || busy}>Refresh</button>
    </div>
    {#if status.last_error}<p class="notice" role="alert">{status.last_error}. Check that ClickHouse is installed in Usage → Overview, then retry.</p>{/if}
  {/if}
  {#if overview}
    <div class="section-heading"><h2>Application health</h2><label>Window <select bind:value={hours} onchange={() => void load()}><option value={1}>Last hour</option><option value={6}>Last 6 hours</option><option value={24}>Last day</option><option value={168}>Last week</option></select></label></div>
    {#if resources.length}
      <div class="resource-grid">
        {#each resources as resource (resource.name)}
          <article class="resource"><h3>{resource.name}</h3>
            <div>{resource.cpu === null ? 'CPU unavailable' : `${fmt(resource.cpu)}% CPU`} · {resource.rss === null ? 'Memory unavailable' : `${formatBytes(resource.rss * 1048576)} RAM`}</div>
            {#if resource.load !== null}<p>Host load (1 minute): {fmt(resource.load)}</p>{/if}
            {#if resource.peakCpu !== null}<MetricChart series={[{ label: `${resource.name} CPU`, points: resource.points }]} unit="percent" height={120} />{/if}
            {#if resource.peakRss !== null}<MetricChart series={[{ label: `${resource.name} RAM`, points: resource.memory }]} unit="bytes" height={120} />{/if}
            <p class="hint">{resource.peakCpu === null ? 'CPU peak unavailable' : `Peak CPU ${fmt(resource.peakCpu)}%`} · {resource.peakRss === null ? 'Memory peak unavailable' : `Peak RAM ${formatBytes(resource.peakRss * 1048576)}`}</p>
          </article>
        {/each}
      </div>
    {:else}<p class="hint">No resource samples yet. Enable collection and use Otto to record activity.</p>{/if}
    <p class="hint">CPU may exceed 100% across multiple cores. Durations measure wall time, including waiting; they are not CPU time.</p>
    <div class="section-heading"><h2>Slow operations</h2><button class="btn small" onclick={analyze} disabled={busy || !status?.collector_ready}>Analyze now</button></div>
    <p class="hint">Up to {config?.suggestion_limit ?? 100} slow-operation suggestions, plus resource alerts. Calls must exceed the per-call threshold; frequent fast calls alone do not qualify. Last analysis: {date(status?.last_analysis_at)}. Next: {date(status?.next_analysis_at)}.</p>
    {#if active.length}
      <div class="suggestions">{#each active as item (item.id)}
        <article class="suggestion"><h3>{item.component} · {item.name}</h3><p>{item.action}</p>
          {#if item.kind === 'resource'}<p class="hint">{fmt(item.count)} spikes · CPU {item.peak_cpu_percent === null ? 'unavailable' : `${fmt(item.peak_cpu_percent)}%`} · RAM {item.peak_rss_mb === null ? 'unavailable' : formatBytes(item.peak_rss_mb * 1048576)} · {date(item.observed_at)}</p>{:else}<p class="hint">{fmt(item.count)} calls · p95 {formatSeconds(item.p95_ms / 1000)} · max {formatSeconds(item.max_ms / 1000)} · {date(item.observed_at)}</p>{/if}
          <div class="actions">{#if item.trace_id}<button class="btn small" onclick={() => void openTrace(item.trace_id)}>View trace</button>{/if}<button class="btn small" onclick={() => void dismiss(item.id)} disabled={busy}>Dismiss</button></div>
        </article>
      {/each}</div>
    {:else}<p class="hint">No open performance suggestions. The next analysis needs enough slow calls or resource spikes to produce evidence.</p>{/if}
    <h2>Measured operations</h2>
    {#if overview.operations.length}
      <div class="table-scroll" use:scrollFocus role="region" aria-label="Operation latency table">
        <table><thead><tr><th scope="col">Operation</th><th scope="col">Calls</th><th scope="col">p50</th><th scope="col">p95</th><th scope="col">Max</th><th scope="col">Errors</th><th scope="col">Evidence</th></tr></thead>
          <tbody>{#each operations as op (`${op.component}:${op.name}`)}<tr><th scope="row"><span>{op.component}</span><code>{op.name}</code></th><td>{fmt(op.count)}</td><td>{formatSeconds(op.p50_ms / 1000)}</td><td>{formatSeconds(op.p95_ms / 1000)}</td><td>{formatSeconds(op.max_ms / 1000)}</td><td>{fmt(op.errors)}</td><td>{#if op.trace_id}<button class="btn small" onclick={() => void openTrace(op.trace_id)}>Trace</button>{:else}—{/if}</td></tr>{/each}</tbody>
        </table>
      </div>
      {#if operationPages > 1}<nav class="actions" aria-label="Operation pages"><button class="btn small" disabled={operationPage === 0} onclick={() => operationPage--}>Previous</button><span>Page {Math.min(operationPage + 1, operationPages)} of {operationPages}</span><button class="btn small" disabled={operationPage >= operationPages - 1} onclick={() => operationPage++}>Next</button></nav>{/if}
    {:else}<p class="hint">No operations in this window. Open a page or run an operation after enabling collection.</p>{/if}
  {/if}
  <div class="section-heading"><h2>Native profiling</h2><button class="btn small" onclick={async () => { try { profile = await api.get<TelemetryProfile | null>('/telemetry/profile'); message = profile ? '' : 'No retained native profile yet.'; } catch (e) { error = e instanceof Error ? e.message : 'Could not load profile'; } }}>View latest profile</button><button class="btn small" onclick={capture} disabled={busy || !status?.enabled || !config?.native_profiling || !status?.native_profiling_supported}>{busy ? 'Working…' : 'Capture short profile'}</button></div>
  <p class="hint">Sampled daemon stacks help investigate CPU hotspots. Long browser tasks are recorded where the webview supports them. JavaScript heap size is not available in every webview.</p>
</section>

{#if trace !== null}
  <Modal title="Trace detail" width={900} onclose={() => { detailSeq++; traceId = null; trace = null; }}>
    <LoadState what="trace" loading={traceBusy} empty={!trace.length} error={traceError} onretry={() => { if (traceId) void openTrace(traceId); }}>
      {#snippet emptyView()}<p>This trace has expired or has not reached local storage yet.</p>{/snippet}
      <ol class="trace-list">{#each trace as span (span.span_id)}<li><code>{span.name}</code><span>{span.component} · {formatSeconds(span.duration_ms / 1000)} · {span.status}</span><small>Span {span.span_id}{span.parent_span_id ? ` · parent ${span.parent_span_id}` : ' · root'}</small></li>{/each}</ol>
    </LoadState>
  </Modal>
{/if}
{#if profile}<Modal title="Native stack profile" width={900} onclose={() => profile = null}><p>{date(profile.captured_at)} · {profile.duration_seconds} seconds · {profile.format}</p><pre class="profile">{profile.frames.join('\n')}</pre></Modal>{/if}

<style>
  .otto-usage { display: flex; flex-direction: column; gap: 16px; min-inline-size: 0; }
  .intro, .hint { color: var(--text-dim); margin: 0; font-size: var(--fs-s); line-height: 1.5; }
  .settings { display: flex; flex-direction: column; align-items: flex-start; gap: 12px; border: 1px solid var(--border); border-radius: var(--radius-l); padding: 16px; }
  .settings details { inline-size: 100%; }
  .fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; padding-block: 12px; }
  .fields label { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-s); }
  .fields input { inline-size: 100%; min-inline-size: 0; }
  .toggle, .status, .section-heading, .actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .section-heading { justify-content: space-between; }
  h2 { margin: 0; font-size: var(--fs-l); }
  h3 { margin: 0; font-size: var(--fs-m); overflow-wrap: anywhere; }
  .notice { color: var(--danger); background: var(--danger-soft); padding: 12px; border-radius: var(--radius-m); overflow-wrap: anywhere; }
  .resource-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 240px), 1fr)); gap: 12px; }
  .resource, .suggestion { border: 1px solid var(--border); border-radius: var(--radius-l); padding: 12px; min-inline-size: 0; }
  .resource { display: flex; flex-direction: column; gap: 8px; }
  .suggestions { display: flex; flex-direction: column; gap: 10px; }
  .suggestion p { margin-block: 8px; }
  .table-scroll { overflow: auto; max-inline-size: 100%; }
  table { inline-size: 100%; border-collapse: collapse; font-size: var(--fs-s); }
  th, td { text-align: start; padding: 10px; border-block-end: 1px solid var(--border); }
  th code { display: block; font-weight: normal; max-inline-size: 360px; overflow-wrap: anywhere; }
  .trace-list { display: flex; flex-direction: column; gap: 12px; padding-inline-start: 24px; }
  .trace-list li > * { display: block; overflow-wrap: anywhere; }
  .trace-list small { font-size: var(--fs-xs); color: var(--text-dim); }
  .profile { white-space: pre-wrap; overflow-wrap: anywhere; max-block-size: 60vh; overflow: auto; font-size: var(--fs-s); }
</style>
