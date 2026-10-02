// Cadence form helpers shared by the Schedules tab — same schedule_json shape
// as scheduled tasks (`{cadence:'interval'|'daily'|'weekly'|'cron'|'once', …}`).

export interface CadenceForm {
  cadence: 'interval' | 'daily' | 'weekly' | 'cron' | 'once';
  everyMin: number;
  at: string;
  weekday: number;
  cronExpr: string;
  /** `once`: local wall-clock `YYYY-MM-DDTHH:MM` in the schedule's timezone. */
  runAt: string;
}

export const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

export function defaultCadence(): CadenceForm {
  return { cadence: 'daily', everyMin: 60, at: '09:00', weekday: 0, cronExpr: '0 9 * * 1', runAt: '' };
}

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

/** Populate the form from a stored schedule object. A `once` (the Assistant
 *  creates them) used to load as an interval and become "every 60 min" on save. */
export function loadCadence(s: Record<string, unknown>, timezone = 'UTC'): CadenceForm {
  const cad = (s.cadence as string) ?? 'interval';
  return {
    cadence: ['daily', 'weekly', 'cron', 'once'].includes(cad) ? (cad as CadenceForm['cadence']) : 'interval',
    everyMin: (s.every_min as number) ?? 60,
    at: (s.at as string) ?? '09:00',
    weekday: (s.weekday as number) ?? 0,
    cronExpr: (s.expr as string) ?? '0 9 * * 1',
    runAt: typeof s.run_at === 'string' ? runAtLocal(s.run_at, timezone) : '',
  };
}

/** The schedule_json the daemon validates (`cadence::validate`). */
export function buildCadence(f: CadenceForm): Record<string, unknown> {
  if (f.cadence === 'interval') return { cadence: 'interval', every_min: Math.max(5, f.everyMin) };
  if (f.cadence === 'daily') return { cadence: 'daily', at: f.at };
  if (f.cadence === 'cron') return { cadence: 'cron', expr: f.cronExpr.trim() };
  if (f.cadence === 'once') return { cadence: 'once', run_at: f.runAt };
  return { cadence: 'weekly', at: f.at, weekday: f.weekday };
}

/** Human label for a stored schedule (+ timezone where it applies). */
export function cadenceLabel(s: Record<string, unknown>, timezone: string): string {
  const c = (s.cadence as string) ?? 'interval';
  const tz = timezone || 'UTC';
  if (c === 'interval') return `every ${(s.every_min as number) ?? 60} min`;
  if (c === 'cron') return `cron ${(s.expr as string) ?? ''} ${tz}`;
  if (c === 'once') {
    const at = typeof s.run_at === 'string' ? runAtLocal(s.run_at, tz).replace('T', ' ') : '';
    return at ? `once at ${at} ${tz}` : 'once';
  }
  if (c === 'daily') return `daily at ${(s.at as string) ?? '09:00'} ${tz}`;
  return `weekly ${WEEKDAYS[(s.weekday as number) ?? 0]} at ${(s.at as string) ?? '09:00'} ${tz}`;
}

/** The browser's IANA timezone (default for new schedules). */
export function browserTz(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}
