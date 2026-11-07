// Cadence form helpers shared by the Schedules tab — same schedule_json shape
// as scheduled tasks (`{cadence:'interval'|'daily'|'weekly'|'cron'|'once', …}`).

import { cadenceLabel, runAtLocal } from '../../lib/cadence.ts';

// The label and the `run_at` reader are shared with scheduled tasks and
// workflow triggers (one humane cadence vocabulary, ux-design-2-04).
export { cadenceLabel, runAtLocal } from '../../lib/cadence.ts';

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

/** The working week where the user is: Sun–Thu in Israel, Mon–Fri elsewhere
 *  (0 = Sunday) — the default days of a new routine. */
export function workdays(tz = browserTz()): number[] {
  return tz === 'Asia/Jerusalem' || tz === 'Asia/Tel_Aviv' ? [0, 1, 2, 3, 4] : [1, 2, 3, 4, 5];
}

// --- Routine form: days + time in a timezone --------------------------------

/** Sun..Sat, 0 = Sunday (cron's numbering). */
export const DAY_CHIPS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** A stored schedule as "these days at HH:MM" (daily / weekly / a plain
 *  day-of-week cron), or null for anything else (interval, once, other cron). */
export function daysOf(s: Record<string, unknown>): { days: number[]; at: string } | null {
  const at = typeof s.at === 'string' ? s.at : '09:00';
  if (s.cadence === 'daily') return { days: [0, 1, 2, 3, 4, 5, 6], at };
  // weekly's weekday counts from Monday (0 = Mon).
  if (s.cadence === 'weekly') return { days: [((Number(s.weekday ?? 0) % 7) + 1) % 7], at };
  if (s.cadence !== 'cron' || typeof s.expr !== 'string') return null;
  const m = /^(\d{1,2}) (\d{1,2}) \* \* ([\d,-]+)$/.exec(s.expr.trim());
  if (!m || Number(m[1]) > 59 || Number(m[2]) > 23) return null;
  const days = new Set<number>();
  for (const part of m[3].split(',')) {
    const [a, b = a] = part.split('-').map(Number);
    if (!(a >= 0 && b <= 7 && a <= b)) return null;
    for (let d = a; d <= b; d++) days.add(d % 7);
  }
  return { days: [...days].sort((x, y) => x - y), at: `${m[2].padStart(2, '0')}:${m[1].padStart(2, '0')}` };
}

/** The schedule for "these days at HH:MM": every day → daily, else a cron. */
export function scheduleForDays(days: number[], at: string): Record<string, unknown> {
  if (days.length === 7) return { cadence: 'daily', at };
  const [h, m] = at.split(':').map(Number);
  return { cadence: 'cron', expr: `${m || 0} ${h || 0} * * ${[...days].sort((x, y) => x - y).join(',')}` };
}

/** `tz`'s offset from UTC at `at`, in ms. */
function tzOffset(at: number, tz: string): number {
  const p = Object.fromEntries(
    new Intl.DateTimeFormat('en-US', { timeZone: tz, hourCycle: 'h23', year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', second: 'numeric' })
      .formatToParts(new Date(at)).map((x) => [x.type, Number(x.value)]),
  );
  return Date.UTC(p.year, p.month - 1, p.day, p.hour % 24, p.minute, p.second) - at;
}

/** The instant of a wall-clock time in `tz`. */
function wallToInstant(y: number, mo: number, d: number, h: number, mi: number, tz: string): number {
  const guess = Date.UTC(y, mo, d, h, mi);
  const first = guess - tzOffset(guess, tz);
  return guess - tzOffset(first, tz);
}

/** The next fire of "these days at HH:MM" in `tz` after `now` (a preview —
 *  the daemon's own next run is the truth once saved). */
export function nextForDays(days: number[], at: string, tz: string, now = new Date()): Date | null {
  if (days.length === 0) return null;
  const [h, m] = at.split(':').map(Number);
  try {
    const local = new Date(now.getTime() + tzOffset(now.getTime(), tz));
    for (let i = 0; i < 8; i++) {
      const day = new Date(Date.UTC(local.getUTCFullYear(), local.getUTCMonth(), local.getUTCDate() + i));
      if (!days.includes(day.getUTCDay())) continue;
      const t = wallToInstant(day.getUTCFullYear(), day.getUTCMonth(), day.getUTCDate(), h || 0, m || 0, tz);
      if (t > now.getTime()) return new Date(t);
    }
  } catch {
    return null;
  }
  return null;
}

/** A routine's cadence in words, in the viewer's time ("Sun–Thu at 09:00").
 *  A routine kept in another timezone reads in the viewer's time too ("Daily
 *  at 22:42"), so every card agrees with its "Next: …"; its own zone shows in
 *  the edit dialog. Only when it can't be converted (another day there, a
 *  custom cron) does the zone stay, so the time is never wrong. */
export function routineWhen(s: { schedule: Record<string, unknown>; timezone: string }, now = new Date()): string {
  const label = cadenceLabel(s.schedule, s.timezone, { cronWords: true });
  const zone = ` · ${s.timezone}`;
  if (!s.timezone || s.timezone === browserTz() || !label.endsWith(zone)) {
    const own = ` · ${browserTz()}`;
    return label.endsWith(own) ? label.slice(0, -own.length) : label;
  }
  const bare = label.slice(0, -zone.length);
  const d = daysOf(s.schedule);
  const next = d && nextForDays(d.days, d.at, s.timezone, now);
  if (!d || !next || !bare.includes(`at ${d.at}`)) return `${bare} (${s.timezone})`;
  const local = `${String(next.getHours()).padStart(2, '0')}:${String(next.getMinutes()).padStart(2, '0')}`;
  // The same calendar day in both zones, or the day names would be wrong.
  const theirDay = new Date(next.getTime() + tzOffset(next.getTime(), s.timezone)).getUTCDate();
  if (theirDay !== next.getDate()) return `${bare} (${s.timezone})`;
  // Your time first, its own clock named so a routine set elsewhere is clear.
  return `${bare.replace(`at ${d.at}`, `at ${local}`)} (${d.at} ${s.timezone})`;
}

/** Every IANA zone the browser knows (the viewer's first), for a select. */
/** A handful of zones people actually pick, after the routine's and yours. */
const COMMON_ZONES = [
  'UTC',
  'Asia/Jerusalem',
  'Europe/London',
  'Europe/Berlin',
  'America/New_York',
  'America/Chicago',
  'America/Los_Angeles',
  'Asia/Kolkata',
  'Asia/Singapore',
  'Asia/Tokyo',
  'Australia/Sydney',
];

/** Timezone choices: `top` (the routine's, yours, then common ones) and
 *  `rest` (every other IANA zone) grouped by region ("Europe", "America" …),
 *  so the select opens on a short list and the long one is scannable. */
export function timezones(current: string): { top: string[]; rest: string[]; regions: [string, string[]][] } {
  let all: string[] = [];
  try {
    all = (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.('timeZone') ?? [];
  } catch {
    all = [];
  }
  const top = [...new Set([current, browserTz(), ...COMMON_ZONES].filter(Boolean))];
  const seen = new Set(top);
  const rest = all.filter((z) => !seen.has(z));
  const by = new Map<string, string[]>();
  for (const z of rest) {
    const region = z.includes('/') ? z.slice(0, z.indexOf('/')) : 'Other';
    by.set(region, [...(by.get(region) ?? []), z]);
  }
  return { top, rest, regions: [...by.entries()].sort(([a], [b]) => a.localeCompare(b)) };
}
