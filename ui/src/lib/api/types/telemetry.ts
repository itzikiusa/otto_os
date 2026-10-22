// Contract mirror: telemetry. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

// Application telemetry (independent of coding-agent tokens and cost).
export interface TelemetryConfig {
  enabled: boolean;
  native_profiling: boolean;
  traces_days: number;
  logs_days: number;
  metrics_days: number;
  analysis_interval_hours: number;
  analysis_window_hours: number;
  suggestion_limit: number;
  slow_threshold_ms: number;
  min_samples: number;
  sample_interval_secs: number;
  cpu_spike_percent: number;
  rss_spike_mb: number;
}
export interface TelemetryStatus {
  enabled: boolean;
  collector_ready: boolean;
  collector_version: string;
  queued: number;
  /** Per-minute resource maxima + spike/profile logs awaiting the next flush. */
  buffered_samples: number;
  dropped: number;
  exported: number;
  /** Records the collector's ClickHouse exporters gave up on (cumulative). */
  collector_send_failed: number;
  /** Exporter queue depth at the end of the last flush. */
  collector_queue_size: number;
  /** Last successful flush (unix seconds). */
  last_flush_at: number | null;
  last_error: string | null;
  last_analysis_at: number | null;
  next_analysis_at: number | null;
  native_profiling_supported: boolean;
}
export interface TelemetryOperation {
  component: string; name: string; count: number; errors: number;
  p50_ms: number; p95_ms: number; max_ms: number; total_ms: number; trace_id: string;
}
export interface TelemetryResource {
  timestamp: number; process: string; cpu_percent: number | null;
  rss_mb: number | null; host_load: number | null;
}
export interface TelemetryOverview {
  hours: number; operations: TelemetryOperation[];
  resources: TelemetryResource[]; status: TelemetryStatus;
}
export interface TelemetrySuggestion extends Omit<TelemetryOperation, 'errors'> {
  id: string; kind: string; threshold_ms: number; window_hours: number;
  /** Exclusive time over the window (the ranking key); null when unmeasured. */
  self_ms: number | null;
  observed_at: number; action: string; dismissed: boolean;
  peak_cpu_percent: number | null; peak_rss_mb: number | null;
}
export interface TelemetrySpan {
  trace_id: string; span_id: string; parent_span_id: string | null;
  name: string; component: string; kind: string; start_unix_nano: number;
  duration_ms: number; status: string; attributes: Record<string, unknown>;
}
export interface TelemetryProfile {
  captured_at: number; duration_seconds: number; format: string; frames: string[];
}
