// One humane label for a stored schedule (`{cadence:'interval'|'daily'|'weekly'|
// 'cron'|'once', …}`) — the shape scheduled tasks, personal agents and workflow
// triggers share. content.md §6: cadences read as people say them ("Every day",
// "Mondays at 09:00 · UTC"), never "every 1440 min"; §7: no local formatters.

const WEEKDAY_PLURAL = ['Mondays', 'Tuesdays', 'Wednesdays', 'Thursdays', 'Fridays', 'Saturdays', 'Sundays'];

/** A `once` schedule's `run_at` as a `datetime-local` value in `tz`: a local
 *  wall time passes through; an RFC3339 instant is shown in the timezone. */
export function runAtLocal(raw: string, tz: string): string {
  const local = raw.trim().replace(' ', 'T');
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2})?$/.test(local)) return local.slice(0, 16);
  const d = new Date(raw);
  if (Number.isNaN(d.getTime())) return '';
  try {
    const parts = Object.fromEntries(
      new Intl.DateTimeFormat('en-CA', {
        timeZone: tz || 'UTC', year: 'numeric', month: '2-digit', day: '2-digit',
        hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
      }).formatToParts(d).map((p) => [p.type, p.value]),
    );
    return `${parts.year}-${parts.month}-${parts.day}T${parts.hour}:${parts.minute}`;
  } catch {
    return d.toISOString().slice(0, 16);
  }
}

/** An interval in words: "Every day", "Every 2 hours", "Every 15 min". */
export function everyLabel(min: number): string {
  if (min > 0 && min % 1440 === 0) return min === 1440 ? 'Every day' : `Every ${min / 1440} days`;
  if (min > 0 && min % 60 === 0) return min === 60 ? 'Every hour' : `Every ${min / 60} hours`;
  return `Every ${min} min`;
}

export interface CadenceLabelOptions {
  /** Include a cron schedule's expression and timezone ("Cron 0 9 * * 1 · UTC").
   *  False where the row renders the expression itself, in mono. Default true. */
  cronExpr?: boolean;
}

/** Human cadence for a stored schedule ("Daily at 03:00 · Europe/London"). */
export function cadenceLabel(
  schedule: Record<string, unknown> | null | undefined,
  timezone: string | null | undefined,
  opts: CadenceLabelOptions = {},
): string {
  const s = schedule ?? {};
  const c = (s.cadence as string) ?? 'interval';
  const tz = timezone || 'UTC';
  if (c === 'interval') return everyLabel(Number(s.every_min ?? 60));
  if (c === 'cron') {
    const expr = typeof s.expr === 'string' ? s.expr.trim() : '';
    return opts.cronExpr === false || !expr ? 'Cron' : `Cron ${expr} · ${tz}`;
  }
  if (c === 'once') {
    const at = typeof s.run_at === 'string' ? runAtLocal(s.run_at, tz).replace('T', ' ') : '';
    return at ? `Once at ${at} · ${tz}` : 'Once';
  }
  const at = (s.at as string) ?? '09:00';
  if (c === 'daily') return `Daily at ${at} · ${tz}`;
  return `${WEEKDAY_PLURAL[Number(s.weekday ?? 0)] ?? WEEKDAY_PLURAL[0]} at ${at} · ${tz}`;
}
