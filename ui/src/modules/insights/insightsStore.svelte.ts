import { insightsApi } from '../../lib/api/insights';
import type { InsightKind, InsightReport, InsightRunPeriod, InsightsConfig } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { loadErrorText } from '../../lib/loadError';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { capabilitiesApi, type ModuleCapability } from './capabilities';
import { indexPathFrom, parseIndex, periodKey, type InsightsIndex } from './insightsParse';
import { runPollDelay } from './runPoll';

export type InsightsFilter = 'all' | InsightKind;
export const INSIGHTS_LIST_PAGE = 200;

function localReportKey(period: InsightRunPeriod, offset: number): string {
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  if (period === 'day') start.setDate(start.getDate() - offset);
  else if (period === 'week') start.setDate(start.getDate() - ((start.getDay() + 6) % 7) - offset * 7);
  else {
    start.setDate(1);
    start.setMonth(start.getMonth() - offset);
  }
  const end = new Date(start);
  if (period === 'week') end.setDate(end.getDate() + 6);
  else if (period === 'month') {
    end.setMonth(end.getMonth() + 1);
    end.setDate(0);
  }
  const ymd = (d: Date) =>
    `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, '0')}${String(d.getDate()).padStart(2, '0')}`;
  return `${period === 'day' ? 'daily' : period === 'week' ? 'weekly' : 'monthly'}:${ymd(start)}_${ymd(end)}`;
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

class CapabilitiesStore {
  caps: ModuleCapability[] = $state([]);
  loading = $state(true);
  loadedOnce = $state(false);
  loadErr = $state('');
  expanded = $state<Set<string>>(new Set());

  async load(force = false): Promise<void> {
    if (force || !this.loadedOnce) this.loading = true;
    this.loadErr = '';
    try {
      this.caps = await capabilitiesApi.list();
      this.loadedOnce = true;
    } catch (e) {
      this.loadErr = loadErrorText(e);
    } finally {
      this.loading = false;
    }
  }

  toggle(feature: string): void {
    const next = new Set(this.expanded);
    if (next.has(feature)) next.delete(feature);
    else next.add(feature);
    this.expanded = next;
  }

  resetForTests(): void {
    this.caps = [];
    this.loading = true;
    this.loadedOnce = false;
    this.loadErr = '';
    this.expanded = new Set();
  }
}

class InsightsStore {
  reports: InsightReport[] = $state([]);
  loading = $state(true);
  loadedOnce = $state(false);
  loadError: string | null = $state(null);
  hasMoreReports = $state(false);
  archiveOffset = 0;
  loadingMore = $state(false);
  index: InsightsIndex = $state({ series: [], ledger: new Map() });
  fullMd: Record<string, string> = $state({});
  schedule = $state<InsightsConfig | null>(null);

  filter: InsightsFilter = $state('all');
  selectedKey: string | null = $state(null);
  listShown = $state(INSIGHTS_LIST_PAGE);

  runChoice = $state('day:1');
  running = $state(false);
  runFailKind = $state<'start' | 'ended' | null>(null);
  runFailReason: string | null = $state(null);
  endedRunId: string | null = $state(null);
  pollRunId: string | null = $state(null);
  pollCount = $state(0);
  pollStartedAt = $state(Date.now());
  stoppingRun = $state(false);

  private pollReportKey: string | null = null;
  private reportBeforeRun: string | null | undefined;
  private pollTimer: ReturnType<typeof setTimeout> | null = null;
  private pollGoneTicks = 0;
  private indexRequest = 0;

  setFilter(next: InsightsFilter): void {
    if (this.filter === next) return;
    this.filter = next;
    this.listShown = INSIGHTS_LIST_PAGE;
  }

  async load(routeKey: string | null = null, forceSpinner = false): Promise<void> {
    if (forceSpinner || !this.loadedOnce) this.loading = true;
    this.loadError = null;
    void insightsApi
      .getConfig()
      .then((c) => {
        this.schedule = c;
      })
      .catch(() => {
        this.schedule = null;
      });
    try {
      const next = await insightsApi.listReports();
      this.reports = next;
      this.archiveOffset = next.length;
      this.hasMoreReports = next.length === 200;
      if (routeKey && !this.reports.some((r) => periodKey(r) === routeKey)) {
        const linked = await insightsApi.reportStatus(routeKey);
        if (linked.report) this.reports = [...this.reports, linked.report];
      }
      this.loadedOnce = true;
      void this.loadIndex();
    } catch (e) {
      this.loadError = loadErrorText(e);
    } finally {
      this.loading = false;
    }
  }

  mount(routeKey: string | null): void {
    if (!this.loadedOnce) {
      void this.load(routeKey, true);
      void this.restoreActiveRun();
      return;
    }
    if (routeKey && !this.reports.some((r) => periodKey(r) === routeKey)) {
      void this.load(routeKey, false);
    }
    if (!this.pollRunId) void this.restoreActiveRun();
  }

  async loadOlderReports(shownLen: number, filteredLen: number): Promise<void> {
    if (this.loadingMore) return;
    if (shownLen < filteredLen) {
      this.listShown = shownLen + INSIGHTS_LIST_PAGE;
      return;
    }
    this.loadingMore = true;
    try {
      const page = await insightsApi.listReports(this.archiveOffset);
      this.archiveOffset += page.length;
      this.hasMoreReports = page.length === 200;
      const ids = new Set(this.reports.map((r) => periodKey(r)));
      this.reports = [...this.reports, ...page.filter((r) => !ids.has(periodKey(r)))];
      this.listShown += INSIGHTS_LIST_PAGE;
    } catch (e) {
      toasts.error('Couldn’t load older reports', loadErrorText(e));
    } finally {
      this.loadingMore = false;
    }
  }

  async loadIndex(): Promise<void> {
    const request = ++this.indexRequest;
    const html = this.reports.find((r) => r.html_path)?.html_path;
    const path = html ? indexPathFrom(html) : null;
    if (!path) return;
    try {
      const next = parseIndex(JSON.parse(await insightsApi.readText(path)));
      if (request === this.indexRequest) this.index = next;
    } catch {
      /* KPIs fall back to summaries */
    }
  }

  async restoreActiveRun(): Promise<void> {
    try {
      const runs = await insightsApi.activeRuns();
      const r = runs[runs.length - 1];
      if (!r || this.pollRunId) return;
      this.pollRunId = r.run_id;
      this.pollGoneTicks = 0;
      this.pollReportKey = r.report_key;
      this.reportBeforeRun = r.report_revision;
      this.pollCount = 0;
      this.pollStartedAt = Number.isFinite(Date.parse(r.started_at)) ? Date.parse(r.started_at) : Date.now();
      this.schedulePoll();
    } catch {
      /* best-effort */
    }
  }

  async stopRun(): Promise<void> {
    const id = this.pollRunId;
    if (!id || this.stoppingRun) return;
    const ok = await confirmer.ask(
      'Stop generating this insights report? The agent session is ended; nothing already published is removed.',
      { title: 'Stop insights run', danger: true, confirmLabel: 'Stop run' },
    );
    if (!ok) return;
    this.stoppingRun = true;
    try {
      await insightsApi.cancelRun(id);
      if (this.pollTimer) clearTimeout(this.pollTimer);
      this.pollTimer = null;
      this.pollRunId = null;
      toasts.info('Insights run stopped');
    } catch (e) {
      toastError('Couldn’t stop the insights run', e);
    } finally {
      this.stoppingRun = false;
    }
  }

  async runNow(canRun: boolean, choice = this.runChoice): Promise<void> {
    if (!canRun || this.loading || this.loadError || this.running || this.pollRunId) return;
    const [p, o] = choice.split(':');
    this.running = true;
    this.runFailReason = null;
    this.runFailKind = null;
    this.endedRunId = null;
    this.pollCount = 0;
    this.pollStartedAt = Date.now();
    const period = p as InsightRunPeriod;
    const offset = Number(o) || 1;
    const fallbackKey = localReportKey(period, offset);
    try {
      const resp = await insightsApi.run({ period, offset });
      if (!resp.started) {
        this.runFailKind = 'start';
        this.runFailReason = resp.reason ?? 'Check that the insights skill is installed.';
        return;
      }
      if (resp.attached) {
        toasts.info('Already generating this report', 'Showing the run in progress instead of starting another.');
      }
      if (resp.run_id) {
        this.pollRunId = resp.run_id;
        if (resp.attached) {
          void insightsApi
            .activeRuns()
            .then((runs) => {
              const active = runs.find((r) => r.run_id === resp.run_id);
              if (this.pollRunId === resp.run_id && active && Number.isFinite(Date.parse(active.started_at))) {
                this.pollStartedAt = Date.parse(active.started_at);
              }
            })
            .catch(() => {});
        }
        this.pollGoneTicks = 0;
        this.pollReportKey = resp.report_key ?? fallbackKey;
        this.reportBeforeRun = resp.report_revision;
        this.schedulePoll();
      } else {
        setTimeout(() => void this.load(null, false), 2500);
        toasts.success('Insights run started', 'The report appears here when it is ready.');
      }
    } catch (e) {
      toastError('Couldn’t start the insights run', e);
    } finally {
      this.running = false;
    }
  }

  private schedulePoll(): void {
    if (!this.pollRunId) {
      void this.load(null, false);
      return;
    }
    if (this.pollTimer) clearTimeout(this.pollTimer);
    this.pollTimer = setTimeout(async () => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
        this.schedulePoll();
        return;
      }
      this.pollCount += 1;
      let ready: InsightReport | null = null;
      try {
        if (!this.pollReportKey) {
          this.pollRunId = null;
          return;
        }
        const status = await insightsApi.reportStatus(this.pollReportKey);
        if (status.report?.html_path && status.html_revision && status.html_revision !== this.reportBeforeRun) {
          ready = status.report;
        }
      } catch {
        /* retry next tick */
      }
      if (!ready && this.pollRunId) {
        try {
          const active = await insightsApi.activeRuns();
          this.pollGoneTicks = active.some((r) => r.run_id === this.pollRunId) ? 0 : this.pollGoneTicks + 1;
        } catch {
          /* keep polling */
        }
        if (this.pollGoneTicks >= 2) {
          this.endedRunId = this.pollRunId;
          this.pollRunId = null;
          this.pollGoneTicks = 0;
          this.runFailKind = 'ended';
          this.runFailReason =
            'The insights run ended without publishing a report — it failed or was stopped. Its session shows what happened.';
          void this.load(null, false);
          return;
        }
      }
      if (ready) {
        this.pollRunId = null;
        await this.load(null, false);
        if (!this.reports.some((r) => periodKey(r) === periodKey(ready!))) {
          this.reports = [...this.reports, ready];
        }
        this.fullMd = {};
        this.setFilter('all');
        this.selectedKey = periodKey(ready);
        void this.loadIndex();
        toasts.success('Insights report ready', periodShort(ready));
      } else {
        this.schedulePoll();
      }
    }, runPollDelay(this.pollCount));
  }

  resetForTests(): void {
    if (this.pollTimer) clearTimeout(this.pollTimer);
    this.pollTimer = null;
    this.indexRequest++;
    this.reports = [];
    this.loading = true;
    this.loadedOnce = false;
    this.loadError = null;
    this.hasMoreReports = false;
    this.archiveOffset = 0;
    this.loadingMore = false;
    this.index = { series: [], ledger: new Map() };
    this.fullMd = {};
    this.schedule = null;
    this.filter = 'all';
    this.selectedKey = null;
    this.listShown = INSIGHTS_LIST_PAGE;
    this.runChoice = 'day:1';
    this.running = false;
    this.runFailKind = null;
    this.runFailReason = null;
    this.endedRunId = null;
    this.pollRunId = null;
    this.pollCount = 0;
    this.pollGoneTicks = 0;
    this.pollStartedAt = Date.now();
    this.stoppingRun = false;
  }
}

export const capabilitiesStore = new CapabilitiesStore();
export const insightsStore = new InsightsStore();
