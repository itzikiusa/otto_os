// Cadence form helpers shared by the Schedules tab — same schedule_json shape
// as scheduled tasks (`{cadence:'interval'|'daily'|'weekly'|'cron'|'once', …}`).

import { runAtLocal } from '../../lib/cadence';

// The label and the `run_at` reader are shared with scheduled tasks and
// workflow triggers (one humane cadence vocabulary, ux-design-2-04).
export { cadenceLabel, runAtLocal } from '../../lib/cadence';

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

/** The browser's IANA timezone (default for new schedules). */
export function browserTz(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}
