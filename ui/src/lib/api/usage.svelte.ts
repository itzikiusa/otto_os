// Usage & metrics API: types mirroring otto-usage's DTOs, plus a small reactive
// store the Usage dashboard reads. Root-only endpoints (the daemon aggregates
// across every workspace).

import { api } from './client';
import { toasts } from '../toast.svelte';

export interface ProviderUsage {
  provider: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

export interface DailyUsage {
  day: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

export interface SessionUsage {
  session_id: string;
  workspace_id: string;
  provider: string;
  events: number;
  total_tokens: number;
  cost_usd: number;
  last_active: string;
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
  total_tokens: number;
  total_cost_usd: number;
  providers: ProviderUsage[];
  daily: DailyUsage[];
  sessions: SessionUsage[];
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
}

export interface UsageConfigReq {
  enabled?: boolean;
  retention_days?: number;
  metrics_interval_secs?: number;
  clickhouse_path?: string;
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

class UsageStore {
  status: UsageStatus | null = $state(null);
  summary: UsageSummary | null = $state(null);
  metrics: MetricPoint[] = $state([]);
  /** Selected look-back window for usage rollups, in days. */
  days = $state(30);
  loading = $state(false);
  installing = $state(false);
  saving = $state(false);

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
          api.get<UsageSummary>(`/usage/summary?days=${this.days}`),
          api.get<MetricPoint[]>('/usage/metrics?minutes=180'),
        ]);
        this.summary = summary;
        this.metrics = metrics;
      } else {
        this.summary = null;
        this.metrics = [];
      }
    } catch (e) {
      toasts.error('Could not load usage', errMsg(e));
    } finally {
      this.loading = false;
    }
  }

  async setDays(days: number): Promise<void> {
    this.days = days;
    if (this.status?.available) {
      try {
        this.summary = await api.get<UsageSummary>(`/usage/summary?days=${days}`);
      } catch (e) {
        toasts.error('Could not load usage', errMsg(e));
      }
    }
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
