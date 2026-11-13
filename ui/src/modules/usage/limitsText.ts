// Wording for the Usage page's "Remaining" section (GET /usage/limits): one
// row per provider login, one bar per window. Pure (type-only import), so
// `node --test` can load it.
//
// The rule this file keeps: a percent is only ever shown next to its age.
// A fresh snapshot reads "87% left"; a stale one "87% left 3 h ago"; a window
// that reset after the snapshot shows no percent at all.
import type { AccountLimits, ClaudeCheck, WindowLimit } from '../../lib/api/types/usage-limits';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** "3 d 4 h", "2 h 5 min", "12 min", "<1 min" for a positive span. */
export function formatSpan(ms: number): string {
  if (!Number.isFinite(ms) || ms < MIN) return '<1 min';
  const d = Math.floor(ms / DAY);
  const h = Math.floor((ms % DAY) / HOUR);
  const m = Math.floor((ms % HOUR) / MIN);
  if (d > 0) return h > 0 ? `${d} d ${h} h` : `${d} d`;
  if (h > 0) return m > 0 ? `${h} h ${m} min` : `${h} h`;
  return `${m} min`;
}

function ms(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
}

/** "resets in 3 d 4 h" / "reset 2 h ago" / "" when the CLI gave no time. */
export function resetText(w: Pick<WindowLimit, 'resets_at'>, now: number): string {
  const at = ms(w.resets_at);
  if (at === null) return '';
  return at > now ? `resets in ${formatSpan(at - now)}` : `reset ${formatSpan(now - at)} ago`;
}

/** "just now" / "5 min ago" / "3 h ago" for the snapshot's age. */
export function ageText(takenAt: string | null | undefined, now: number): string {
  const at = ms(takenAt);
  if (at === null) return '';
  const span = now - at;
  return span < MIN ? 'just now' : `${formatSpan(span)} ago`;
}

/** The snapshot is old: by the server's verdict, or by the clock since the
 *  page loaded (a tab left open must not keep calling an old reading fresh). */
export function isStale(a: Pick<AccountLimits, 'status' | 'taken_at'>, staleAfterSecs: number, now: number): boolean {
  if (a.status === 'stale') return true;
  if (a.status !== 'ok') return false;
  const at = ms(a.taken_at);
  return at !== null && now - at > staleAfterSecs * 1000;
}

/** A window's reset passed (on the server or since the page loaded). */
export function windowReset(w: WindowLimit, now: number): boolean {
  if (w.reset_since_snapshot) return true;
  const at = ms(w.resets_at);
  return at !== null && at <= now;
}

/** The percent line for one window. */
export function windowText(w: WindowLimit, stale: boolean, takenAt: string | null, now: number): string {
  if (windowReset(w, now) || w.left_percent === null) {
    return 'Reset since the last reading — no newer numbers yet';
  }
  const line = percentLine(w, stale, takenAt, now);
  return w.active === false ? `${line} · not in effect now` : line;
}

function percentLine(w: WindowLimit, stale: boolean, takenAt: string | null, now: number): string {
  if (w.left_percent === null) return '';
  const left = `${Math.round(w.left_percent)}% left`;
  const reset = resetText(w, now);
  if (stale) {
    const age = ageText(takenAt, now);
    return [age ? `${left} ${age}` : `${left} (old reading)`, reset].filter(Boolean).join(' · ');
  }
  return [left, reset].filter(Boolean).join(' · ');
}

/** Bar tone by what is left: under 10% danger, under 25% warning. */
export function barTone(leftPercent: number | null): 'danger' | 'warning' | 'ok' {
  if (leftPercent === null) return 'ok';
  if (leftPercent < 10) return 'danger';
  if (leftPercent < 25) return 'warning';
  return 'ok';
}

const PROVIDERS: Record<string, string> = { codex: 'Codex', claude: 'Claude', agy: 'Antigravity' };

export function providerName(p: string): string {
  return PROVIDERS[p] ?? p;
}

/** "Pro", "Plus", "Max 20x" — the CLI's plan id, capitalised. */
export function planLabel(plan: string | null): string {
  if (!plan) return '';
  return plan.charAt(0).toUpperCase() + plan.slice(1).replace(/_/g, ' ');
}

/** "Credits: 62,500" / "Credits: unlimited" / "" when none. */
export function creditsText(c: AccountLimits['credits']): string {
  if (!c) return '';
  if (c.unlimited) return 'Credits: unlimited';
  if (!c.has_credits) return '';
  const n = Number(c.balance);
  if (c.balance === null || !Number.isFinite(n)) return '';
  return `Credits: ${n.toLocaleString('en-US', { maximumFractionDigits: 2 })}`;
}

/** Screen-reader name for one bar. */
export function meterLabel(a: Pick<AccountLimits, 'label' | 'provider'>, w: WindowLimit, text: string): string {
  return `${providerName(a.provider)} ${a.label}, ${w.name}: ${text}`;
}

/** The daemon reuses a Claude check for 5 minutes (limits/cli.rs CHECK_TTL). */
export const CHECK_TTL_MS = 5 * MIN;

/** An opted-in Claude login whose last check is missing or older than the
 *  daemon's cache — the page asks for one on load. */
export function checkDue(c: ClaudeCheck | null | undefined, now: number): boolean {
  if (!c?.enabled || c.checking) return false;
  const at = ms(c.checked_at);
  return at === null || now - at >= CHECK_TTL_MS;
}

/** Whether the open page should start a check on its own now. Re-asked on
 *  every 60 s poll, so an opted-in row never goes Stale while the page is
 *  open: due by the daemon's clock (`checkDue`), the tab visible, and no
 *  automatic attempt of ours within the cache window — a run that failed
 *  before the daemon recorded it (offline, refused) is not retried every
 *  poll, so the promise in `CHECK_CONFIRM` holds either way. */
export function autoCheckDue(
  c: ClaudeCheck | null | undefined,
  now: number,
  lastAutoAt: number | null,
  hidden: boolean,
): boolean {
  if (hidden || !checkDue(c, now)) return false;
  return lastAutoAt === null || now - lastAutoAt >= CHECK_TTL_MS;
}

/** The line under a Claude row's switch. */
export function checkStatusText(c: ClaudeCheck | null | undefined, busy: boolean, now: number): string {
  if (!c?.enabled) return 'Off — Otto never asks Anthropic for this login.';
  if (busy || c.checking) return 'Checking with Anthropic…';
  if (c.error) return `Last check failed: ${c.error}`;
  const age = ageText(c.checked_at, now);
  return age ? `Checked ${age}` : 'Not checked yet';
}

/** What the switch's confirmation says: what runs, where it goes, what is read. */
export const CHECK_CONFIRM =
  'Otto will run this login’s own claude CLI and ask it for plan usage. The CLI sends that request to Anthropic with its own sign-in; Otto does not read the login. Otto checks at most once every 5 minutes, and only while the Usage page is open on screen. You can turn this off at any time.';
