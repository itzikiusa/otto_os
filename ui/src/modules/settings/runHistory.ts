// Settings → Backup & restore → Database storage → "Run history" (perf N8).
// Run-history retention (`data_retention.run_history_days`) is OPT-IN: `0`
// keeps finished runs forever. The daemon stores `data_retention` as one
// object and a partial object merges over the DEFAULTS (not over what is
// stored), so the control must write the whole stored object back with only
// `run_history_days` changed — otherwise other customized windows reset.

/** Choices offered (days; `0` = keep forever, the default). The daemon floors
 *  any positive value at 14 days. */
export const RUN_HISTORY_CHOICES: readonly number[] = [0, 14, 30, 60, 90, 180, 365];

/** Smallest window the daemon accepts (it floors smaller positives). */
export const RUN_HISTORY_MIN_DAYS = 14;

/** The effective window from `GET /settings` (`0` = off). Mirrors the
 *  daemon's clamp: ≤0 or non-numeric → 0; positive → at least 14. */
export function runHistoryDays(settings: Record<string, unknown> | null | undefined): number {
  const r = settings?.data_retention;
  const raw = r && typeof r === 'object' ? (r as Record<string, unknown>).run_history_days : undefined;
  const n = typeof raw === 'number' && Number.isFinite(raw) ? Math.trunc(raw) : 0;
  return n <= 0 ? 0 : Math.max(n, RUN_HISTORY_MIN_DAYS);
}

/** The `data_retention` object to PUT: the stored one with only
 *  `run_history_days` replaced. */
export function withRunHistoryDays(
  settings: Record<string, unknown> | null | undefined,
  days: number,
): Record<string, unknown> {
  const r = settings?.data_retention;
  const base = r && typeof r === 'object' && !Array.isArray(r) ? (r as Record<string, unknown>) : {};
  return { ...base, run_history_days: Math.max(0, Math.trunc(days)) };
}

/** Human label for a window. */
export function runHistoryLabel(days: number): string {
  return days <= 0 ? 'Keep forever (default)' : `${days} days`;
}
