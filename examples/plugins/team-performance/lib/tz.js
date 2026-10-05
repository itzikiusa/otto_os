// Timezone-aware calendar helpers (pure). Every "day" in the plugin is a LOCAL
// day in the team's configured IANA zone — a commit at 23:30 in UTC+3 belongs
// to that local date, not to the next UTC one. Weekends are configurable
// (default Sat/Sun = [6,0]; e.g. [5,6] for a Fri/Sat week).
'use strict';

const DAY = 86400000;
const DEFAULT_WEEKEND = [6, 0];
const WD = { Sun: 0, Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6 };

const fmtCache = new Map();
function fmt(tz) {
  const k = tz || 'UTC';
  let f = fmtCache.get(k);
  if (!f) {
    f = new Intl.DateTimeFormat('en-US', {
      timeZone: k, hourCycle: 'h23', weekday: 'short',
      year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit',
    });
    fmtCache.set(k, f);
  }
  return f;
}

/** Local calendar parts of `ms` in `tz`: {y, m, d, wd, h, mi, s}. */
const offCache = new Map(); // `${tz}|${hour}` -> UTC offset ms (offsets only change on hour boundaries)
const UTC_WD = [0, 1, 2, 3, 4, 5, 6];
function parts(ms, tz) {
  // Fast path: Intl.formatToParts is ~µs per call and phases call it a lot —
  // resolve the zone offset once per hour bucket and read parts with UTC getters.
  const hour = Math.floor(ms / 3600000);
  const ck = `${tz}|${hour}`;
  let off = offCache.get(ck);
  if (off === undefined) {
    const t0 = hour * 3600000;
    const q = slowParts(t0, tz);
    off = Date.UTC(q.y, q.m - 1, q.d, q.h, q.mi, q.s) - t0;
    if (offCache.size > 200000) offCache.clear();
    offCache.set(ck, off);
  }
  const d = new Date(ms + off);
  return { y: d.getUTCFullYear(), m: d.getUTCMonth() + 1, d: d.getUTCDate(), wd: UTC_WD[d.getUTCDay()], h: d.getUTCHours(), mi: d.getUTCMinutes(), s: d.getUTCSeconds() };
}

function slowParts(ms, tz) {
  const o = {};
  for (const p of fmt(tz).formatToParts(new Date(ms))) o[p.type] = p.value;
  return { y: +o.year, m: +o.month, d: +o.day, wd: WD[o.weekday], h: +o.hour % 24, mi: +o.minute, s: +o.second };
}

/** 'YYYY-MM-DD' of `ms` in the local calendar of `tz`. */
function dayKey(ms, tz) {
  const p = parts(ms, tz);
  return `${p.y}-${String(p.m).padStart(2, '0')}-${String(p.d).padStart(2, '0')}`;
}

/** True when `ms` falls on a local workday (weekday not in `weekend`). */
function isWorkday(ms, tz, weekend = DEFAULT_WEEKEND) {
  return !weekend.includes(parts(ms, tz).wd);
}

/** Next local midnight strictly after `ms` (DST-safe: re-checks the key). */
function nextMidnight(ms, tz) {
  const p = parts(ms, tz);
  const intoDay = ((p.h * 60 + p.mi) * 60 + p.s) * 1000 + (ms % 1000 + 1000) % 1000;
  let t = ms - intoDay + DAY;
  // DST shifts land the guess ±1h off: nudge until it's the first instant of a new day.
  const k = dayKey(ms, tz);
  if (dayKey(t, tz) === k) { while (dayKey(t, tz) === k) t += 3600000; }
  const q = parts(t, tz);
  return t - ((q.h * 60 + q.mi) * 60 + q.s) * 1000;
}

/**
 * Fractional working days between two instants in local time: each local
 * workday that isn't in `offDays` (a Set of dayKeys — vacations, holidays)
 * contributes the share of its 24 h that the interval covers. Mon 10:00 →
 * Tue 10:00 = 1; Fri 10:00 → Mon 10:00 (Sat/Sun weekend) = 1.
 */
function businessDaysTz(startMs, endMs, { tz, weekend = DEFAULT_WEEKEND, offDays } = {}) {
  if (!(endMs > startMs)) return 0;
  let total = 0;
  let t = startMs;
  while (t < endMs) {
    const nm = nextMidnight(t, tz);
    const segEnd = Math.min(nm, endMs);
    if (isWorkday(t, tz, weekend) && !(offDays && offDays.has(dayKey(t, tz)))) {
      // Normalise by the local day's real length (23/25 h on DST days).
      const dayStart = startOfDay(t, tz);
      total += (segEnd - t) / (nm - dayStart);
    }
    t = segEnd;
  }
  return total;
}

/** First instant of the local day containing `ms`. */
function startOfDay(ms, tz) {
  const p = parts(ms, tz);
  return ms - (((p.h * 60 + p.mi) * 60 + p.s) * 1000 + (ms % 1000 + 1000) % 1000);
}

/** Distinct local dayKeys covered by `timestamps` (sorted). */
function distinctDays(timestamps, tz) {
  return [...new Set((timestamps || []).filter((t) => Number.isFinite(t)).map((t) => dayKey(t, tz)))].sort();
}

module.exports = { DAY, DEFAULT_WEEKEND, dayKey, isWorkday, businessDaysTz, distinctDays, nextMidnight };
