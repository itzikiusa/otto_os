// Usage & metrics API: types mirroring otto-usage's DTOs, plus a small reactive
// store the Usage dashboard reads. Root-only endpoints (the daemon aggregates
// across every workspace).

import { api } from './client';
import { toasts } from '../toast.svelte';
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
  metrics: MetricPoint[] = $state([]);
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

  // --- Auto-refresh (opt-in) -----------------------------------------------
  /** Whether the dashboard should auto-refresh the full summary on a timer. */
  autoRefresh = $state(false);
  private autoRefreshTimer: ReturnType<typeof setInterval> | null = null;
  /** Default auto-refresh cadence in ms (mirrors the Brokers panel pattern). */
  static readonly AUTO_REFRESH_MS = 60_000;

  // --- Metrics-tick throttle -----------------------------------------------
  private lastMetricsFetch = 0;
  private metricsFetching = false;

  /** Query string shared by every summary fetch (window + scope). */
  private summaryQuery(): string {
    return `days=${this.days}&otto_only=${this.ottoOnly}`;
  }

  async loadStatus(): Promise<void> {
    try {
      this.status = await api.get<UsageStatus>('/usage/status');
    } catch (e) {
      toasts.error('Could not load usage status', errMsg(e));
    }
  }

  /** Load status + (when available) summary + metrics for the current window. */
  async loadAll(): Promise<void> {
    this.loading = true;
    try {
      await this.loadStatus();
      if (this.status?.available) {
        const [summary, metrics] = await Promise.all([
          api.get<UsageSummary>(`/usage/summary?${this.summaryQuery()}`),
          api.get<MetricPoint[]>('/usage/metrics?minutes=180'),
        ]);
        this.summary = summary;
        this.metrics = metrics;
        this.lastMetricsFetch = Date.now();
      } else {
        this.summary = null;
        this.metrics = [];
      }
      // Budgets are config (not engine) data — load them whether or not the
      // engine is available so the caps are still editable.
      await this.loadBudgets();
    } catch (e) {
      toasts.error('Could not load usage', errMsg(e));
    } finally {
      this.loading = false;
    }
  }

  /** Load the budget config + live spend status (root-only). */
  async loadBudgets(): Promise<void> {
    try {
      this.budgets = await api.get<UsageBudgetStatus>('/usage/budgets');
    } catch (e) {
      // Non-fatal: the dashboard still renders without budgets.
      toasts.error('Could not load usage budgets', errMsg(e));
    }
  }

  /** Persist the budget config and refresh the status. */
  async saveBudgets(cfg: UsageBudgetConfig): Promise<void> {
    this.savingBudgets = true;
    try {
      this.budgets = await api.put<UsageBudgetStatus>('/usage/budgets', cfg);
      toasts.success('Budgets saved');
    } catch (e) {
      toasts.error('Could not save budgets', errMsg(e));
    } finally {
      this.savingBudgets = false;
    }
  }

  async setDays(days: number): Promise<void> {
    this.days = days;
    await this.refreshSummary();
  }

  /** Toggle the Otto-only vs all-sessions view and reload. */
  async setOttoOnly(ottoOnly: boolean): Promise<void> {
    this.ottoOnly = ottoOnly;
    await this.refreshSummary();
  }

  // --- Auto-refresh toggle (mirrors Brokers pattern) -----------------------

  /** Turn opt-in auto-refresh on or off. When on, fires every `AUTO_REFRESH_MS`. */
  setAutoRefresh(on: boolean): void {
    this.autoRefresh = on;
    if (on) {
      if (!this.autoRefreshTimer) {
        this.autoRefreshTimer = setInterval(() => {
          void this.loadAll();
        }, UsageStore.AUTO_REFRESH_MS);
      }
    } else {
      if (this.autoRefreshTimer) {
        clearInterval(this.autoRefreshTimer);
        this.autoRefreshTimer = null;
      }
    }
  }

  // --- WS event handler (usage_metrics_tick) --------------------------------

  /** Called by the events client when a `usage_metrics_tick` WS event arrives.
   *  Refreshes the metrics sparkline in near-real-time; throttled so a burst of
   *  ticks doesn't hammer the API. Kept as a capped fallback even if ticks stop. */
  applyMetricsTick(): void {
    if (!this.status?.available) return;
    const now = Date.now();
    if (now - this.lastMetricsFetch < METRICS_REFRESH_THROTTLE_MS) return;
    if (this.metricsFetching) return;
    this.metricsFetching = true;
    this.lastMetricsFetch = now;
    api
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
    try {
      this.summary = await api.get<UsageSummary>(`/usage/summary?${this.summaryQuery()}`);
    } catch (e) {
      toasts.error('Could not load usage', errMsg(e));
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
