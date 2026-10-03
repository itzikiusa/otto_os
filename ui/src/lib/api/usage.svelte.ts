// Usage & metrics API: types mirroring otto-usage's DTOs, plus a small reactive
// store the Usage dashboard reads. Root sees every session; a non-root caller
// holding Usage:View sees only the sessions they created (`scope: 'own'`).

import { api } from './client';
import { pollWhileVisible, type Poller } from '../poll';
import { toasts } from '../toast.svelte';
import { loadErrorText } from '../loadError';
import { exportCsv, downloadJson } from '../components/exporters';
import type { Id } from './types';

export interface ProviderUsage {
  provider: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

export interface DailyUsage {
  day: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

export interface SessionUsage {
  session_id: string;
  workspace_id: string;
  provider: string;
  /** Most-common model used by this session (any(model) from ClickHouse). */
  model: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
  last_active: string;
  /** Otto session title (pane name) — null for external sessions. */
  title: string | null;
  /** "review" | "product" | "channel" | "agent" | … — null for external. */
  kind: string | null;
  /** Human-readable workspace name — null for external. */
  workspace_name: string | null;
  /** True when cost was estimated via the Opus-tier FALLBACK (unrecognised
   *  model). The UI renders these as "estimated". */
  fallback_priced?: boolean | null;
}

/** Per-feature (by-kind) rollup: usage grouped by the kind of Otto work
 *  (review / product / channel / agent / connection / external …) rather than by
 *  provider. Built server-side by classifying each session. */
export interface FeatureUsage {
  /** "review" | "product" | "channel" | "agent" | "connection" | "external" | … */
  feature: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
  /** Distinct sessions that contributed to this bucket. */
  sessions: number;
}

export interface MetricPoint {
  ts: string;
  cpu_pct: number;
  mem_used_mb: number;
  mem_total_mb: number;
  mem_pct: number;
  load_avg_1: number;
  process_rss_mb: number;
  process_cpu_pct: number;
  active_sessions: number;
}

export interface UsageSummary {
  days: number;
  total_events: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_write_tokens: number;
  total_tokens: number;
  total_cost_usd: number;
  providers: ProviderUsage[];
  daily: DailyUsage[];
  sessions: SessionUsage[];
  /** Per-feature (by-kind) rollup — review / product / channel / agent / … */
  by_kind: FeatureUsage[];
  /** Per-(provider, model) token rollup over the window, biggest first. */
  models?: ModelUsage[];
  /** Per-(day, provider, model) token rollup — feeds the model chart. */
  daily_models?: DailyModelUsage[];
  /** "all" (root: every session) | "own" (non-root: only sessions you created). */
  scope?: 'all' | 'own';
}

/** Token rollup for one (provider, model) pair. */
export interface ModelUsage {
  provider: string;
  model: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

/** Token rollup for one (day, provider, model) triple. */
export interface DailyModelUsage {
  day: string;
  provider: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

/** Token rollup for one calendar month (`YYYY-MM`). */
export interface MonthlyUsage {
  month: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

/** Four token buckets + their sum + the estimated cost. */
export interface TokenTotals {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

/** `GET /usage/report?days=N&otto_only=B` — ccusage-style report tables. */
export interface UsageReport {
  days: number;
  generated_at: string;
  priced_as_of: string;
  scope: 'all' | 'own';
  otto_only: boolean;
  totals: TokenTotals;
  daily: DailyUsage[];
  monthly: MonthlyUsage[];
  models: ModelUsage[];
  daily_models: DailyModelUsage[];
  /** Up to 1000 sessions, biggest first (enriched like the summary's). */
  sessions: SessionUsage[];
}

export interface CcusageDiffRow {
  provider: string;
  model: string;
  ours: TokenTotals;
  theirs: TokenTotals;
}

export interface CcusageDayRow {
  day: string;
  ours: TokenTotals;
  theirs: TokenTotals;
}

/** `POST /usage/ccusage-check {days?}` (root) — runs `npx ccusage` on demand
 *  and compares it with Otto's numbers over the same dates (all sessions,
 *  external included). `ran=false` + `error` when npx is missing, ccusage
 *  failed / timed out, or its output didn't parse. */
export interface CcusageCheck {
  ran: boolean;
  command: string;
  duration_ms: number;
  error?: string | null;
  since: string;
  until: string;
  totals_ours: TokenTotals;
  totals_theirs: TokenTotals;
  rows: CcusageDiffRow[];
  daily: CcusageDayRow[];
}

export interface UsageStatus {
  available: boolean;
  enabled: boolean;
  binary: string | null;
  version: string | null;
  data_dir: string;
  retention_days: number;
  metrics_interval_secs: number;
  usage_rows: number;
  metric_rows: number;
  disk_bytes: number;
  /** Date the pricing rate table was last reconciled (YYYY-MM-DD). Used by the
   *  UI to label cost estimates as "priced as of <date>". */
  priced_as_of?: string | null;
}

export interface UsageConfigReq {
  enabled?: boolean;
  retention_days?: number;
  metrics_interval_secs?: number;
  clickhouse_path?: string;
}

// --- Usage budgets (opt-in spend caps) ------------------------------------

export interface WorkspaceBudget {
  workspace_id: Id;
  /** USD cap over the window; 0 = no cap. */
  monthly_usd: number;
}

export interface ProviderBudget {
  provider: string;
  monthly_usd: number;
}

export interface UsageBudgetConfig {
  /** Master opt-in; false = budgets are informational only (default). */
  enforce: boolean;
  /** When enforcing, true = hard block on exceed; false = warn only (default). */
  block_on_exceed: boolean;
  /** Window the caps apply to, in days (default 30). */
  window_days: number;
  workspaces: WorkspaceBudget[];
  providers: ProviderBudget[];
}

export interface BudgetStatusRow {
  /** "workspace" | "provider". */
  scope: string;
  /** Workspace id or provider name. */
  key: string;
  /** Workspace name / provider name when resolvable. */
  label: string | null;
  limit_usd: number;
  spent_usd: number;
  /** spent / limit (0 when no limit). */
  used_fraction: number;
  /** Spend crossed the 80% warn line. */
  warning: boolean;
  /** Spend met/exceeded the cap. */
  exceeded: boolean;
}

export interface UsageBudgetStatus {
  config: UsageBudgetConfig;
  window_days: number;
  rows: BudgetStatusRow[];
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Maximum time between background metrics refreshes driven by `applyMetricsTick`
 *  (capped poll fallback). Even if WS events come faster, we only re-fetch once
 *  per this window. */
const METRICS_REFRESH_THROTTLE_MS = 10_000;

class UsageStore {
  status: UsageStatus | null = $state(null);
  summary: UsageSummary | null = $state(null);
  /** Server payload, replaced wholesale on every fetch — `$state.raw`, no
   *  deep proxy over 180 points (X2). */
  metrics: MetricPoint[] = $state.raw([]);
  /** Selected look-back window for usage rollups, in days. */
  days = $state(30);
  /** When true, show only sessions that ran inside Otto (exclude the user's own
   *  Claude/codex runs, recorded as "external"). */
  ottoOnly = $state(true);
  loading = $state(false);
  installing = $state(false);
  saving = $state(false);
  /** Usage budgets (caps + live spend status). Null until loaded. */
  budgets: UsageBudgetStatus | null = $state(null);
  savingBudgets = $state(false);

  // Load failures render INLINE with Retry (never a toast, never mistaken for
  // "ClickHouse isn't installed" / "no usage" / "no budgets").
  /** `/usage/status` failed — we don't know whether the engine is installed. */
  statusError = $state<string | null>(null);
  /** Summary/metrics failed while the engine IS available. */
  summaryError = $state<string | null>(null);
  budgetsError = $state<string | null>(null);

  /** Whether the caller is root. Non-root `Usage:View` holders get their own
   *  sessions (`summary.scope === 'own'`); the root-only metrics endpoint is
   *  never requested for them. Set by the page from `auth.isRoot`. */
  admin = $state(true);

  // --- Usage report (GET /usage/report) ------------------------------------
  report: UsageReport | null = $state.raw(null);
  reportLoading = $state(false);
  reportError = $state<string | null>(null);
  private reportSeq = 0;

  // --- ccusage cross-check (POST /usage/ccusage-check, root) ----------------
  ccusage: CcusageCheck | null = $state.raw(null);
  ccusageRunning = $state(false);
  ccusageError = $state<string | null>(null);

  // --- Auto-refresh (opt-in) -----------------------------------------------
  /** Whether the dashboard should auto-refresh the full summary on a timer. */
  autoRefresh = $state(false);
  private autoRefreshPoll: Poller | null = null;
  /** Default auto-refresh cadence in ms (mirrors the Brokers panel pattern). */
  static readonly AUTO_REFRESH_MS = 60_000;

  /** Request token for summary fetches: clicking 90d then 7d must never let
   *  the slower 90-day response land last under the "7d" label (or into a
   *  `-7d` CSV export). Every summary load bumps it; only the newest writes. */
  private summarySeq = 0;

  // --- Metrics-tick throttle -----------------------------------------------
  private lastMetricsFetch = 0;
  private metricsFetching = false;
  /** Mounted readers of `metrics` (the Usage page). The `usage_metrics_tick`
   *  refetch runs only while one is mounted — it used to refetch 180 minutes of
   *  points every minute for the app's lifetime after one visit (SF-10). */
  private metricsSubscribers = 0;

  /** Register a mounted reader of `metrics`; call the returned fn on unmount. */
  watchMetrics(): () => void {
    this.metricsSubscribers += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.metricsSubscribers = Math.max(0, this.metricsSubscribers - 1);
    };
  }

  /** Query string shared by every summary fetch (window + scope). */
  private summaryQuery(): string {
    return `days=${this.days}&otto_only=${this.ottoOnly}`;
  }

  async loadStatus(): Promise<void> {
    try {
      this.status = await api.get<UsageStatus>('/usage/status');
      this.statusError = null;
    } catch (e) {
      this.statusError = loadErrorText(e);
    }
  }

  /** Load status + (when available) summary + metrics for the current window. */
  async loadAll(): Promise<void> {
    this.loading = true;
    let mine: number | null = null;
    // Budgets are config (not engine) data — load them whether or not the
    // engine is available (or the summary failed) so the caps are still
    // editable, and a skipped load never reads as "No budgets set". Started
    // now, in parallel with status → summary, instead of after them (it was
    // a third serial round trip on every page open). Never rejects.
    const budgets = this.loadBudgets();
    try {
      await this.loadStatus();
      if (this.status?.available) {
        mine = ++this.summarySeq;
        // System CPU/RAM metrics are root-only: a non-root caller never asks.
        const [summary, metrics] = await Promise.all([
          api.get<UsageSummary>(`/usage/summary?${this.summaryQuery()}`),
          this.admin ? api.get<MetricPoint[]>('/usage/metrics?minutes=180') : Promise.resolve<MetricPoint[]>([]),
        ]);
        if (mine === this.summarySeq) {
          this.summary = summary;
          this.summaryError = null;
        }
        this.metrics = metrics;
        this.lastMetricsFetch = Date.now();
        // Keep an open report in step with the refreshed window.
        if (this.report) void this.loadReport();
      } else if (this.status) {
        this.summary = null;
        this.metrics = [];
      }
    } catch (e) {
      if (mine === this.summarySeq) this.summaryError = loadErrorText(e);
    }
    try {
      await budgets;
    } finally {
      this.loading = false;
    }
  }

  /** Load the budget config + live spend status (root-only). */
  private budgetsSeq = 0;

  async loadBudgets(): Promise<void> {
    const mine = ++this.budgetsSeq;
    try {
      const next = await api.get<UsageBudgetStatus>('/usage/budgets');
      if (mine !== this.budgetsSeq || this.savingBudgets) return;
      this.budgets = next;
      this.budgetsError = null;
    } catch (e) {
      if (mine !== this.budgetsSeq || this.savingBudgets) return;
      // Non-fatal: the dashboard still renders; the Budgets panel shows why.
      this.budgetsError = loadErrorText(e);
    }
  }

  /** Persist the budget config and refresh the status. */
  async saveBudgets(cfg: UsageBudgetConfig): Promise<boolean> {
    this.savingBudgets = true;
    ++this.budgetsSeq;
    try {
      this.budgets = await api.put<UsageBudgetStatus>('/usage/budgets', cfg);
      toasts.success('Budgets saved');
      return true;
    } catch (e) {
      toasts.error('Could not save budgets', errMsg(e));
      return false;
    } finally {
      // Ignore refreshes begun before or during this mutation.
      ++this.budgetsSeq;
      this.savingBudgets = false;
    }
  }

  async setDays(days: number): Promise<void> {
    this.days = days;
    await Promise.all([this.refreshSummary(), this.report ? this.loadReport() : Promise.resolve()]);
  }

  /** Toggle the Otto-only vs all-sessions view and reload. */
  async setOttoOnly(ottoOnly: boolean): Promise<void> {
    this.ottoOnly = ottoOnly;
    await Promise.all([this.refreshSummary(), this.report ? this.loadReport() : Promise.resolve()]);
  }

  /** Load the ccusage-style report for the current window + scope. */
  async loadReport(): Promise<void> {
    const mine = ++this.reportSeq;
    this.reportLoading = true;
    try {
      const r = await api.get<UsageReport>(`/usage/report?${this.summaryQuery()}`);
      if (mine !== this.reportSeq) return;
      this.report = r;
      this.reportError = null;
    } catch (e) {
      if (mine === this.reportSeq) this.reportError = loadErrorText(e);
    } finally {
      if (mine === this.reportSeq) this.reportLoading = false;
    }
  }

  /** Opt-in: run `npx ccusage` through the daemon and compare (root only).
   *  A run that executes but fails comes back as `ran=false` + `error`; a
   *  request failure (timeout, 403…) lands in `ccusageError`. */
  async runCcusage(days: number): Promise<void> {
    if (this.ccusageRunning) return;
    this.ccusageRunning = true;
    this.ccusageError = null;
    try {
      this.ccusage = await api.post<CcusageCheck>('/usage/ccusage-check', { days });
    } catch (e) {
      this.ccusageError = loadErrorText(e);
    } finally {
      this.ccusageRunning = false;
    }
  }

  /** Export the model rollup as CSV. */
  exportModelsCsv(): void {
    const models = this.summary?.models ?? [];
    if (models.length === 0) return;
    exportCsv(
      models.map((m) => ({
        provider: m.provider,
        model: m.model,
        events: m.events,
        input_tokens: m.input_tokens,
        output_tokens: m.output_tokens,
        cache_read_tokens: m.cache_read_tokens,
        cache_write_tokens: m.cache_write_tokens,
        total_tokens: m.total_tokens,
        cost_usd: m.cost_usd,
      })),
      `otto-usage-models-${this.days}d.csv`,
    );
  }

  // --- Auto-refresh toggle (mirrors Brokers pattern) -----------------------

  /** Turn opt-in auto-refresh on or off. When on, fires every `AUTO_REFRESH_MS`. */
  setAutoRefresh(on: boolean): void {
    this.autoRefresh = on;
    if (on) {
      // Shared poll chain: no overlapping loadAll, paused while hidden.
      this.autoRefreshPoll ??= pollWhileVisible(() => this.loadAll(), {
        ms: UsageStore.AUTO_REFRESH_MS,
        immediate: false,
      });
    } else {
      this.autoRefreshPoll?.stop();
      this.autoRefreshPoll = null;
    }
  }

  // --- WS event handler (usage_metrics_tick) --------------------------------

  /** Called by the events client when a `usage_metrics_tick` WS event arrives.
   *  Refreshes the metrics sparkline in near-real-time; throttled so a burst of
   *  ticks doesn't hammer the API. Kept as a capped fallback even if ticks stop. */
  applyMetricsTick(): void {
    if (!this.status?.available || !this.admin) return;
    if (this.metricsSubscribers === 0) return;
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
    const now = Date.now();
    if (now - this.lastMetricsFetch < METRICS_REFRESH_THROTTLE_MS) return;
    if (this.metricsFetching) return;
    this.metricsFetching = true;
    this.lastMetricsFetch = now;
    api.bg
      .get<MetricPoint[]>('/usage/metrics?minutes=180')
      .then((m) => {
        this.metrics = m;
      })
      .catch(() => {
        /* silent — sparkline lag is benign */
      })
      .finally(() => {
        this.metricsFetching = false;
      });
  }

  private async refreshSummary(): Promise<void> {
    if (!this.status?.available) return;
    const mine = ++this.summarySeq;
    try {
      const summary = await api.get<UsageSummary>(`/usage/summary?${this.summaryQuery()}`);
      if (mine === this.summarySeq) {
        this.summary = summary;
        this.summaryError = null;
      }
    } catch (e) {
      if (mine === this.summarySeq) this.summaryError = loadErrorText(e);
    }
  }

  // --- Export helpers -------------------------------------------------------

  /** Export the provider rollup as CSV. */
  exportProvidersCsv(): void {
    if (!this.summary) return;
    exportCsv(
      this.summary.providers.map((p) => ({
        provider: p.provider,
        events: p.events,
        input_tokens: p.input_tokens,
        output_tokens: p.output_tokens,
        cache_read_tokens: p.cache_read_tokens,
        cache_write_tokens: p.cache_write_tokens,
        total_tokens: p.total_tokens,
        cost_usd: p.cost_usd,
      })),
      `otto-usage-providers-${this.days}d.csv`,
    );
  }

  /** Export the daily rollup as CSV. */
  exportDailyCsv(): void {
    if (!this.summary) return;
    exportCsv(
      this.summary.daily.map((d) => ({
        day: d.day,
        events: d.events,
        input_tokens: d.input_tokens,
        output_tokens: d.output_tokens,
        cache_read_tokens: d.cache_read_tokens,
        cache_write_tokens: d.cache_write_tokens,
        total_tokens: d.total_tokens,
        cost_usd: d.cost_usd,
      })),
      `otto-usage-daily-${this.days}d.csv`,
    );
  }

  /** Export the top-sessions table as CSV. */
  exportSessionsCsv(): void {
    if (!this.summary) return;
    exportCsv(
      this.summary.sessions.map((s) => ({
        session_id: s.session_id,
        title: s.title ?? '',
        kind: s.kind ?? '',
        workspace: s.workspace_name ?? '',
        provider: s.provider,
        model: s.model,
        events: s.events,
        input_tokens: s.input_tokens,
        output_tokens: s.output_tokens,
        cache_read_tokens: s.cache_read_tokens,
        cache_write_tokens: s.cache_write_tokens,
        total_tokens: s.total_tokens,
        cost_usd: s.cost_usd,
        fallback_priced: s.fallback_priced ? 'yes' : 'no',
        last_active: s.last_active,
      })),
      `otto-usage-sessions-${this.days}d.csv`,
    );
  }

  /** Export the full summary payload as JSON (for programmatic consumption). */
  exportSummaryJson(): void {
    if (!this.summary) return;
    downloadJson(this.summary, `otto-usage-summary-${this.days}d.json`);
  }

  /** Install/update ClickHouse via the official installer (large download). */
  async install(): Promise<void> {
    if (this.installing) return;
    this.installing = true;
    toasts.info('Installing ClickHouse…', 'Downloading the engine — this can take a few minutes.');
    try {
      this.status = await api.post<UsageStatus>('/usage/install', {});
      if (this.status.available) {
        toasts.success('ClickHouse ready', this.status.version ?? 'installed');
        await this.loadAll();
      } else {
        toasts.error('Install finished but engine is not available', 'Check daemon logs.');
      }
    } catch (e) {
      toasts.error('Install failed', errMsg(e));
    } finally {
      this.installing = false;
    }
  }

  async saveConfig(req: UsageConfigReq): Promise<void> {
    this.saving = true;
    try {
      this.status = await api.put<UsageStatus>('/usage/config', req);
      toasts.success('Usage settings saved');
      await this.loadAll();
    } catch (e) {
      toasts.error('Save failed', errMsg(e));
    } finally {
      this.saving = false;
    }
  }
}

export const usage = new UsageStore();
