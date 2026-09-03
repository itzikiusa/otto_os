// Shared shapes + human formatting for `MetricChart.svelte` and any stat row
// that sits next to it (AWS CloudWatch cards today). Kept out of the component
// so callers can format values without instantiating a chart.

export type MetricChartUnit =
  | 'count'
  | 'bytes'
  | 'percent'
  | 'seconds'
  | 'ms'
  | 'count_per_sec'
  | 'bytes_per_sec';

export interface MetricChartPoint {
  /** Epoch ms. */
  t: number;
  /** `null` = gap (the line breaks there). */
  v: number | null;
}

export interface MetricChartSeries {
  label: string;
  points: MetricChartPoint[];
  color?: string;
}

function trim(n: number, digits: number): string {
  return n.toLocaleString(undefined, { maximumFractionDigits: digits });
}

export function formatBytes(n: number): string {
  const a = Math.abs(n);
  if (a < 1024) return `${trim(n, 0)} B`;
  const u = ['KB', 'MB', 'GB', 'TB', 'PB'];
  let v = n / 1024;
  let i = 0;
  while (Math.abs(v) >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${trim(v, Math.abs(v) >= 100 ? 0 : 1)} ${u[i]}`;
}

export function formatCount(n: number): string {
  const a = Math.abs(n);
  if (a >= 1e9) return `${trim(n / 1e9, 1)}B`;
  if (a >= 1e6) return `${trim(n / 1e6, 1)}M`;
  if (a >= 1e3) return `${trim(n / 1e3, 1)}k`;
  return trim(n, a < 10 && !Number.isInteger(n) ? 2 : 0);
}

export function formatSeconds(s: number): string {
  const a = Math.abs(s);
  if (a === 0) return '0 s';
  if (a < 0.001) return `${trim(s * 1e6, 0)} µs`;
  if (a < 1) return `${trim(s * 1000, a < 0.01 ? 2 : 0)} ms`;
  if (a < 60) return `${trim(s, a < 10 ? 2 : 1)} s`;
  if (a < 3600) return `${trim(s / 60, 1)} min`;
  if (a < 86400) return `${trim(s / 3600, 1)} h`;
  return `${trim(s / 86400, 1)} d`;
}

/** Human value in the series' unit; `—` for null / non-finite. */
export function formatMetric(v: number | null | undefined, unit: MetricChartUnit): string {
  if (v == null || !Number.isFinite(v)) return '—';
  switch (unit) {
    case 'bytes':
      return formatBytes(v);
    case 'bytes_per_sec':
      return `${formatBytes(v)}/s`;
    case 'percent':
      return `${trim(v, Math.abs(v) < 10 ? 1 : 0)}%`;
    case 'seconds':
      return formatSeconds(v);
    case 'ms':
      return formatSeconds(v / 1000);
    case 'count_per_sec':
      return `${formatCount(v)}/s`;
    default:
      return formatCount(v);
  }
}

/** Short axis-tick label for a time, chosen by the visible span. */
export function formatTimeTick(t: number, spanMs: number): string {
  const d = new Date(t);
  const p = (n: number) => String(n).padStart(2, '0');
  if (spanMs > 3 * 86_400_000) return `${p(d.getMonth() + 1)}/${p(d.getDate())}`;
  if (spanMs > 86_400_000) return `${p(d.getDate())} ${p(d.getHours())}:00`;
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}
