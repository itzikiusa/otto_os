// `session_auto_archive_days` (Settings ▸ Daemon ▸ Sessions). The daemon reads
// it as a u64 (`v.as_u64()`, otto-sessions manager `auto_archive_stale`):
// absent, 0 or anything that isn't a whole non-negative number means OFF.
// Pure, so `node --test` can load it.

/** Upper bound the form accepts (~100 years); the daemon treats anything
 *  too large for a date as "never", but a typo shouldn't get that far. */
export const MAX_ARCHIVE_DAYS = 36_500;

/** The stored value as the form shows it (0 = never). */
export function archiveDaysFrom(settings: Record<string, unknown>): number {
  const v = settings['session_auto_archive_days'];
  return typeof v === 'number' && Number.isInteger(v) && v > 0 ? v : 0;
}

/** Why the typed value can't be saved, or '' when it can. */
export function archiveDaysError(v: unknown): string {
  if (typeof v !== 'number' || !Number.isInteger(v) || v < 0 || v > MAX_ARCHIVE_DAYS) {
    return `Enter a whole number of days from 0 (never) to ${MAX_ARCHIVE_DAYS.toLocaleString('en-US')}.`;
  }
  return '';
}
