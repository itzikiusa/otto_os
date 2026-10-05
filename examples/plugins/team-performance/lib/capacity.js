// Capacity: the working days a person (or team) was actually available in a
// window — business days minus holidays minus their entered time off
// (people.json time_off). Pure, day-granular, zero-dep. Every per-person rate
// in the plugin divides by THIS, never by raw calendar/business days, so a
// two-week vacation never reads as a productivity drop.
//
// window   = { since, until } epoch ms, `until` exclusive.
// time_off = [{ from: 'YYYY-MM-DD', to: 'YYYY-MM-DD' }]  (both inclusive).
// opts     = { tz: minutes east of UTC (default 0), weekend: [0,6] (getUTCDay
//              numbers that are NOT worked), holidays: ['YYYY-MM-DD'] }.
'use strict';

const DAY = 86400000;
const DEFAULT_WEEKEND = [0, 6];

const dayKey = (ms) => new Date(ms).toISOString().slice(0, 10);
const parseDay = (s) => {
  const t = Date.parse(String(s).slice(0, 10) + 'T00:00:00Z');
  return Number.isFinite(t) ? t : null;
};

/** The set of local-calendar work days ('YYYY-MM-DD') inside the window. */
function workDaysIn(window, opts = {}) {
  const out = new Set();
  if (!window || !(window.since < window.until)) return out;
  const tz = (Number(opts.tz) || 0) * 60000;
  const weekend = new Set(opts.weekend || DEFAULT_WEEKEND);
  const holidays = new Set((opts.holidays || []).map((h) => String(h).slice(0, 10)));
  // Shift into local wall time, then walk whole local days. A day counts when
  // its local midnight falls inside [since, until).
  const from = Math.ceil((window.since + tz) / DAY) * DAY;
  for (let d = from; d < window.until + tz; d += DAY) {
    if (weekend.has(new Date(d).getUTCDay())) continue;
    const k = dayKey(d);
    if (!holidays.has(k)) out.add(k);
  }
  return out;
}

/** Work days in the window the person was off (overlapping entries and
 *  weekend/holiday days inside a vacation are never double-subtracted). */
function timeOffDays(person, window, opts = {}) {
  const work = workDaysIn(window, opts);
  const off = new Set();
  for (const t of (person && person.time_off) || []) {
    const a = parseDay(t && t.from);
    const b = parseDay(t && (t.to || t.from));
    if (a == null || b == null || b < a) continue;
    for (let d = a; d <= b; d += DAY) {
      const k = dayKey(d);
      if (work.has(k)) off.add(k);
    }
  }
  return off.size;
}

/** { business_days, time_off_days, capacity_days } for one person. */
function availability(person, window, opts = {}) {
  const business = workDaysIn(window, opts).size;
  const off = timeOffDays(person, window, opts);
  return { business_days: business, time_off_days: off, capacity_days: Math.max(0, business - off) };
}

/** Business days in the window minus the person's time off. */
function availableDays(person, window, opts = {}) {
  return availability(person, window, opts).capacity_days;
}

/** Team capacity: per-person breakdown + totals. `people` is an array or an
 *  id→person map. */
function teamCapacity(people, window, opts = {}) {
  const list = Array.isArray(people) ? people.map((p, i) => [p.id ?? String(i), p]) : Object.entries(people || {});
  const per = {};
  let capacity = 0;
  let off = 0;
  let business = 0;
  for (const [id, p] of list) {
    const a = availability(p, window, opts);
    per[id] = a;
    capacity += a.capacity_days;
    off += a.time_off_days;
    business += a.business_days;
  }
  return { people: per, business_days: business, time_off_days: off, capacity_days: capacity, headcount: list.length };
}

/** value / days, or null when there is no capacity to divide by. */
function perCapacity(value, days) {
  if (value == null || !Number.isFinite(Number(value)) || !(Number(days) > 0)) return null;
  return Number(value) / Number(days);
}

module.exports = { DAY, workDaysIn, timeOffDays, availability, availableDays, teamCapacity, perCapacity };
