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

const DOW_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const DOW_PLURAL = ['Sundays', 'Mondays', 'Tuesdays', 'Wednesdays', 'Thursdays', 'Fridays', 'Saturdays'];

/** Cron day-of-week field → sorted days (0 = Sun; 7 folds to 0), or null. */
function cronDays(f: string): number[] | null {
  const days = new Set<number>();
  for (const part of f.split(',')) {
    const m = /^(\d)(?:-(\d))?$/.exec(part);
    if (!m) return null;
    const a = Number(m[1]);
    const b = m[2] === undefined ? a : Number(m[2]);
    if (a > 7 || b > 7 || b < a) return null;
    for (let d = a; d <= b; d++) days.add(d % 7);
  }
  return [...days].sort((x, y) => x - y);
}

/** Days as people say them: "Weekdays", "Sun–Thu", "Mondays", "Mon, Wed, Fri". */
function daysLabel(days: number[]): string {
  const key = days.join('');
  if (days.length === 7) return 'Every day';
  if (key === '12345') return 'Weekdays';
  if (key === '06') return 'Weekends';
  if (days.length === 1) return DOW_PLURAL[days[0]];
  // A contiguous run, wrapping past Saturday (Sun–Thu is 0..4; Fri–Sun wraps).
  for (let start = 0; start < 7; start++) {
    const run = days.map((_, i) => (start + i) % 7);
    if (run.every((d) => days.includes(d)) && days.length > 2) {
      return `${DOW_SHORT[run[0]]}–${DOW_SHORT[run[run.length - 1]]}`;
    }
  }
  return days.map((d) => DOW_SHORT[d]).join(', ');
}

/** A common cron in words ("Sun–Thu at 09:00", "Every day at 07:30",
 *  "Monthly on day 1 at 08:00", "Every hour at :15"); null when the
 *  expression is anything less ordinary — callers then show it raw. */
export function cronHuman(expr: string): string | null {
  const f = expr.trim().split(/\s+/);
  if (f.length !== 5) return null;
  const [mi, hr, dom, mon, dow] = f;
  if (!/^\d{1,2}$/.test(mi) || Number(mi) > 59 || mon !== '*') return null;
  const mm = mi.padStart(2, '0');
  if (hr === '*' && dom === '*' && dow === '*') return `Every hour at :${mm}`;
  if (!/^\d{1,2}$/.test(hr) || Number(hr) > 23) return null;
  const at = `${hr.padStart(2, '0')}:${mm}`;
  if (dom === '*' && dow === '*') return `Every day at ${at}`;
  if (dom === '*') {
    const days = cronDays(dow);
    return days && days.length ? `${daysLabel(days)} at ${at}` : null;
  }
  if (/^\d{1,2}$/.test(dom) && Number(dom) >= 1 && Number(dom) <= 31 && dow === '*') {
    return `Monthly on day ${Number(dom)} at ${at}`;
  }
  return null;
}

export interface CadenceLabelOptions {
  /** Include a cron schedule's expression and timezone ("Cron 0 9 * * 1 · UTC").
   *  False where the row renders the expression itself, in mono. Default true. */
  cronExpr?: boolean;
  /** Say a common cron in words ("Sun–Thu at 09:00 · UTC", [`cronHuman`]);
   *  an unusual one still reads "Cron <expr> · <tz>". Default false. */
  cronWords?: boolean;
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
    const words = opts.cronWords && expr ? cronHuman(expr) : null;
    if (words) return `${words} · ${tz}`;
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
