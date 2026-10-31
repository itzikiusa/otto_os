///<reference types="svelte" />
;
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
import type { InsightKind, InsightReport, InsightRunPeriod, InsightsConfig } from '../../lib/api/types';
import { scheduleCopy } from './scheduleStatus';
import { toasts } from '../../lib/toast.svelte';
import { router } from '../../lib/router.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { registry } from '../../lib/commands.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { rel, now } from '../../lib/stores/now.svelte';
import { isPopout, isTauri, openPopout } from '../../lib/desktop';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { downloadJson, downloadText } from '../../lib/components/exporters';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import CapabilitiesPage from './CapabilitiesPage.svelte';
import ReportDetail, { type Kpi } from './ReportDetail.svelte';
import type { ViewMode } from './ReportDetail.svelte';
import { capabilitiesApi } from './capabilities';
import type { SupportBundle } from './capabilities';
import { escapeHtml } from '../../lib/html';
import { METRIC_KEYS, delta, indexPathFrom, itemStatus, parseIndex, parseSummaryCached,
    periodKey, reportMetrics, siblingPath, type InsightsIndex, type MetricKey, type ParsedSummary } from './insightsParse';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
  // Reports: load
  // ---------------------------------------------------------------------------

  let reports: InsightReport[] = $state([]);
  let loading = $state(true);
  let loadError: string | null = $state(null);
  let hasMoreReports = $state(false);
  let archiveOffset = 0;
  let loadingMore = $state(false);
  /** `index.json` (series + action ledger) — best-effort, may stay empty. */
  let index: InsightsIndex = $state({ series: [], ledger: new Map() });
  /** Full summary markdown per report key (the list carries an 80-line excerpt). */
  let fullMd: Record<string, string> = $state({});

  let loaded = false;
  let disposed = false;
  $effect(() => {
    if (loaded) return;
    loaded = true;
    void load();
    void restoreActiveRun();
    return () => {
      disposed = true;
      if (pollTimer) clearTimeout(pollTimer);
    };
  });

  /** The schedule config (with its last failed scheduled attempts), so the
   *  empty state says "off" only when it is (error-handling-6-03). A failed
   *  read keeps the generic copy — the reports load is what matters here. */
  let schedule = $state<InsightsConfig | null>(null);
  const emptySchedule = $derived(scheduleCopy(schedule));

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    void insightsApi.getConfig().then((c) => { schedule = c; }).catch(() => { schedule = null; });
    try {
      reports = await insightsApi.listReports();
      archiveOffset = reports.length;
      hasMoreReports = reports.length === 200;
      if (routeKey && !reports.some((r) => keyOf(r) === routeKey)) {
        const linked = await insightsApi.reportStatus(routeKey);
        if (linked.report) reports = [...reports, linked.report];
      }
      void loadIndex();
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function loadOlderReports(): Promise<void> {
    if (loadingMore) return;
    if (shownRows.length < filtered.length) { listShown = shownRows.length + LIST_PAGE; return; }
    loadingMore = true;
    try {
      const page = await insightsApi.listReports(archiveOffset);
      if (disposed) return;
      archiveOffset += page.length;
      hasMoreReports = page.length === 200;
      const ids = new Set(reports.map(keyOf));
      reports = [...reports, ...page.filter((r) => !ids.has(keyOf(r)))];
      listShown += LIST_PAGE;
    } catch (e) { toasts.error('Couldn’t load older reports', loadErrorText(e)); }
    finally { loadingMore = false; }
  }

  let indexRequest = 0;
  async function loadIndex(): Promise<void> {
    const request = ++indexRequest;
    const html = reports.find((r) => r.html_path)?.html_path;
    const path = html ? indexPathFrom(html) : null;
    if (!path) return;
    try {
      const next = parseIndex(JSON.parse(await insightsApi.readText(path)));
      if (!disposed && request === indexRequest) index = next;
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
    for (const r of reports) m.set(keyOf(r), parseSummaryCached(fullMd[keyOf(r)] ?? r.summary));
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
  // SE-17: daily reports grow by 365 a year. Rows vary in height, so instead of
  // a fixed-row window the list mounts a page and grows on demand; the
  // selected report is always mounted, and off-screen rows skip layout
  // (content-visibility).
  const LIST_PAGE = 200;
  let listShown = $state(LIST_PAGE);
  $effect(() => {
    void filter;
    listShown = LIST_PAGE;
  });
  const selectedIndex = $derived(selectedKey ? filtered.findIndex((r) => keyOf(r) === selectedKey) : -1);
  const shownRows = $derived(filtered.slice(0, Math.max(listShown, selectedIndex + 1)));
  /** Phone: list/detail is push navigation — the detail replaces the list. */
  let phoneDetail = $state(false);
  $effect(() => { phoneDetail = routeKey !== null; });

  // Open on the routed report, else the last-viewed one, else the newest.
  $effect(() => {
    if (loading || reports.length === 0) return;
    if (routeKey) {
      if (selectedKey !== routeKey) selectedKey = routeKey;
      return;
    }
    if (selectedKey && filtered.some((r) => keyOf(r) === selectedKey)) return;
    selectedKey = initialSelection('insights', filtered, keyOf);
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

  // Fetch the full summary for the open report (the list has an excerpt) AND
  // for the report its deltas compare against: a metric the excerpt cuts off
  // falls back to the index (tool errors: 0), so comparing the open report's
  // summary value with the previous one's index value showed "Up 24, worse"
  // for a day that actually went 41 → 24.
  $effect(() => {
    const r = selected;
    if (!r) return;
    const prev = untrack(() => previousOf(r));
    const want = [r, ...(prev ? [prev] : [])].filter((x) => untrack(() => fullMd[keyOf(x)]) == null);
    if (!want.length) return;
    let current = true;
    for (const x of want) {
      const k = keyOf(x);
      const path = x.html_path ? siblingPath(x.html_path, 'summary') : null;
      const summary = path ? insightsApi.readText(path) : insightsApi.reportStatus(k, true).then(({ report }) => report?.summary ?? '');
      void summary
        .then((text) => {
          if (current && text.trim()) fullMd = { ...fullMd, [k]: text };
        })
        .catch(() => {
          if (current) fullMd = { ...fullMd, [k]: x.summary };
        });
    }
    return () => { current = false; };
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
  /** Report key → the previous report of the same kind, built once per
   *  `reports` change. `previousOf` used to filter + findIndex over every
   *  report for each row (twice per row) — O(rows × reports) per render. */
  const previousByKey = $derived.by(() => {
    const m = new Map<string, InsightReport | null>();
    const byKind = new Map<string, InsightReport[]>();
    for (const r of reports) {
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
  // Run now — with run_id polling
  // ---------------------------------------------------------------------------

  // Compatibility for older daemons without report_key. New daemons resolve
  // this in their own local timezone, matching the skill collector.
  function localReportKey(period: InsightRunPeriod, offset: number): string {
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    if (period === 'day') start.setDate(start.getDate() - offset);
    else if (period === 'week') start.setDate(start.getDate() - (start.getDay() + 6) % 7 - offset * 7);
    else { start.setDate(1); start.setMonth(start.getMonth() - offset); }
    const end = new Date(start);
    if (period === 'week') end.setDate(end.getDate() + 6);
    else if (period === 'month') { end.setMonth(end.getMonth() + 1); end.setDate(0); }
    const ymd = (d: Date) => `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, '0')}${String(d.getDate()).padStart(2, '0')}`;
    return `${period === 'day' ? 'daily' : period === 'week' ? 'weekly' : 'monthly'}:${ymd(start)}_${ymd(end)}`;
  }

  const RUN_OPTIONS: { value: string; label: string }[] = [
    { value: 'day:1', label: 'Yesterday' },
    { value: 'day:2', label: '2 days ago' },
    { value: 'week:1', label: 'Last week' },
    { value: 'week:2', label: '2 weeks ago' },
    { value: 'month:1', label: 'Last month' },
    { value: 'month:2', label: '2 months ago' },
  ];
  /** Starting / stopping a run is root-only on the daemon (`POST /insights/run`,
   *  `/insights/runs/{id}/cancel` → `require_root`): other members read the
   *  reports but never see controls that can only end in a 403. */
  const canRun = $derived(auth.isRoot);
  let runChoice = $state('day:1');
  let running = $state(false);
  /** Why the last run banner is up: start refused, or a run that ended with no report. */
  let runFailKind = $state<'start' | 'ended' | null>(null);
  /** Reason shown under that banner. */
  let runFailReason: string | null = $state(null);
  /** Session of a run that ended without a report, so the banner can open it. */
  let endedRunId = $state<string | null>(null);
  let pollRunId: string | null = $state(null);
  let pollReportKey: string | null = null;
  let reportBeforeRun: string | null | undefined;
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  // 3 s × 100 ≈ 5 min: the banner promises "a few minutes", and the old 20
  // checks (one minute) dropped the banner silently while the agent was still
  // writing — the report then never appeared until a manual reload.
  const POLL_MAX = 100;
  let pollCount = $state(0);
  /** Consecutive poll ticks the run was missing from the active-run list. */
  let pollGoneTicks = 0;
  /** Wall time is independent of visibility-aware polling attempts. */
  let pollStartedAt = $state(Date.now());
  const pollElapsed = $derived(Math.max(0, Math.floor((now() - pollStartedAt) / 1000)));
  let stoppingRun = $state(false);

  /** A run started earlier (this page was left, or another window started it)
   *  is still generating: bring its banner back and resume polling for it. */
  async function restoreActiveRun(): Promise<void> {
    try {
      const runs = await insightsApi.activeRuns();
      const r = runs[runs.length - 1];
      if (disposed || !r || pollRunId) return;
      pollRunId = r.run_id;
      pollGoneTicks = 0;
      pollReportKey = r.report_key;
      reportBeforeRun = r.report_revision;
      pollCount = 0;
      pollStartedAt = Number.isFinite(Date.parse(r.started_at)) ? Date.parse(r.started_at) : Date.now();
      schedulePoll();
    } catch {
      /* best-effort: without it the page still works, just without the banner */
    }
  }

  async function stopRun(): Promise<void> {
    const id = pollRunId;
    if (!id || stoppingRun) return;
    const ok = await confirmer.ask('Stop generating this insights report? The agent session is ended; nothing already published is removed.', {
      title: 'Stop insights run', danger: true, confirmLabel: 'Stop run',
    });
    if (!ok) return;
    stoppingRun = true;
    try {
      await insightsApi.cancelRun(id);
      if (pollTimer) clearTimeout(pollTimer);
      pollRunId = null;
      toasts.info('Insights run stopped');
    } catch (e) {
      toastError('Couldn’t stop the insights run', e);
    } finally {
      stoppingRun = false;
    }
  }

  async function runNow(choice = runChoice): Promise<void> {
    if (!canRun || loading || loadError || running || pollRunId) return;
    const [p, o] = choice.split(':');
    running = true;
    runFailReason = null;
    runFailKind = null;
    endedRunId = null;
    pollCount = 0;
    pollStartedAt = Date.now();
    const period = p as InsightRunPeriod;
    const offset = Number(o) || 1;
    const fallbackKey = localReportKey(period, offset);
    try {
      const resp = await insightsApi.run({ period, offset });
      if (disposed) return;
      if (!resp.started) {
        runFailKind = 'start';
        runFailReason = resp.reason ?? 'Check that the insights skill is installed.';
        return;
      }
      if (resp.attached) toasts.info('Already generating this report', 'Showing the run in progress instead of starting another.');
      if (resp.run_id) {
        pollRunId = resp.run_id;
        if (resp.attached) {
          void insightsApi.activeRuns().then(runs => {
            const active = runs.find(r => r.run_id === resp.run_id);
            if (!disposed && pollRunId === resp.run_id && active && Number.isFinite(Date.parse(active.started_at))) pollStartedAt = Date.parse(active.started_at);
          }).catch(() => {});
        }
        pollGoneTicks = 0;
        pollReportKey = resp.report_key ?? fallbackKey;
        reportBeforeRun = resp.report_revision;
        schedulePoll();
      } else {
        setTimeout(() => void load(), 2500);
        toasts.success('Insights run started', 'The report appears here when it is ready.');
      }
    } catch (e) {
      toastError('Couldn’t start the insights run', e);
    } finally {
      running = false;
    }
  }

  function schedulePoll(): void {
    if (disposed) return;
    if (pollCount >= POLL_MAX || !pollRunId) {
      if (pollRunId) {
        toasts.info('Still generating the insights report', 'It will show up in the list once the agent finishes — reopen Insights to check.');
      }
      pollRunId = null;
      void load();
      return;
    }
    pollTimer = setTimeout(async () => {
      // Hidden window: wait (the tick isn't counted against POLL_MAX).
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
        schedulePoll();
        return;
      }
      pollCount += 1;
      let ready: InsightReport | null = null;
      try {
        // No key to poll (an older daemon's run): end the banner instead of
        // returning without rescheduling — that left it stuck and blocked Run now.
        if (!pollReportKey) {
          pollRunId = null;
          return;
        }
        const status = await insightsApi.reportStatus(pollReportKey);
        if (disposed) return;
        if (status.report?.html_path && status.html_revision && status.html_revision !== reportBeforeRun) ready = status.report;
      } catch {
        /* keep polling; the next tick retries */
      }
      // The run itself ended (failed, or stopped from another window) without
      // publishing: end the banner with why instead of polling ~5 min and
      // blocking Run now (S17-311). Two consecutive sightings, so a run that
      // finished between the report check and this one isn't misread.
      if (!ready && pollRunId) {
        try {
          const active = await insightsApi.activeRuns();
          if (disposed) return;
          pollGoneTicks = active.some((r) => r.run_id === pollRunId) ? 0 : pollGoneTicks + 1;
        } catch {
          /* unknown — keep polling */
        }
        if (pollGoneTicks >= 2) {
          endedRunId = pollRunId;
          pollRunId = null;
          pollGoneTicks = 0;
          runFailKind = 'ended';
          runFailReason = 'The insights run ended without publishing a report — it failed or was stopped. Its session shows what happened.';
          void load();
          return;
        }
      }
      if (ready) {
        pollRunId = null;
        await load();
        if (disposed) return;
        if (!reports.some((r) => keyOf(r) === keyOf(ready!))) reports = [...reports, ready];
        fullMd = {};
        filter = 'all';
        select(ready);
        void loadIndex();
        toasts.success('Insights report ready', periodShort(ready));
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
      const s = itemStatus(a, index.ledger);
      if (s === 'regressed') regressed++;
      else if (s === 'improved' || s === 'closed') improved++;
      else if (s === 'new') fresh++;
    }
    return { total: acts.length, regressed, improved, fresh };
  }
  $effect(() => { if (runFailReason) announce(`${runFailKind === 'ended' ? 'The insights run didn’t publish a report.' : 'Couldn’t start the insights run.'} ${runFailReason}`, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`insights-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":`Insights`,"subtitle":tab === 'health'
      ? 'What Otto can do right now — config, detected tools and stored accounts'
      : 'Action-first reports on how you work with your agents',leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if(viewport.isPhone && tab === 'reports' && phoneDetail && selected){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => { phoneDetail = false; if (routeKey) router.replace('insights'); },"aria-label":`Back to reports`,"title":`Back to reports`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
         }
      }
    };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
      if(!detailOnly){
         { svelteHTML.createElement("div", {         "class":`segmented`,"role":`tablist`,"tabindex":-1,"aria-label":`Insights view`,"onkeydown":onTabKey,});
           { svelteHTML.createElement("button", {        "role":`tab`,"tabindex":tab === 'reports' ? 0 : -1,"aria-selected":tab === 'reports',"onclick":() => router.go('insights'),});tab === 'reports';  }
           { svelteHTML.createElement("button", {        "role":`tab`,"tabindex":tab === 'health' ? 0 : -1,"aria-selected":tab === 'health',"onclick":() => router.go('insights/health'),});tab === 'health';  }
         }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(tab === 'health'){
         { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-icon":`download`}),"onclick":downloadBundle,"disabled":bundleLoading,"title":`Download a redacted support bundle (secrets stripped)`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}
          bundleLoading ? 'Preparing…' : 'Download support bundle';
         }
      } else if (!detailOnly){
         { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`gear`}),...__sveltets_2_empty({"data-label":`Schedule settings`}),"onclick":() => router.go('settings/insights'),"aria-label":`Schedule settings`,"title":`Schedule settings`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":14,}});}
         }
        if(canRun && (reports.length > 0 || loading)){
           { svelteHTML.createElement("select", {         "class":`input run-period`,"bind:value":runChoice,"disabled":loading || !!loadError || running || !!pollRunId,"aria-label":`Period to report on`,"title":`Period to report on`,});/*Ωignore_startΩ*/() => runChoice = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let o of __sveltets_2_ensureArray(RUN_OPTIONS)){o.value; { svelteHTML.createElement("option", { "value":o.value,});o.label; }}
           }
           { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":loading || !!loadError || running || !!pollRunId,"onclick":() => runNow(),});
             { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}
            running ? 'Starting…' : pollRunId ? 'Running…' : 'Run now';
           }
        }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
    
   PageHeader}

  
   { svelteHTML.createElement("div", {   "class":`sr-only`,"role":`status`,});pollRunId ? 'Generating the report' : ''; }
  if(tab === 'health'){
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
       { const $$_egaPseitilibapaC2C = __sveltets_2_ensureComponent(CapabilitiesPage); new $$_egaPseitilibapaC2C({ target: __sveltets_2_any(), props: {}});}
     PageBody}
  }else{
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
      if(runFailReason || pollRunId){
         { svelteHTML.createElement("div", { "class":`banners`,});
          if(runFailReason){
             { svelteHTML.createElement("div", { "class":`banner warn`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
               { svelteHTML.createElement("div", { "class":`banner-body`,});
                 { svelteHTML.createElement("strong", {});runFailKind === 'ended' ? 'The insights run didn’t publish a report.' : 'Couldn’t start the insights run.'; }
                 { svelteHTML.createElement("span", {});runFailReason; }
               }
              if(runFailKind === 'ended' && endedRunId){
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => endedRunId && ws.navigateToSession(endedRunId),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}    }
              }else{
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('settings/skills'),});   }
              }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => { runFailReason = null; runFailKind = null; endedRunId = null; },"aria-label":`Dismiss`,"title":`Dismiss`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
             }
          }
          if(pollRunId){
            
             { svelteHTML.createElement("div", { "class":`banner`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
               { svelteHTML.createElement("span", { "class":`banner-text`,});                        }
               { svelteHTML.createElement("span", {   "class":`dim`,"aria-hidden":`true`,});Math.floor(pollElapsed / 60); String(pollElapsed % 60).padStart(2, '0');  }
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => pollRunId && ws.navigateToSession(pollRunId),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}    }
              if(canRun){ { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":stoppingRun,"aria-busy":stoppingRun,"onclick":() => void stopRun(),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} stoppingRun ? 'Stopping…' : 'Stop'; }}
             }
          }
         }
      }

      if(loadError && reports.length > 0){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`insight reports`,"variant":`compact`,"error":loadError,"empty":true,"onretry":load,}});}
      }
      if(loading && reports.length === 0){
         { svelteHTML.createElement("div", { "class":`split`,});
          
           { svelteHTML.createElement("aside", { "class":`list-pane`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":6,"height":84,"label":`insight reports`,}});} }
           { svelteHTML.createElement("section", {  "class":`detail-pane`,});viewport.isPhone; { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":4,"height":36,"announce":false,}});} }
         }
      } else if (loadError && reports.length === 0){
        
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         "what":`insight reports`,"variant":`page`,loading,"error":loadError,"empty":true,"onretry":load,}});}
      } else if (reports.length === 0){
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`gauge`,"title":`No insight reports yet`,"body":`An agent reads your recent sessions and writes an action-first report: what’s working, what’s slowing you down, and five things to change. ${emptySchedule.text}`.trim(),"actionLabel":canRun ? (running ? 'Starting…' : "Run yesterday’s report") : undefined,"actionIcon":`play`,"onaction":canRun ? () => runNow('day:1') : undefined,}});
           { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => router.go('settings/insights'),});emptySchedule.offerTurnOn ? 'Turn on scheduled reports' : 'Schedule settings'; }
         EmptyState}
      }else{
         { svelteHTML.createElement("div", {  "class":`split`,});detailOnly;
          if(!detailOnly){
             { svelteHTML.createElement("aside", {      "class":`list-pane`,"aria-label":`Reports`,"style":`--list-pane-w:${listW}px`,});viewport.isPhone && phoneDetail;
               { svelteHTML.createElement("div", {     "class":`filters`,"role":`group`,"aria-label":`Filter by period`,});
                   for(let k of __sveltets_2_ensureArray(visibleKinds)){k.id;
                   { svelteHTML.createElement("button", {      "class":`filter-chip`,"aria-pressed":filter === k.id,"onclick":() => { filter = k.id; if (routeKey) router.replace('insights'); },});filter === k.id;
                    k.label;  { svelteHTML.createElement("span", { "class":`count`,});counts[k.id] ?? 0; }
                   }
                }
               }
               { svelteHTML.createElement("div", {   "class":`list`,...__sveltets_2_empty({"data-testid":`report-list`}),});
                   for(let r of __sveltets_2_ensureArray(shownRows)){keyOf(r);
                  const p = parsedByKey.get(keyOf(r));
                  const acts = actionSummary(r);
                  const stats = rowStats(r);
                   { svelteHTML.createElement("button", {      "class":`rep-row`,"aria-current":keyOf(r) === selectedKey ? 'true' : undefined,"onclick":() => select(r),});keyOf(r) === selectedKey;
                     { svelteHTML.createElement("div", { "class":`row-top`,});
                       { svelteHTML.createElement("span", { "class":`chip`,});kindLabel(r.kind); }
                       { svelteHTML.createElement("span", { "class":`row-date`,});periodShort(r); }
                       { svelteHTML.createElement("span", { "class":`grow`,}); }
                      if(r.created_at){ { svelteHTML.createElement("span", {   "class":`dim row-rel`,"title":new Date(r.created_at).toLocaleString(),});rel(r.created_at); }}
                     }
                    if(p?.headline){
                       { svelteHTML.createElement("div", { "class":`row-headline`,});p.headline; }
                    } else if (!r.summary.trim() && !r.html_path){
                       { svelteHTML.createElement("div", { "class":`row-headline dim`,});           }
                    }
                    if(stats.length > 0){
                       { svelteHTML.createElement("div", { "class":`row-stats`,});
                           for(let s of __sveltets_2_ensureArray(stats)){s.key;
                           { svelteHTML.createElement("span", {   "class":`stat ${s.d?.tone ?? 'neutral'}`,"title":statTitle(s.key, s.value),});
                            statText(s.key, s.value);
                            if(s.d && s.d.direction !== 'flat'){ { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":s.d.direction === 'up' ? 'arrowUp' : 'arrowDown',"size":12,}});}}
                           }
                        }
                       }
                    }
                    if(acts.total > 0){
                       { svelteHTML.createElement("div", { "class":`row-actions`,});
                         { svelteHTML.createElement("span", { "class":`dim`,});plural(acts.total, 'action'); }
                        if(acts.regressed){ { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {      "variant":`text`,"tone":`warning`,"label":`${acts.regressed} regressed`,}});}}
                        if(acts.improved){ { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {      "variant":`text`,"tone":`success`,"label":`${acts.improved} improved`,}});}}
                        if(acts.fresh){ { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {      "variant":`text`,"tone":`info`,"label":`${acts.fresh} new`,}});}}
                       }
                    }
                   }
                }
                   { svelteHTML.createElement("p", { "class":`dim list-empty`,}); filter === 'adhoc' ? 'ad-hoc' : filter;   { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (filter = 'all'),});  } }
                
                if(filtered.length > shownRows.length || hasMoreReports){
                   { svelteHTML.createElement("button", {     "class":`btn small ghost more-reports`,"onclick":loadOlderReports,"disabled":loadingMore,});
                    loadingMore ? 'Loading older reports…' : 'Show older reports';
                   }
                }
               }
             }
            if(!viewport.isPhone){
               { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`insights.listW`,"label":`Resize the reports list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
            }
          }
           { svelteHTML.createElement("section", {  "class":`detail-pane`,});viewport.isPhone && !phoneDetail && !detailOnly;
            if(selected){
              const parsed = parsedByKey.get(keyOf(selected)) ?? parseSummaryCached(selected.summary);
              const prev = previousOf(selected);
               { const $$_liateDtropeR4C = __sveltets_2_ensureComponent(ReportDetail); new $$_liateDtropeR4C({ target: __sveltets_2_any(), props: {                    "report":selected,parsed,"markdown":markdownOf(selected),kpis,"compareLabel":prev ? periodShort(prev) : null,"ledger":index.ledger,mode,"onmode":setMode,"onopenwindow":() => openWindow(selected),"onexportmd":() => exportMd(selected),"ondownloadhtml":() => downloadHtml(selected),}});}
            } else if (routeKey){
               { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`gauge`,"title":`This report isn’t on disk`,"body":`It may have been removed from the insights folder. Open the list to pick another period.`,}});
                 { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => router.go('insights'),});   }
               EmptyState}
            }
           }
         }
      }
     PageBody}
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const InsightsPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type InsightsPage__SvelteComponent_ = ReturnType<typeof InsightsPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default InsightsPage__SvelteComponent_;