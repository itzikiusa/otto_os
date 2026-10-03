// API client storage gauge (perf2 N2): when the History list suggests a
// retention limit, and how sizes read. Retention stays opt-in — this only
// decides whether to OFFER presets; nothing is deleted without a click.
import type { ApiClientStorage } from '../../lib/api/types';

/** Offer presets past any of these (history rows / bytes, runs of one automation). */
export const SUGGEST_HISTORY_ROWS = 5_000;
export const SUGGEST_HISTORY_BYTES = 50 * 1024 * 1024;
export const SUGGEST_RUNS_PER_AUTOMATION = 200;

export interface RetentionPreset {
  label: string;
  /** Settings written: history rows / days (0 = no limit); runs kept per automation (null = unchanged). */
  rows: number;
  days: number;
  runsKeep: number | null;
}

export const RETENTION_PRESETS: readonly RetentionPreset[] = [
  { label: 'Keep 1,000', rows: 1_000, days: 0, runsKeep: 200 },
  { label: 'Keep 5,000', rows: 5_000, days: 0, runsKeep: 200 },
  { label: 'Keep 30 days', rows: 0, days: 30, runsKeep: 200 },
];

/** Whether the History list should show the size banner: big, and no limit
 *  configured for the part that is big. */
export function suggestRetention(
  s: ApiClientStorage | null,
  limits: { rows: number; days: number; runsKeep: number },
): boolean {
  if (!s) return false;
  const historyBig = s.history_rows > SUGGEST_HISTORY_ROWS || s.history_bytes > SUGGEST_HISTORY_BYTES;
  const runsBig = s.max_runs_per_automation > SUGGEST_RUNS_PER_AUTOMATION;
  return (historyBig && !limits.rows && !limits.days) || (runsBig && !limits.runsKeep);
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
