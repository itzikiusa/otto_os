<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { plural } from '../../lib/plural';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import MetricChart from '../../lib/components/MetricChart.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import { formatCount as fmt, formatBytes, formatSeconds, type MetricChartPoint } from '../../lib/metric-format';
  import { ottoTelemetry as t } from './ottoTelemetryStore.svelte';
  import { usageUi } from './usageUiState.svelte';

  $effect(() => {
    usageUi.telemetryDirty = t.dirty;
  });

  const operationPages = $derived(Math.max(1, Math.ceil((t.overview?.operations.length ?? 0) / 50)));
  const operations = $derived(
    (t.overview?.operations ?? []).slice(
      Math.min(t.operationPage, operationPages - 1) * 50,
      (Math.min(t.operationPage, operationPages - 1) + 1) * 50,
    ),
  );
  function scrollFocus(node: HTMLElement) {
    const update = () => {
      if (node.scrollWidth > node.clientWidth + 1) node.tabIndex = 0;
      else node.removeAttribute('tabindex');
    };
    const observer = new ResizeObserver(update);
    observer.observe(node);
    if (node.firstElementChild) observer.observe(node.firstElementChild);
    return { destroy: () => observer.disconnect() };
  }
  const date = (n: number | null | undefined) => (n ? new Date(n * 1000).toLocaleString() : 'Not yet');
  const active = $derived(t.suggestions.filter((s) => !s.dismissed));
  const resources = $derived.by(() => {
    const groups = new Map<
      string,
      {
        cpu: number | null;
        rss: number | null;
        peakCpu: number | null;
        peakRss: number | null;
        load: number | null;
        points: MetricChartPoint[];
        memory: MetricChartPoint[];
      }
    >();
    for (const sample of t.overview?.resources ?? []) {
      const group = groups.get(sample.process) ?? {
        cpu: null,
        rss: null,
        peakCpu: null,
        peakRss: null,
        load: null,
        points: [],
        memory: [],
      };
      if (sample.cpu_percent !== null) {
        group.cpu = sample.cpu_percent;
        group.peakCpu = Math.max(group.peakCpu ?? 0, sample.cpu_percent);
      }
      if (sample.rss_mb !== null) {
        group.rss = sample.rss_mb;
        group.peakRss = Math.max(group.peakRss ?? 0, sample.rss_mb);
      }
      if (sample.host_load !== null) group.load = sample.host_load;
      group.points.push({ t: sample.timestamp * 1000, v: sample.cpu_percent });
      group.memory.push({ t: sample.timestamp * 1000, v: sample.rss_mb === null ? null : sample.rss_mb * 1048576 });
      groups.set(sample.process, group);
    }
    return Array.from(groups, ([name, data]) => ({ name, ...data }));
  });

  let analyzeBtnEl = $state<HTMLButtonElement | null>(null);
  let suggestionsEl = $state<HTMLElement | null>(null);

  async function handleDismiss(id: string, index: number): Promise<void> {
    const remaining = active.filter((s) => s.id !== id);
    const nextTargetId = remaining[Math.min(index, remaining.length - 1)]?.id ?? null;
    await t.dismiss(id);
    await tick();
    if (nextTargetId && suggestionsEl) {
      const escaped = typeof CSS !== 'undefined' && CSS.escape ? CSS.escape(nextTargetId) : nextTargetId;
      const nextBtn = suggestionsEl.querySelector<HTMLButtonElement>(`button[data-dismiss-id="${escaped}"]`);
      if (nextBtn) {
        nextBtn.focus();
        return;
      }
    }
    analyzeBtnEl?.focus();
  }

  function handleHoursChange(event: Event): void {
    const next = Number((event.currentTarget as HTMLSelectElement).value);
    if (!Number.isFinite(next) || next <= 0) return;
    usageUi.telemetryHours = next;
    t.hours = next;
    void t.load(false);
  }

  onMount(() => {
    usageUi.onDiscardTelemetry = () => t.discardChanges();
    usageUi.telemetryHours = t.hours;
    const stop = t.mount({ getScope: () => ({ hours: usageUi.telemetryHours }) });
    return () => {
      if (usageUi.onDiscardTelemetry) usageUi.onDiscardTelemetry = null;
      stop();
    };
  });
</script>

<section class="otto-usage" aria-label="Otto application usage">
  <p class="intro">Application performance on this Mac: page rendering, server operations, CPU and memory. Agent tokens and cost stay on the Usage page and its Report.</p>
  <LoadState what="application telemetry" loading={t.loading} empty={!t.config} error={t.loadError} onretry={() => void t.load(true)}>
    {#if t.config}
      <form class="settings" onsubmit={(event) => { event.preventDefault(); void t.save(); }}>
        <div class="toggle">
          <Switch
            checked={t.config.enabled}
            onchange={(next) => void t.toggleEnabled(next)}
            disabled={t.togglingEnabled || t.savingRetention}
            label="Collect application telemetry on this Mac"
          />
          <span>Collect application telemetry on this Mac</span>
        </div>
        <p class="hint">Opt-in. Otto manages a local OpenTelemetry Collector and stores measurements in its local ClickHouse. Enabling downloads the collector from GitHub. No prompts, query text, file paths, request bodies or credentials are collected.</p>
        <details>
          <summary>Retention, analysis and profiling</summary>
          <div class="fields">
            <label>Trace retention (days)<input class="input" type="number" min="1" max="30" step="1" required bind:value={t.config.traces_days} disabled={t.savingRetention} /></label>
            <label>Log retention (days)<input class="input" type="number" min="1" max="30" step="1" required bind:value={t.config.logs_days} disabled={t.savingRetention} /></label>
            <label>Metric retention (days)<input class="input" type="number" min="1" max="90" step="1" required bind:value={t.config.metrics_days} disabled={t.savingRetention} /></label>
            <label>Analyze every (hours)<input class="input" type="number" min="1" max="168" step="1" required bind:value={t.config.analysis_interval_hours} disabled={t.savingRetention} /></label>
            <label>Analysis window (hours)<input class="input" type="number" min="1" max={t.config.traces_days * 24} step="1" required bind:value={t.config.analysis_window_hours} disabled={t.savingRetention} /></label>
            <label>Slow threshold (ms per call)<input class="input" type="number" min="1" max="60000" required bind:value={t.config.slow_threshold_ms} disabled={t.savingRetention} /></label>
            <label>Minimum calls<input class="input" type="number" min="1" max="10000" step="1" required bind:value={t.config.min_samples} disabled={t.savingRetention} /></label>
            <label>Maximum suggestions<input class="input" type="number" min="1" max="100" step="1" required bind:value={t.config.suggestion_limit} disabled={t.savingRetention} /></label>
            <label>Sample every (seconds)<input class="input" type="number" min="2" max="60" step="1" required bind:value={t.config.sample_interval_secs} disabled={t.savingRetention} /></label>
            <label>Sustained CPU threshold (%)<input class="input" type="number" max="10000" min="1" required bind:value={t.config.cpu_spike_percent} disabled={t.savingRetention} /></label>
            <label>Memory increase between samples (MB)<input class="input" type="number" min="16" max="65536" required bind:value={t.config.rss_spike_mb} disabled={t.savingRetention} /></label>
          </div>
          <div class="toggle">
            <Switch
              checked={t.config.native_profiling}
              onchange={(next) => { if (t.config) t.config.native_profiling = next; }}
              disabled={t.savingRetention || !t.status?.native_profiling_supported}
              label="Allow short native stack profiles of Otto"
            />
            <span>Allow short native stack profiles of Otto</span>
          </div>
          <p class="hint">Profiles sample the daemon only, on supported macOS systems. They can briefly add CPU load. Browser timings and process metrics do not require native profiling.</p>
        </details>
        <button class="btn primary" type="submit" disabled={t.savingRetention || t.togglingEnabled}>{t.savingRetention ? 'Saving…' : 'Save telemetry settings'}</button>
        {#if t.message}<p class="saved">{t.message}</p>{/if}
      </form>
    {/if}
  </LoadState>
  {#if t.error}<div class="notice">{t.error} <button class="btn small" onclick={() => (t.error = '')}>Dismiss error</button></div>{/if}
  {#if t.status}
    <div class="status">
      <span class="chip">{!t.status.enabled ? 'Collection off' : t.status.collector_ready ? 'Collecting locally' : 'Collector starting or unavailable'}</span>
      <span class="num-inline">{fmt(t.status.exported)} collector accepted · {fmt(t.status.queued)} queued · {fmt(t.status.dropped)} discarded or unacknowledged</span>
      <button class="btn small" onclick={() => void t.load(false)} disabled={t.loading}>Refresh</button>
    </div>
    {#if t.status.last_error}<p class="notice" use:announceShown={{ text: `The local collector reported an error. ${t.status.last_error}`, assertive: true }}>The local collector reported an error. Check that ClickHouse is installed in Usage → Overview, then retry. <span class="dim">{t.status.last_error}</span></p>{/if}
  {/if}
  {#if t.overview}
    <div class="section-heading"><h2>Application health</h2><label class="toggle">Window <select class="input" value={usageUi.telemetryHours} onchange={handleHoursChange}><option value={1}>Last hour</option><option value={6}>Last 6 hours</option><option value={24}>Last day</option><option value={168}>Last week</option></select></label></div>
    {#if resources.length}
      <div class="resource-grid">
        {#each resources as resource (resource.name)}
          <article class="resource"><h3>{resource.name}</h3>
            <div class="num-inline">{resource.cpu === null ? 'CPU unavailable' : `${fmt(resource.cpu)}% CPU`} · {resource.rss === null ? 'Memory unavailable' : `${formatBytes(resource.rss * 1048576)} RAM`}</div>
            {#if resource.load !== null}<p class="num-inline">Host load (1 minute): {fmt(resource.load)}</p>{/if}
            {#if resource.peakCpu !== null}<MetricChart series={[{ label: `${resource.name} CPU`, points: resource.points }]} unit="percent" height={120} />{/if}
            {#if resource.peakRss !== null}<MetricChart series={[{ label: `${resource.name} RAM`, points: resource.memory }]} unit="bytes" height={120} />{/if}
            <p class="hint num-inline">{resource.peakCpu === null ? 'CPU peak unavailable' : `Peak CPU ${fmt(resource.peakCpu)}%`} · {resource.peakRss === null ? 'Memory peak unavailable' : `Peak RAM ${formatBytes(resource.peakRss * 1048576)}`}</p>
          </article>
        {/each}
      </div>
    {:else}<p class="hint">No resource samples yet. Enable collection and use Otto to record activity.</p>{/if}
    <p class="hint">CPU may exceed 100% across multiple cores. Durations measure wall time, including waiting; they are not CPU time.</p>
    <div class="section-heading"><h2>Slow operations</h2><button class="btn small" bind:this={analyzeBtnEl} onclick={() => void t.analyze()} disabled={t.analyzing || !t.status?.collector_ready}>{t.analyzing ? 'Analyzing…' : 'Analyze now'}</button></div>
    <p class="hint">Up to {t.config?.suggestion_limit ?? 100} slow-operation suggestions, plus resource alerts. Calls must exceed the per-call threshold; frequent fast calls alone do not qualify. Last analysis: {date(t.status?.last_analysis_at)}. Next: {date(t.status?.next_analysis_at)}.</p>
    {#if active.length}
      <div class="suggestions" bind:this={suggestionsEl} use:announceShown={`${plural(active.length, 'open performance suggestion')}.`}>
        {#each active as item, idx (item.id)}
          <article class="suggestion"><h3>{item.component} · {item.name}</h3><p>{item.action}</p>
            {#if item.kind === 'resource'}<p class="hint num-inline">{fmt(item.count)} spikes · CPU {item.peak_cpu_percent === null ? 'unavailable' : `${fmt(item.peak_cpu_percent)}%`} · RAM {item.peak_rss_mb === null ? 'unavailable' : formatBytes(item.peak_rss_mb * 1048576)} · {date(item.observed_at)}</p>{:else}<p class="hint num-inline">{fmt(item.count)} calls · p95 {formatSeconds(item.p95_ms / 1000)} · max {formatSeconds(item.max_ms / 1000)} · {date(item.observed_at)}</p>{/if}
            <div class="actions">
              {#if item.trace_id}<button class="btn small" onclick={() => void t.openTrace(item.trace_id)}>View trace</button>{/if}
              <button
                class="btn small"
                data-dismiss-id={item.id}
                onclick={() => void handleDismiss(item.id, idx)}
                disabled={t.clearing && t.dismissingId === item.id}
              >{t.clearing && t.dismissingId === item.id ? 'Dismissing…' : 'Dismiss'}</button>
            </div>
          </article>
        {/each}
      </div>
    {:else}<p class="hint">No open performance suggestions. The next analysis needs enough slow calls or resource spikes to produce evidence.</p>{/if}
    <h2>Measured operations</h2>
    {#if t.overview.operations.length}
      <div class="table-scroll" use:scrollFocus role="region" aria-label="Operation latency table">
        <table><thead><tr><th scope="col">Operation</th><th scope="col" class="num">Calls</th><th scope="col" class="num">p50</th><th scope="col" class="num">p95</th><th scope="col" class="num">Max</th><th scope="col" class="num">Errors</th><th scope="col">Evidence</th></tr></thead>
          <tbody>{#each operations as op (`${op.component}:${op.name}`)}<tr><th scope="row"><span>{op.component}</span><code>{op.name}</code></th><td class="num">{fmt(op.count)}</td><td class="num">{formatSeconds(op.p50_ms / 1000)}</td><td class="num">{formatSeconds(op.p95_ms / 1000)}</td><td class="num">{formatSeconds(op.max_ms / 1000)}</td><td class="num">{fmt(op.errors)}</td><td>{#if op.trace_id}<button class="btn small" onclick={() => void t.openTrace(op.trace_id)}>Trace</button>{:else}—{/if}</td></tr>{/each}</tbody>
        </table>
      </div>
      {#if operationPages > 1}<nav class="actions" aria-label="Operation pages"><button class="btn small" disabled={t.operationPage === 0} onclick={() => t.operationPage--}>Previous</button><span class="num-inline">Page {Math.min(t.operationPage + 1, operationPages)} of {operationPages}</span><button class="btn small" disabled={t.operationPage >= operationPages - 1} onclick={() => t.operationPage++}>Next</button></nav>{/if}
    {:else}<p class="hint">No operations in this window. Open a page or run an operation after enabling collection.</p>{/if}
  {/if}
  <div class="section-heading"><h2>Native profiling</h2><button class="btn small" onclick={() => void t.viewLatestProfile()}>View latest profile</button><button class="btn small" onclick={() => void t.capture()} disabled={t.capturing || !t.status?.enabled || !t.config?.native_profiling || !t.status?.native_profiling_supported}>{t.capturing ? 'Capturing…' : 'Capture short profile'}</button></div>
  <p class="hint">Sampled daemon stacks help investigate CPU hotspots. Long browser tasks are recorded where the webview supports them. JavaScript heap size is not available in every webview.</p>
</section>

{#if t.trace !== null}
  <Modal title="Trace detail" width={680} onclose={() => t.closeTrace()}>
    <LoadState what="trace" loading={t.traceBusy} empty={!t.trace.length} error={t.traceError} onretry={() => { if (t.traceId) void t.openTrace(t.traceId); }}>
      {#snippet emptyView()}<p>This trace has expired or has not reached local storage yet.</p>{/snippet}
      <ol class="trace-list">{#each t.trace as span (span.span_id)}<li><code>{span.name}</code><span class="num-inline">{span.component} · {formatSeconds(span.duration_ms / 1000)} · {span.status}</span><small>Span {span.span_id}{span.parent_span_id ? ` · parent ${span.parent_span_id}` : ' · root'}</small></li>{/each}</ol>
    </LoadState>
  </Modal>
{/if}
{#if t.profile}<Modal title="Native stack profile" width={680} onclose={() => (t.profile = null)}><p class="num-inline">{date(t.profile.captured_at)} · {t.profile.duration_seconds} seconds · {t.profile.format}</p><pre class="profile">{t.profile.frames.join('\n')}</pre></Modal>{/if}

<style>
  .otto-usage { display: flex; flex-direction: column; gap: 16px; min-inline-size: 0; }
  .intro, .hint, .saved { color: var(--text-dim); margin: 0; font-size: var(--fs-s); line-height: 1.5; }
  .saved { color: var(--success); }
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
  th.num, td.num { text-align: end; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .num-inline { font-variant-numeric: tabular-nums; }
  th code { display: block; font-weight: normal; max-inline-size: 360px; overflow-wrap: anywhere; }
  .trace-list { display: flex; flex-direction: column; gap: 12px; padding-inline-start: 24px; }
  .trace-list li > * { display: block; overflow-wrap: anywhere; }
  .trace-list small { font-size: var(--fs-xs); color: var(--text-dim); }
  .profile { white-space: pre-wrap; overflow-wrap: anywhere; max-block-size: 420px; overflow: auto; font-size: var(--fs-s); }
</style>
