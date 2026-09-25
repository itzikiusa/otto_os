<script lang="ts">
  // Insights view — two tabs:
  //   • Reports: list/detail. Left = a timeline of generated reports (period
  //     chips, date, the one-line headline, KPI deltas, action-item status);
  //     right = the selected report rendered (ReportDetail: key findings,
  //     Action Plan checklist, summary markdown, sandboxed HTML). Opens on the
  //     last-viewed report, else the newest — never an empty "pick one" pane.
  //   • Health:  Capability & Health Registry (B3, GET /capabilities)
  // Scheduled reports are opt-in (Settings → Insights) and catch-up.
  //
  // Routes: `#/insights`, `#/insights/health`, and `#/insights/r/<kind>/<start>/<end>`
  // (deep link to one report — what "Open in new window" pops out, where the
  // list pane is hidden and only the report shows).
  import { insightsApi } from '../../lib/api/insights';
  import type { InsightKind, InsightReport, InsightRunPeriod } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { isPopout, isTauri, openPopout } from '../../lib/desktop';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { downloadJson, downloadText } from '../../lib/components/exporters';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import CapabilitiesPage from './CapabilitiesPage.svelte';
  import ReportDetail, { type Kpi } from './ReportDetail.svelte';
  import type { ViewMode } from './ReportDetail.svelte';
  import { capabilitiesApi } from './capabilities';
  import type { SupportBundle } from './capabilities';
  import {
    METRIC_KEYS,
    delta,
    indexPathFrom,
    itemStatus,
    parseIndex,
    parseSummary,
    periodKey,
    reportMetrics,
    siblingPath,
    type InsightsIndex,
    type MetricKey,
    type ParsedSummary,
  } from './insightsParse';

  // ---------------------------------------------------------------------------
  // Routing
  // ---------------------------------------------------------------------------

  const tab = $derived(router.parts[1] === 'health' ? 'health' : 'reports');
  /** `daily:20260923_20260923` when the route names one report. */
  const routeKey = $derived.by(() => {
    const [, r, kind, start, end] = router.parts;
    if (r !== 'r' || !kind || !start || !end) return null;
    return periodKey({ kind, period_start: start, period_end: end });
  });
  /** A pop-out window of one report: no list pane, just the report. */
  const detailOnly = $derived(isPopout && routeKey != null);

  // ---------------------------------------------------------------------------
  // Health tab: redacted support bundle
  // ---------------------------------------------------------------------------

  let bundleLoading = $state(false);
  async function downloadBundle(): Promise<void> {
    if (bundleLoading) return;
    bundleLoading = true;
    try {
      const bundle: SupportBundle = await capabilitiesApi.bundle();
      const ts = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
      downloadJson(bundle, `otto-support-bundle-${ts}.json`);
      toasts.success(
        'Support bundle downloaded',
        `${bundle.redaction_hits} secret value${bundle.redaction_hits !== 1 ? 's' : ''} redacted.`,
      );
    } catch (e) {
      toasts.error("Couldn't download the support bundle", e instanceof Error ? e.message : String(e));
    } finally {
      bundleLoading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Reports: load
  // ---------------------------------------------------------------------------

  let reports: InsightReport[] = $state([]);
  let loading = $state(true);
  let loadError: string | null = $state(null);
  /** `index.json` (series + action ledger) — best-effort, may stay empty. */
  let index: InsightsIndex = $state({ series: [], ledger: new Map() });
  /** Full summary markdown per report key (the list carries an 80-line excerpt). */
  let fullMd: Record<string, string> = $state({});

  let loaded = false;
  $effect(() => {
    if (loaded) return;
    loaded = true;
    void load();
    return () => {
      if (pollTimer) clearTimeout(pollTimer);
    };
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      reports = await insightsApi.listReports();
      void loadIndex();
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function loadIndex(): Promise<void> {
    const html = reports.find((r) => r.html_path)?.html_path;
    const path = html ? indexPathFrom(html) : null;
    if (!path) return;
    try {
      index = parseIndex(JSON.parse(await insightsApi.readText(path)));
    } catch {
      /* no index yet / unreadable — KPIs fall back to the summaries alone */
    }
  }

  // ---------------------------------------------------------------------------
  // Derived: parsed summaries, filter, selection
  // ---------------------------------------------------------------------------

  const keyOf = (r: InsightReport): string => periodKey(r);

  const parsedByKey = $derived.by(() => {
    const m = new Map<string, ParsedSummary>();
    for (const r of reports) m.set(keyOf(r), parseSummary(fullMd[keyOf(r)] ?? r.summary));
    return m;
  });
  const seriesByKey = $derived(new Map(index.series.map((s) => [s.periodKey, s])));

  function metricsOf(r: InsightReport) {
    return reportMetrics(parsedByKey.get(keyOf(r))?.metrics ?? {}, seriesByKey.get(keyOf(r)));
  }

  type Filter = 'all' | InsightKind;
  let filter: Filter = $state('all');
  const kinds: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'daily', label: 'Daily' },
    { id: 'weekly', label: 'Weekly' },
    { id: 'monthly', label: 'Monthly' },
    { id: 'adhoc', label: 'Ad-hoc' },
  ];
  const counts = $derived.by(() => {
    const c: Record<string, number> = { all: reports.length };
    for (const r of reports) c[r.kind] = (c[r.kind] ?? 0) + 1;
    return c;
  });
  // Only offer kinds that exist (plus All) — no dead "Ad-hoc 0" chip.
  const visibleKinds = $derived(kinds.filter((k) => k.id === 'all' || (counts[k.id] ?? 0) > 0));
  const filtered = $derived(filter === 'all' ? reports : reports.filter((r) => r.kind === filter));

  let selectedKey: string | null = $state(null);
  /** Phone: list/detail is push navigation — the detail replaces the list. */
  let phoneDetail = $state(false);

  // Open on the routed report, else the last-viewed one, else the newest.
  $effect(() => {
    if (loading || reports.length === 0) return;
    if (routeKey && reports.some((r) => keyOf(r) === routeKey)) {
      if (selectedKey !== routeKey) selectedKey = routeKey;
      return;
    }
    if (selectedKey && reports.some((r) => keyOf(r) === selectedKey)) return;
    selectedKey = initialSelection('insights', reports, keyOf);
  });
  $effect(() => {
    if (selectedKey && !detailOnly) rememberSelection('insights', selectedKey);
  });

  const selected = $derived(reports.find((r) => keyOf(r) === selectedKey) ?? null);

  function select(r: InsightReport): void {
    selectedKey = keyOf(r);
    if (viewport.isPhone) phoneDetail = true;
    if (routeKey) router.replace('insights');
  }

  // Fetch the full summary for the open report (the list has an excerpt).
  $effect(() => {
    const r = selected;
    if (!r?.html_path) return;
    const k = keyOf(r);
    if (fullMd[k] != null) return;
    const path = siblingPath(r.html_path, 'summary');
    if (!path) return;
    void insightsApi
      .readText(path)
      .then((text) => {
        if (text.trim()) fullMd = { ...fullMd, [k]: text };
      })
      .catch(() => {
        fullMd = { ...fullMd, [k]: r.summary };
      });
  });

  // ---------------------------------------------------------------------------
  // KPIs for the selected report: value, delta vs the previous report of the
  // same kind, and a trend over the last 12 of that kind.
  // ---------------------------------------------------------------------------

  const METRIC_LABEL: Record<MetricKey, string> = {
    sessions: 'Sessions',
    turns: 'Turns',
    toolErrors: 'Tool errors',
    spend: 'Spend',
    achievement: 'Achievement',
  };
  const TREND_LEN = 12;

  /** Same-kind reports, newest first (the list is already newest first). */
  function sameKind(r: InsightReport): InsightReport[] {
    return reports.filter((x) => x.kind === r.kind);
  }
  function previousOf(r: InsightReport): InsightReport | null {
    const list = sameKind(r);
    const i = list.findIndex((x) => keyOf(x) === keyOf(r));
    return i >= 0 ? (list[i + 1] ?? null) : null;
  }

  function shortDate(iso: string): string {
    const d = new Date(`${iso}T00:00:00`);
    return Number.isNaN(d.getTime())
      ? iso
      : d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
  }
  function periodShort(r: InsightReport): string {
    return r.period_start === r.period_end
      ? shortDate(r.period_start)
      : `${shortDate(r.period_start)} – ${shortDate(r.period_end)}`;
  }

  const kpis = $derived.by<Kpi[]>(() => {
    const r = selected;
    if (!r) return [];
    const { values, source } = metricsOf(r);
    const prev = previousOf(r);
    const prevValues = prev ? metricsOf(prev).values : {};
    const list = sameKind(r);
    const i = list.findIndex((x) => keyOf(x) === keyOf(r));
    const window = list.slice(i, i + TREND_LEN).reverse();
    const out: Kpi[] = [];
    for (const k of METRIC_KEYS) {
      const v = values[k];
      if (v == null) continue;
      out.push({
        key: k,
        label: METRIC_LABEL[k],
        value: v,
        source: source[k] ?? 'summary',
        delta: delta(k, v, prevValues[k]),
        trend: window.map((x) => metricsOf(x).values[k] ?? null),
        trendLabels: window.map(periodShort),
      });
    }
    return out;
  });

  // ---------------------------------------------------------------------------
  // Detail view mode + exports
  // ---------------------------------------------------------------------------

  const MODE_KEY = 'otto.insights.viewMode';
  function readMode(): ViewMode {
    try {
      const m = localStorage.getItem(MODE_KEY);
      return m === 'markdown' || m === 'html' ? m : 'preview';
    } catch {
      return 'preview';
    }
  }
  let mode: ViewMode = $state(readMode());
  function setMode(m: ViewMode): void {
    mode = m;
    try {
      localStorage.setItem(MODE_KEY, m);
    } catch {
      /* per-device convenience only */
    }
  }

  function markdownOf(r: InsightReport): string {
    return fullMd[keyOf(r)] ?? r.summary;
  }
  function fileStem(r: InsightReport): string {
    return `insights-${r.kind}-${r.period_start}${r.period_end !== r.period_start ? `_${r.period_end}` : ''}`;
  }
  function exportMd(r: InsightReport): void {
    downloadText(markdownOf(r), `${fileStem(r)}.md`, 'text/markdown');
    toasts.success('Summary exported', `${fileStem(r)}.md`);
  }
  async function downloadHtml(r: InsightReport): Promise<void> {
    if (!r.html_path) return;
    try {
      const text = await insightsApi.readText(r.html_path);
      downloadText(text, r.html_path.split('/').at(-1) ?? `${fileStem(r)}.html`, 'text/html');
    } catch (e) {
      toasts.error("Couldn't download the report", e instanceof Error ? e.message : String(e));
    }
  }

  /** Desktop: a native pop-out window on this report's route. Browser: a new
   *  tab whose only content is the report in a sandboxed, opaque-origin
   *  iframe — never the report itself at the app's origin. */
  async function openWindow(r: InsightReport): Promise<void> {
    const route = `insights/r/${r.kind}/${r.period_start}/${r.period_end}`;
    try {
      if (isTauri && (await openPopout(route, `Insights · ${periodShort(r)}`))) return;
      if (!r.html_path) return;
      const html = await insightsApi.readText(r.html_path);
      const attr = html.replace(/&/g, '&amp;').replace(/"/g, '&quot;');
      const wrapper =
        '<!doctype html><meta charset="utf-8"><title>Insight report</title>' +
        '<style>html,body{margin:0;height:100%}iframe{border:0;width:100%;height:100%}</style>' +
        `<iframe sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox" srcdoc="${attr}"></iframe>`;
      const url = URL.createObjectURL(new Blob([wrapper], { type: 'text/html' }));
      window.open(url, '_blank', 'noopener');
      // Not revoked: the new tab needs the URL; it's released when the tab closes.
    } catch (e) {
      toasts.error("Couldn't open the report window", e instanceof Error ? e.message : String(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Run now — with run_id polling
  // ---------------------------------------------------------------------------

  const RUN_OPTIONS: { value: string; label: string }[] = [
    { value: 'day:1', label: 'Yesterday' },
    { value: 'day:2', label: '2 days ago' },
    { value: 'week:1', label: 'Last week' },
    { value: 'week:2', label: '2 weeks ago' },
    { value: 'month:1', label: 'Last month' },
    { value: 'month:2', label: '2 months ago' },
  ];
  let runChoice = $state('day:1');
  let running = $state(false);
  /** Reason the last run did not start (e.g. skill not installed). */
  let runFailReason: string | null = $state(null);
  let pollRunId: string | null = $state(null);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  // 3 s × 100 ≈ 5 min: the banner promises "a few minutes", and the old 20
  // checks (one minute) dropped the banner silently while the agent was still
  // writing — the report then never appeared until a manual reload.
  const POLL_MAX = 100;
  let pollCount = $state(0);

  async function runNow(choice = runChoice): Promise<void> {
    if (running || pollRunId) return;
    const [p, o] = choice.split(':');
    running = true;
    runFailReason = null;
    pollCount = 0;
    try {
      const resp = await insightsApi.run({ period: p as InsightRunPeriod, offset: Number(o) || 1 });
      if (!resp.started) {
        runFailReason = resp.reason ?? 'Check that the insights skill is installed.';
        return;
      }
      if (resp.run_id) {
        pollRunId = resp.run_id;
        schedulePoll();
      } else {
        setTimeout(() => void load(), 2500);
        toasts.success('Insights run started', 'The report appears here when it is ready.');
      }
    } catch (e) {
      toasts.error("Couldn't start the insights run", e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

  function schedulePoll(): void {
    if (pollCount >= POLL_MAX || !pollRunId) {
      if (pollRunId) {
        toasts.info('Still generating the insights report', 'It will show up in the list once the agent finishes — reopen Insights to check.');
      }
      pollRunId = null;
      void load();
      return;
    }
    pollTimer = setTimeout(async () => {
      pollCount += 1;
      const prev = reports.length;
      try {
        reports = await insightsApi.listReports();
      } catch {
        /* keep polling; the next tick retries */
      }
      if (reports.length > prev) {
        pollRunId = null;
        selectedKey = keyOf(reports[0]);
        void loadIndex();
        toasts.success('Insights report ready', periodShort(reports[0]));
      } else {
        schedulePoll();
      }
    }, 3_000);
  }

  // ---------------------------------------------------------------------------
  // ⌘K
  // ---------------------------------------------------------------------------

  $effect(() => {
    const r = selected;
    return registry.register('insights', [
      { id: 'insights.run', title: "Run yesterday's insights report", group: 'Insights', keywords: 'generate usage report now', run: () => void runNow('day:1') },
      { id: 'insights.run-week', title: "Run last week's insights report", group: 'Insights', keywords: 'generate weekly usage report', run: () => void runNow('week:1') },
      ...(r
        ? [
            { id: 'insights.export-md', title: 'Export insights summary as Markdown', group: 'Insights', keywords: 'download md report', run: () => exportMd(r) },
            { id: 'insights.open-window', title: 'Open insights report in new window', group: 'Insights', keywords: 'pop out html', run: () => void openWindow(r) },
          ]
        : []),
    ]);
  });

  // ---------------------------------------------------------------------------
  // List-row helpers
  // ---------------------------------------------------------------------------

  function kindLabel(k: string): string {
    return k === 'adhoc' ? 'Ad-hoc' : k[0].toUpperCase() + k.slice(1);
  }

  const ROW_STATS: MetricKey[] = ['sessions', 'toolErrors', 'achievement'];
  function rowStats(r: InsightReport) {
    const cur = metricsOf(r).values;
    const prev = previousOf(r);
    const pv = prev ? metricsOf(prev).values : {};
    return ROW_STATS.filter((k) => cur[k] != null).map((k) => ({ key: k, value: cur[k]!, d: delta(k, cur[k], pv[k]) }));
  }
  function statText(k: MetricKey, v: number): string {
    const n = Math.round(v);
    if (k === 'achievement') return `${n}% achieved`;
    if (k === 'toolErrors') return `${n.toLocaleString()} error${n === 1 ? '' : 's'}`;
    return `${n.toLocaleString()} session${n === 1 ? '' : 's'}`;
  }
  function statTitle(k: MetricKey, v: number): string {
    return `${METRIC_LABEL[k]}: ${k === 'achievement' ? `${Math.round(v)}%` : Math.round(v).toLocaleString()}`;
  }
  function actionSummary(r: InsightReport): { total: number; regressed: number; improved: number; fresh: number } {
    const acts = parsedByKey.get(keyOf(r))?.actions ?? [];
    let regressed = 0;
    let improved = 0;
    let fresh = 0;
    for (const a of acts) {
      const s = itemStatus(a, index.ledger);
      if (s === 'regressed') regressed++;
      else if (s === 'improved' || s === 'closed') improved++;
      else if (s === 'new') fresh++;
    }
    return { total: acts.length, regressed, improved, fresh };
  }
</script>

<div class="insights-page">
  <PageHeader
    title="Insights"
    subtitle={tab === 'health'
      ? 'What Otto can do right now — config, detected tools and stored accounts'
      : 'Action-first reports on how you work with your agents'}
  >
    {#snippet leading()}
      {#if viewport.isPhone && tab === 'reports' && phoneDetail && selected}
        <button class="icon-btn" onclick={() => (phoneDetail = false)} aria-label="Back to reports" title="Back to reports">
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
    {#snippet tabs()}
      {#if !detailOnly}
        <div class="segmented" role="tablist" aria-label="Insights view">
          <button role="tab" aria-selected={tab === 'reports'} class:active={tab === 'reports'} onclick={() => router.go('insights')}>Reports</button>
          <button role="tab" aria-selected={tab === 'health'} class:active={tab === 'health'} onclick={() => router.go('insights/health')}>Health</button>
        </div>
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if tab === 'health'}
        <button class="btn small" data-icon="download" onclick={downloadBundle} disabled={bundleLoading} title="Download a redacted support bundle (secrets stripped)">
          <Icon name="download" size={12} />
          {bundleLoading ? 'Preparing…' : 'Download support bundle'}
        </button>
      {:else if !detailOnly}
        <button class="icon-btn" data-overflow="-1" data-icon="gear" data-label="Schedule settings" onclick={() => router.go('settings/insights')} aria-label="Schedule settings" title="Schedule settings">
          <Icon name="gear" size={14} />
        </button>
        {#if reports.length > 0 || loading}
          <select class="input run-period" bind:value={runChoice} disabled={running || !!pollRunId} aria-label="Period to report on" title="Period to report on">
            {#each RUN_OPTIONS as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
          </select>
          <button class="btn small primary" disabled={running || !!pollRunId} onclick={() => runNow()}>
            <Icon name="play" size={12} />
            {running ? 'Starting…' : pollRunId ? 'Running…' : 'Run now'}
          </button>
        {/if}
      {/if}
    {/snippet}
  </PageHeader>

  {#if tab === 'health'}
    <PageBody width="readable">
      <CapabilitiesPage />
    </PageBody>
  {:else}
    <PageBody fill padded={false}>
      {#if runFailReason || pollRunId}
        <div class="banners">
          {#if runFailReason}
            <div class="banner warn" role="alert">
              <Icon name="warning" size={14} />
              <div class="banner-body">
                <strong>Couldn't start the insights run.</strong>
                <span>{runFailReason}</span>
              </div>
              <button class="btn small" onclick={() => router.go('settings/skills')}>Open skills settings</button>
              <button class="icon-btn" onclick={() => (runFailReason = null)} aria-label="Dismiss" title="Dismiss"><Icon name="x" size={14} /></button>
            </div>
          {/if}
          {#if pollRunId}
            <div class="banner" role="status">
              <Icon name="refresh" size={14} />
              <span>Generating the report — an agent is reading your transcripts. This can take a few minutes; it appears in the list when it's done.</span>
              <span class="dim">{Math.floor((pollCount * 3) / 60)}:{String((pollCount * 3) % 60).padStart(2, '0')} elapsed</span>
            </div>
          {/if}
        </div>
      {/if}

      {#if loading && reports.length === 0}
        <div class="split">
          <aside class="list-pane" aria-busy="true"><Skeleton rows={6} height={84} /></aside>
          <section class="detail-pane"><p class="dim loading-text" role="status">Loading insight reports…</p></section>
        </div>
      {:else if loadError && reports.length === 0}
        <div class="load-error" role="alert">
          <Icon name="warning" size={16} />
          <div>
            <strong>Couldn't load insight reports.</strong>
            <p class="dim">Otto couldn't read the reports from the daemon. Retry, or check Settings → Logs.</p>
            <p class="dim mono err-detail">{loadError}</p>
          </div>
          <button class="btn small" onclick={load}>Retry</button>
        </div>
      {:else if reports.length === 0}
        <EmptyState
          variant="page"
          icon="gauge"
          title="No insight reports yet"
          body="An agent reads your recent sessions and writes an action-first report: what's working, what's slowing you down, and five things to change. Scheduled reports are off until you turn them on."
          actionLabel={running ? 'Starting…' : "Run yesterday's report"}
          actionIcon="play"
          onaction={() => runNow('day:1')}
        >
          <button class="btn ghost" onclick={() => router.go('settings/insights')}>Turn on scheduled reports</button>
        </EmptyState>
      {:else}
        <div class="split" class:detail-only={detailOnly}>
          {#if !detailOnly}
            <aside class="list-pane" class:hide-phone={viewport.isPhone && phoneDetail} aria-label="Reports">
              <div class="filters" role="group" aria-label="Filter by period">
                {#each visibleKinds as k (k.id)}
                  <button class="filter-chip" class:active={filter === k.id} aria-pressed={filter === k.id} onclick={() => (filter = k.id)}>
                    {k.label} <span class="count">{counts[k.id] ?? 0}</span>
                  </button>
                {/each}
              </div>
              <div class="list" data-testid="report-list">
                {#each filtered as r (keyOf(r))}
                  {@const p = parsedByKey.get(keyOf(r))}
                  {@const acts = actionSummary(r)}
                  <button class="row" class:active={keyOf(r) === selectedKey} aria-current={keyOf(r) === selectedKey ? 'true' : undefined} onclick={() => select(r)}>
                    <div class="row-top">
                      <span class="chip">{kindLabel(r.kind)}</span>
                      <span class="row-date">{periodShort(r)}</span>
                      <span class="grow"></span>
                      {#if r.created_at}<span class="dim row-rel" title={new Date(r.created_at).toLocaleString()}>{rel(r.created_at)}</span>{/if}
                    </div>
                    {#if p?.headline}
                      <div class="row-headline">{p.headline}</div>
                    {:else if !r.summary.trim()}
                      <div class="row-headline dim">No summary yet — the run may still be writing it.</div>
                    {/if}
                    {#if rowStats(r).length > 0}
                      <div class="row-stats">
                        {#each rowStats(r) as s (s.key)}
                          <span class="stat {s.d?.tone ?? 'neutral'}" title={statTitle(s.key, s.value)}>
                            {statText(s.key, s.value)}
                            {#if s.d && s.d.direction !== 'flat'}<Icon name={s.d.direction === 'up' ? 'arrowUp' : 'arrowDown'} size={12} />{/if}
                          </span>
                        {/each}
                      </div>
                    {/if}
                    {#if acts.total > 0}
                      <div class="row-actions">
                        <span class="dim">{acts.total} action{acts.total === 1 ? '' : 's'}</span>
                        {#if acts.regressed}<span class="tag warning"><span class="dot"></span>{acts.regressed} regressed</span>{/if}
                        {#if acts.improved}<span class="tag success"><span class="dot"></span>{acts.improved} improved</span>{/if}
                        {#if acts.fresh}<span class="tag info"><span class="dot"></span>{acts.fresh} new</span>{/if}
                      </div>
                    {/if}
                  </button>
                {:else}
                  <p class="dim list-empty">No {filter === 'adhoc' ? 'ad-hoc' : filter} reports. <button class="btn small ghost" onclick={() => (filter = 'all')}>Show all</button></p>
                {/each}
              </div>
            </aside>
          {/if}
          <section class="detail-pane" class:hide-phone={viewport.isPhone && !phoneDetail && !detailOnly}>
            {#if selected}
              {@const parsed = parsedByKey.get(keyOf(selected)) ?? parseSummary(selected.summary)}
              {@const prev = previousOf(selected)}
              <ReportDetail
                report={selected}
                {parsed}
                markdown={markdownOf(selected)}
                {kpis}
                compareLabel={prev ? periodShort(prev) : null}
                ledger={index.ledger}
                {mode}
                onmode={setMode}
                onopenwindow={() => openWindow(selected)}
                onexportmd={() => exportMd(selected)}
                ondownloadhtml={() => downloadHtml(selected)}
              />
            {:else if routeKey}
              <EmptyState variant="page" icon="gauge" title="This report isn't on disk" body="It may have been removed from the insights folder. Open the list to pick another period.">
                <button class="btn ghost" onclick={() => router.go('insights')}>Show all reports</button>
              </EmptyState>
            {/if}
          </section>
        </div>
      {/if}
    </PageBody>
  {/if}
</div>

<style>
  .insights-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .run-period {
    width: auto;
    height: 22px;
    padding-block: 0;
    font-size: var(--fs-s);
  }

  .banners {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 16px 0;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-s);
  }
  .banner.warn {
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: var(--warning-soft);
  }
  .banner.warn > :global(svg) {
    color: var(--warning);
  }
  .banner-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .split {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .list-pane {
    width: 320px;
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
  }
  .detail-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .loading-text {
    padding: 20px;
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .filter-chip:hover {
    background: var(--hover);
    color: var(--text);
  }
  .filter-chip.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
  }
  .filter-chip .count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    width: 100%;
    gap: 5px;
    padding: 9px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
    font: inherit;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 28%, transparent);
  }
  .row-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row-date {
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .row-rel {
    font-size: var(--fs-xs);
  }
  .row-headline {
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .row-headline.dim {
    color: var(--text-dim);
  }
  .row-stats {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .stat {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--text-dim);
  }
  .stat.good {
    color: var(--success);
  }
  .stat.bad {
    color: var(--warning);
  }
  .row-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-xs);
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .tag .dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: currentColor;
  }
  .tag.warning {
    color: var(--warning);
  }
  .tag.success {
    color: var(--success);
  }
  .tag.info {
    color: var(--info);
  }
  .list-empty {
    padding: 12px;
    font-size: var(--fs-s);
  }

  .load-error {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin: 20px;
    padding: 14px 16px;
    max-width: 720px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .load-error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .load-error > div {
    flex: 1;
    min-width: 0;
  }
  .load-error p {
    margin: 4px 0 0;
  }
  .err-detail {
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }

  @media (max-width: 1024px) {
    .list-pane {
      width: 280px;
    }
  }
  @media (max-width: 640px) {
    .list-pane {
      width: 100%;
      border-inline-end: none;
    }
    .hide-phone {
      display: none;
    }
  }
</style>
