// A turn that ended on the provider's usage limit is a state with a next
// step (T3 Code's UsageLimitRecoveryBanner / UsageLimitRecoveryWorker, MIT):
// when the limit resets — from the provider's own words when it states it,
// else from the usage meter's limits snapshot — and which line to show.
// Pure (unit/runtimeUsageLimit.test.ts). Detection is failureCause()
// (turnFailure.ts): a structured class (Codex usageLimitExceeded) or the
// provider's limit text (Claude, Gemini / ACP, pi).
import type { AccountLimits } from '../../../lib/api/types/usage-limits';

/** When the provider's error says the limit resets (ms), or null.
 *  Shapes: Claude "…usage limit reached|1763906400" (epoch seconds) and
 *  "…limit reached ∙ resets 3pm" / "resets at 14:00"; Codex "try again in
 *  2 hours 5 minutes" / "try again at 3:05 PM"; Gemini "Please retry in 41s";
 *  an ISO time. A clock time in the past means tomorrow. */
export function resetFromText(text: string, now: number): number | null {
  const epoch = /limit reached\|(\d{9,11})\b/i.exec(text);
  if (epoch) return Number(epoch[1]) * 1000;
  const iso = /\b(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2}))\b/.exec(text);
  if (iso && !Number.isNaN(Date.parse(iso[1]))) return Date.parse(iso[1]);
  const rel = /(?:try again|retry) in\s+((?:\d+(?:\.\d+)?\s*(?:days?|d|hours?|hrs?|h|minutes?|mins?|m|seconds?|secs?|s)\b[\s,and]*)+)/i.exec(text);
  if (rel) {
    let ms = 0;
    for (const m of rel[1].matchAll(/(\d+(?:\.\d+)?)\s*(days?|d|hours?|hrs?|h|minutes?|mins?|m|seconds?|secs?|s)\b/gi)) {
      const n = Number(m[1]);
      const u = m[2].toLowerCase();
      ms += n * (u.startsWith('d') ? 86_400_000 : u.startsWith('h') ? 3_600_000 : u.startsWith('m') ? 60_000 : 1000);
    }
    return ms > 0 ? now + ms : null;
  }
  const clock = /(?:resets?|try again)(?: at)?\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?\b/i.exec(text);
  if (clock) {
    let h = Number(clock[1]);
    const mi = Number(clock[2] ?? 0);
    const ap = clock[3]?.toLowerCase();
    if (!ap && !clock[2]) return null; // "resets 3" — not a time
    if (ap === 'pm' && h < 12) h += 12;
    if (ap === 'am' && h === 12) h = 0;
    if (h > 23 || mi > 59) return null;
    const d = new Date(now);
    d.setHours(h, mi, 0, 0);
    if (d.getTime() <= now) d.setDate(d.getDate() + 1);
    return d.getTime();
  }
  return null;
}

/** When the usage meter says `provider`'s exhausted window resets (ms): the
 *  soonest future reset among its spent windows (or any window while the
 *  account reports the limit reached), or null. */
export function resetFromLimits(accounts: readonly AccountLimits[] | null | undefined, provider: string, now: number): number | null {
  let best: number | null = null;
  for (const a of accounts ?? []) {
    if (a.provider !== provider) continue;
    for (const w of a.windows) {
      const at = w.resets_at ? Date.parse(w.resets_at) : NaN;
      const spent = (w.used_percent ?? 0) >= 99 || (w.left_percent ?? 100) <= 0 || !!a.limit_reached;
      if (spent && at > now && (best === null || at < best)) best = at;
    }
  }
  return best;
}

/** "14:00", or "Tue 14:00" when it is not today. */
export function resetClock(at: number, now: number): string {
  const d = new Date(at);
  const hm = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return new Date(now).toDateString() === d.toDateString() ? hm : `${d.toLocaleDateString([], { weekday: 'short' })} ${hm}`;
}
