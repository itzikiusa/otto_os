// Cadence form helpers shared by the Schedules tab — same schedule_json shape
// as scheduled tasks (`{cadence:'interval'|'daily'|'weekly'|'cron', …}`).

export interface CadenceForm {
  cadence: 'interval' | 'daily' | 'weekly' | 'cron';
  everyMin: number;
  at: string;
  weekday: number;
  cronExpr: string;
}

export const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

export function defaultCadence(): CadenceForm {
  return { cadence: 'daily', everyMin: 60, at: '09:00', weekday: 0, cronExpr: '0 9 * * 1' };
}

/** Populate the form from a stored schedule object. */
export function loadCadence(s: Record<string, unknown>): CadenceForm {
  const cad = (s.cadence as string) ?? 'interval';
  return {
    cadence: ['daily', 'weekly', 'cron'].includes(cad) ? (cad as CadenceForm['cadence']) : 'interval',
    everyMin: (s.every_min as number) ?? 60,
    at: (s.at as string) ?? '09:00',
    weekday: (s.weekday as number) ?? 0,
    cronExpr: (s.expr as string) ?? '0 9 * * 1',
  };
}

/** The schedule_json the daemon validates (`cadence::validate`). */
export function buildCadence(f: CadenceForm): Record<string, unknown> {
  if (f.cadence === 'interval') return { cadence: 'interval', every_min: Math.max(5, f.everyMin) };
  if (f.cadence === 'daily') return { cadence: 'daily', at: f.at };
  if (f.cadence === 'cron') return { cadence: 'cron', expr: f.cronExpr.trim() };
  return { cadence: 'weekly', at: f.at, weekday: f.weekday };
}

/** Human label for a stored schedule (+ timezone where it applies). */
export function cadenceLabel(s: Record<string, unknown>, timezone: string): string {
  const c = (s.cadence as string) ?? 'interval';
  const tz = timezone || 'UTC';
  if (c === 'interval') return `every ${(s.every_min as number) ?? 60} min`;
  if (c === 'cron') return `cron ${(s.expr as string) ?? ''} ${tz}`;
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
