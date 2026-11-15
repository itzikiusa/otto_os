<script lang="ts">
  import { announce } from '../../lib/announce.svelte';
  import { untrack } from 'svelte';
  import { plural } from '../../lib/plural';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
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
  import { toastError } from '../../lib/toastError';
  import { onTabKey } from '../../lib/tabKeys';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
  import type { InsightReport } from '../../lib/api/types';
  import { scheduleCopy } from './scheduleStatus';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { rel, now } from '../../lib/stores/now.svelte';
  import { isPopout, isTauri, openPopout } from '../../lib/desktop';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { downloadJson, downloadText } from '../../lib/components/exporters';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import CapabilitiesPage from './CapabilitiesPage.svelte';
  import ReportDetail, { type Kpi } from './ReportDetail.svelte';
  import type { ViewMode } from './ReportDetail.svelte';
  import { capabilitiesApi } from './capabilities';
  import type { SupportBundle } from './capabilities';
  import { escapeHtml } from '../../lib/html';
  import { METRIC_KEYS, delta, itemStatus, parseSummaryCached,
    periodKey, reportMetrics, siblingPath, type MetricKey, type ParsedSummary } from './insightsParse';
  import { insightsStore as s, type InsightsFilter } from './insightsStore.svelte';

  // ---------------------------------------------------------------------------
  // Routing
  // ---------------------------------------------------------------------------

  // The report list's width — drag / ←→ on the divider, remembered across visits.
  let listW = $state(loadPaneWidth('insights.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));

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
        `${plural(bundle.redaction_hits, 'secret value')} redacted.`,
      );
    } catch (e) {
      toastError('Couldn’t download the support bundle', e);
    } finally {
      bundleLoading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Reports: load (backed by singleton insightsStore so state survives navigation)
  // ---------------------------------------------------------------------------

  let mounted = false;
  $effect(() => {
    if (mounted) return;
    mounted = true;
    s.mount(untrack(() => routeKey));
  });

  const emptySchedule = $derived(scheduleCopy(s.schedule));

  function load(): Promise<void> {
    return s.load(routeKey, true);
  }

  function loadOlderReports(): Promise<void> {
    return s.loadOlderReports(shownRows.length, filtered.length);
  }

  // ---------------------------------------------------------------------------
  // Derived: parsed summaries, filter, selection
  // ---------------------------------------------------------------------------

  const keyOf = (r: InsightReport): string => periodKey(r);

  const parsedByKey = $derived.by(() => {
    const m = new Map<string, ParsedSummary>();
    for (const r of s.reports) m.set(keyOf(r), parseSummaryCached(s.fullMd[keyOf(r)] ?? r.summary));
    return m;
  });
  const seriesByKey = $derived(new Map(s.index.series.map((x) => [x.periodKey, x])));

  function metricsOf(r: InsightReport) {
    return reportMetrics(parsedByKey.get(keyOf(r))?.metrics ?? {}, seriesByKey.get(keyOf(r)));
  }

  const kinds: { id: InsightsFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'daily', label: 'Daily' },
    { id: 'weekly', label: 'Weekly' },
    { id: 'monthly', label: 'Monthly' },
    { id: 'adhoc', label: 'Ad-hoc' },
  ];
  const counts = $derived.by(() => {
    const c: Record<string, number> = { all: s.reports.length };
    for (const r of s.reports) c[r.kind] = (c[r.kind] ?? 0) + 1;
    return c;
  });
  // Only offer kinds that exist (plus All) — no dead "Ad-hoc 0" chip.
  const visibleKinds = $derived(kinds.filter((k) => k.id === 'all' || (counts[k.id] ?? 0) > 0));
  const filtered = $derived(s.filter === 'all' ? s.reports : s.reports.filter((r) => r.kind === s.filter));

  const selectedIndex = $derived(s.selectedKey ? filtered.findIndex((r) => keyOf(r) === s.selectedKey) : -1);
  const shownRows = $derived(filtered.slice(0, Math.max(s.listShown, selectedIndex + 1)));
  /** Phone: list/detail is push navigation — the detail replaces the list. */
  let phoneDetail = $state(false);
  $effect(() => { phoneDetail = routeKey !== null; });

  // Open on the routed report, else the last-viewed one, else the newest.
  $effect(() => {
    if (s.loading || s.reports.length === 0) return;
    if (routeKey) {
      if (s.selectedKey !== routeKey) s.selectedKey = routeKey;
      return;
    }
    if (s.selectedKey && filtered.some((r) => keyOf(r) === s.selectedKey)) return;
    s.selectedKey = initialSelection('insights', filtered, keyOf);
  });
  $effect(() => {
    if (s.selectedKey && !detailOnly) rememberSelection('insights', s.selectedKey);
  });

  const selected = $derived(s.reports.find((r) => keyOf(r) === s.selectedKey) ?? null);

  function select(r: InsightReport): void {
    s.selectedKey = keyOf(r);
    if (viewport.isPhone) phoneDetail = true;
    if (routeKey) router.replace('insights');
  }

  $effect(() => {
    const r = selected;
    if (!r) return;
    const prev = untrack(() => previousOf(r));
    const want = [r, ...(prev ? [prev] : [])].filter((x) => untrack(() => s.fullMd[keyOf(x)]) == null);
    if (!want.length) return;
    for (const x of want) {
      const k = keyOf(x);
      const path = x.html_path ? siblingPath(x.html_path, 'summary') : null;
      const summary = path ? insightsApi.readText(path) : insightsApi.reportStatus(k, true).then(({ report }) => report?.summary ?? '');
      void summary
        .then((text) => {
          if (text.trim()) s.fullMd = { ...s.fullMd, [k]: text };
        })
        .catch(() => {
          s.fullMd = { ...s.fullMd, [k]: x.summary };
        });
    }
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

  function sameKind(r: InsightReport): InsightReport[] {
    return s.reports.filter((x) => x.kind === r.kind);
  }
  const previousByKey = $derived.by(() => {
    const m = new Map<string, InsightReport | null>();
    const byKind = new Map<string, InsightReport[]>();
    for (const r of s.reports) {
      const list = byKind.get(r.kind);
      if (list) list.push(r);
      else byKind.set(r.kind, [r]);
    }
    for (const list of byKind.values()) list.forEach((r, i) => m.set(keyOf(r), list[i + 1] ?? null));
    return m;
  });
  function previousOf(r: InsightReport): InsightReport | null {
    return previousByKey.get(keyOf(r)) ?? null;
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
    return s.fullMd[keyOf(r)] ?? r.summary;
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
      toastError('Couldn’t download the report', e);
    }
  }

  /** Desktop: a native pop-out window on this report's route. Browser: a new
   *  tab whose only content is the report in a sandboxed, opaque-origin
   *  iframe — never the report itself at the app's origin. Popups it opens
   *  stay sandboxed (no `allow-popups-to-escape-sandbox`). */
  async function openWindow(r: InsightReport): Promise<void> {
    const route = `insights/r/${r.kind}/${r.period_start}/${r.period_end}`;
    try {
      if (isTauri && (await openPopout(route, `Insights · ${periodShort(r)}`))) return;
      if (!r.html_path) return;
      const html = await insightsApi.readText(r.html_path);
      const attr = escapeHtml(html);
      const wrapper =
        '<!doctype html><meta charset="utf-8"><title>Insight report</title>' +
        '<style>html,body{margin:0;height:100%}iframe{border:0;width:100%;height:100%}</style>' +
        `<iframe sandbox="allow-scripts allow-popups" srcdoc="${attr}"></iframe>`;
      const url = URL.createObjectURL(new Blob([wrapper], { type: 'text/html' }));
      window.open(url, '_blank', 'noopener');
      // Not revoked: the new tab needs the URL; it's released when the tab closes.
    } catch (e) {
      toastError('Couldn’t open the report window', e);
    }
  }

  // ---------------------------------------------------------------------------
  // Run now — delegated to singleton insightsStore
  // ---------------------------------------------------------------------------

  const RUN_OPTIONS: { value: string; label: string }[] = [
    { value: 'day:1', label: 'Yesterday' },
    { value: 'day:2', label: '2 days ago' },
    { value: 'week:1', label: 'Last week' },
    { value: 'week:2', label: '2 weeks ago' },
    { value: 'month:1', label: 'Last month' },
    { value: 'month:2', label: '2 months ago' },
  ];
  const canRun = $derived(auth.isRoot);
  const pollElapsed = $derived(Math.max(0, Math.floor((now() - s.pollStartedAt) / 1000)));
  function runNow(choice = s.runChoice): Promise<void> {
    return s.runNow(canRun, choice);
  }
  function stopRun(): Promise<void> {
    return s.stopRun();
  }

  // ---------------------------------------------------------------------------
  // ⌘K
  // ---------------------------------------------------------------------------

  $effect(() => {
    const r = selected;
    return registry.register('insights', [
      ...(canRun ? [
      { id: 'insights.run', title: "Run yesterday’s insights report", group: 'Insights', keywords: 'generate usage report now', run: () => void runNow('day:1') },
      { id: 'insights.run-week', title: "Run last week’s insights report", group: 'Insights', keywords: 'generate weekly usage report', run: () => void runNow('week:1') },
      ] : []),
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
    if (k === 'toolErrors') return `${n.toLocaleString()} ${n === 1 ? 'error' : 'errors'}`;
    return `${n.toLocaleString()} ${n === 1 ? 'session' : 'sessions'}`;
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
      const st = itemStatus(a, s.index.ledger);
      if (st === 'regressed') regressed++;
      else if (st === 'improved' || st === 'closed') improved++;
      else if (st === 'new') fresh++;
    }
    return { total: acts.length, regressed, improved, fresh };
  }
  $effect(() => { if (s.runFailReason) announce(`${s.runFailKind === 'ended' ? 'The insights run didn’t publish a report.' : 'Couldn’t start the insights run.'} ${s.runFailReason}`, { assertive: true }); });
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
        <button class="icon-btn" onclick={() => { phoneDetail = false; if (routeKey) router.replace('insights'); }} aria-label="Back to reports" title="Back to reports">
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
    {#snippet tabs()}
      {#if !detailOnly}
        <div class="segmented" role="tablist" tabindex="-1" aria-label="Insights view" onkeydown={onTabKey}>
          <button role="tab" tabindex={tab === 'reports' ? 0 : -1} aria-selected={tab === 'reports'} class:active={tab === 'reports'} onclick={() => router.go('insights')}>Reports</button>
          <button role="tab" tabindex={tab === 'health' ? 0 : -1} aria-selected={tab === 'health'} class:active={tab === 'health'} onclick={() => router.go('insights/health')}>Health</button>
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
        {#if canRun && (s.reports.length > 0 || s.loading)}
          <select class="input run-period" bind:value={s.runChoice} disabled={s.loading || !!s.loadError || s.running || !!s.pollRunId} aria-label="Period to report on" title="Period to report on">
            {#each RUN_OPTIONS as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
          </select>
          <button class="btn small primary" disabled={s.loading || !!s.loadError || s.running || !!s.pollRunId} onclick={() => void runNow()}>
            <Icon name="play" size={12} />
            {s.running ? 'Starting…' : s.pollRunId ? 'Running…' : 'Run now'}
          </button>
        {/if}
      {/if}
    {/snippet}
  </PageHeader>

  <!-- Always mounted (outside the tab switch), so the edge is spoken; never
       holds the ticking elapsed clock. -->
  <div class="sr-only" role="status">{s.pollRunId ? 'Generating the report' : ''}</div>
  {#if tab === 'health'}
    <PageBody width="readable">
      <CapabilitiesPage />
    </PageBody>
  {:else}
    <PageBody fill padded={false}>
      {#if s.runFailReason || s.pollRunId}
        <div class="banners">
          {#if s.runFailReason}
            <div class="banner warn">
              <Icon name="warning" size={14} />
              <div class="banner-body">
                <strong>{s.runFailKind === 'ended' ? 'The insights run didn’t publish a report.' : 'Couldn’t start the insights run.'}</strong>
                <span>{s.runFailReason}</span>
              </div>
              {#if s.runFailKind === 'ended' && s.endedRunId}
                <button class="btn small" onclick={() => s.endedRunId && ws.navigateToSession(s.endedRunId)}><Icon name="terminal" size={12} /> View live session</button>
              {:else}
                <button class="btn small" onclick={() => router.go('skills-eval/bundled')}>Open Skills Lab → Bundled</button>
              {/if}
              <button class="icon-btn" onclick={() => { s.runFailReason = null; s.runFailKind = null; s.endedRunId = null; }} aria-label="Dismiss" title="Dismiss"><Icon name="x" size={14} /></button>
            </div>
          {/if}
          {#if s.pollRunId}
            <!-- Not a live region: the elapsed clock ticks every second and the
                 buttons' names would be re-read with it. The sr-only status
                 above the banners says "Generating the report" once. -->
            <div class="banner">
              <Icon name="refresh" size={14} />
              <span class="banner-text">Generating the report — an agent is reading your transcripts. This can take a few minutes; it appears in the list when it’s done.</span>
              <span class="dim" aria-hidden="true">{Math.floor(pollElapsed / 60)}:{String(pollElapsed % 60).padStart(2, '0')} elapsed</span>
              <button class="btn small" onclick={() => s.pollRunId && ws.navigateToSession(s.pollRunId)}><Icon name="terminal" size={12} /> View live session</button>
              {#if canRun}<button class="btn small" disabled={s.stoppingRun} aria-busy={s.stoppingRun} onclick={() => void stopRun()}><Icon name="stop" size={12} /> {s.stoppingRun ? 'Stopping…' : 'Stop'}</button>{/if}
            </div>
          {/if}
        </div>
      {/if}

      {#if s.loadError && s.reports.length > 0}
        <LoadState what="insight reports" variant="compact" error={s.loadError} empty={true} onretry={load} />
      {/if}
      {#if s.loading && s.reports.length === 0}
        <div class="split">
          <!-- One announcing skeleton (the list); the detail's is silent. -->
          <aside class="list-pane"><Skeleton rows={6} height={84} label="insight reports" /></aside>
          <section class="detail-pane" class:hide-phone={viewport.isPhone}><Skeleton rows={4} height={36} announce={false} /></section>
        </div>
      {:else if s.loadError && s.reports.length === 0}
        <!-- Same inline error + Retry as every other module's failed load. -->
        <LoadState what="insight reports" variant="page" loading={s.loading} error={s.loadError} empty onretry={load} />
      {:else if s.reports.length === 0}
        <EmptyState
          variant="page"
          icon="gauge"
          title="No insight reports yet"
          body={`An agent reads your recent sessions and writes an action-first report: what’s working, what’s slowing you down, and five things to change. ${emptySchedule.text}`.trim()}
          actionLabel={canRun ? (s.running ? 'Starting…' : "Run yesterday’s report") : undefined}
          actionIcon="play"
          onaction={canRun ? () => void runNow('day:1') : undefined}
        >
          <button class="btn ghost" onclick={() => router.go('settings/insights')}>{emptySchedule.offerTurnOn ? 'Turn on scheduled reports' : 'Schedule settings'}</button>
        </EmptyState>
      {:else}
        <div class="split" class:detail-only={detailOnly}>
          {#if !detailOnly}
            <aside class="list-pane" class:hide-phone={viewport.isPhone && phoneDetail} aria-label="Reports" style="--list-pane-w:{listW}px">
              <div class="filters" role="group" aria-label="Filter by period">
                {#each visibleKinds as k (k.id)}
                  <button class="filter-chip" class:active={s.filter === k.id} aria-pressed={s.filter === k.id} onclick={() => { s.setFilter(k.id); if (routeKey) router.replace('insights'); }}>
                    {k.label} <span class="count">{counts[k.id] ?? 0}</span>
                  </button>
                {/each}
              </div>
              <div class="list" data-testid="report-list">
                {#each shownRows as r (keyOf(r))}
                  {@const p = parsedByKey.get(keyOf(r))}
                  {@const acts = actionSummary(r)}
                  {@const stats = rowStats(r)}
                  <button class="rep-row" class:active={keyOf(r) === s.selectedKey} aria-current={keyOf(r) === s.selectedKey ? 'true' : undefined} onclick={() => select(r)}>
                    <div class="row-top">
                      <span class="chip">{kindLabel(r.kind)}</span>
                      <span class="row-date">{periodShort(r)}</span>
                      <span class="grow"></span>
                      {#if r.created_at}<span class="dim row-rel" title={new Date(r.created_at).toLocaleString()}>{rel(r.created_at)}</span>{/if}
                    </div>
                    {#if p?.headline}
                      <div class="row-headline">{p.headline}</div>
                    {:else if !r.summary.trim() && !r.html_path}
                      <div class="row-headline dim">No summary yet — the run may still be writing it.</div>
                    {/if}
                    {#if stats.length > 0}
                      <div class="row-stats">
                        {#each stats as st (st.key)}
                          <span class="stat {st.d?.tone ?? 'neutral'}" title={statTitle(st.key, st.value)}>
                            {statText(st.key, st.value)}
                            {#if st.d && st.d.direction !== 'flat'}<Icon name={st.d.direction === 'up' ? 'arrowUp' : 'arrowDown'} size={12} />{/if}
                          </span>
                        {/each}
                      </div>
                    {/if}
                    {#if acts.total > 0}
                      <div class="row-actions">
                        <span class="dim">{plural(acts.total, 'action')}</span>
                        {#if acts.regressed}<StatusBadge variant="text" tone="warning" label="{acts.regressed} regressed" />{/if}
                        {#if acts.improved}<StatusBadge variant="text" tone="success" label="{acts.improved} improved" />{/if}
                        {#if acts.fresh}<StatusBadge variant="text" tone="info" label="{acts.fresh} new" />{/if}
                      </div>
                    {/if}
                  </button>
                {:else}
                  <p class="dim list-empty">No {s.filter === 'adhoc' ? 'ad-hoc' : s.filter} reports. <button class="btn small ghost" onclick={() => s.setFilter('all')}>Show all</button></p>
                {/each}
                {#if filtered.length > shownRows.length || s.hasMoreReports}
                  <button class="btn small ghost more-reports" onclick={loadOlderReports} disabled={s.loadingMore}>
                    {s.loadingMore ? 'Loading older reports…' : 'Show older reports'}
                  </button>
                {/if}
              </div>
            </aside>
            {#if !viewport.isPhone}
              <PaneDivider bind:width={listW} storageKey="insights.listW" label="Resize the reports list" />
            {/if}
          {/if}
          <section class="detail-pane" class:hide-phone={viewport.isPhone && !phoneDetail && !detailOnly}>
            {#if selected}
              {@const parsed = parsedByKey.get(keyOf(selected)) ?? parseSummaryCached(selected.summary)}
              {@const prev = previousOf(selected)}
              <ReportDetail
                report={selected}
                {parsed}
                markdown={markdownOf(selected)}
                {kpis}
                compareLabel={prev ? periodShort(prev) : null}
                ledger={s.index.ledger}
                {mode}
                onmode={setMode}
                onopenwindow={() => openWindow(selected)}
                onexportmd={() => exportMd(selected)}
                ondownloadhtml={() => downloadHtml(selected)}
              />
            {:else if routeKey}
              <EmptyState variant="page" icon="gauge" title="This report isn’t on disk" body="It may have been removed from the insights folder. Open the list to pick another period.">
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
    flex-wrap: wrap;
    gap: 10px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-s);
  }
  .banner-text {
    flex: 1 1 180px;
    min-width: 0;
  }
  .banner.warn {
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: var(--warning-soft);
  }
  .banner.warn > :global(svg) {
    color: var(--warning);
  }
  .banner-body {
    flex: 1 1 160px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  @media (pointer: coarse), (max-width: 640px) {
    .run-period {
      min-height: var(--hit-min);
      height: auto;
    }
  }

  .split {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .list-pane {
    width: var(--list-pane-w, 280px);
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface);
  }
  .detail-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
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
    gap: 4px;
    height: 22px;
    padding: 0 8px;
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
    border-color: var(--accent-line);
    color: var(--text);
  }
  .filter-chip .count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  @media (pointer: coarse), (max-width: 640px) {
    .filter-chip { min-height: var(--hit-min); height: auto; }
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
  .rep-row {
    content-visibility: auto;
    contain-intrinsic-size: auto 96px;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    width: 100%;
    gap: 4px;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    text-align: start;
    cursor: pointer;
    font: inherit;
  }
  .rep-row:hover {
    background: var(--hover);
  }
  .rep-row.active {
    background: var(--accent-soft);
    border-color: var(--accent-soft-strong);
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
  .more-reports {
    align-self: center;
    margin: 6px 0;
  }
  .list-empty {
    padding: 12px;
    font-size: var(--fs-s);
  }

  @media (max-width: 640px) {
    .list-pane {
      width: 100%;
    }
    .hide-phone {
      display: none;
    }
  }
</style>
