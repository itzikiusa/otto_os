// Usage ▸ Remaining wording (modules/usage/limitsText.ts): a percent is only
// ever shown next to its age once the reading is old, a window that reset
// after the reading shows no percent, and countdowns read "3 d 4 h".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ageText,
  barTone,
  autoCheckDue,
  CHECK_CONFIRM,
  checkDue,
  checkStatusText,
  creditsText,
  formatSpan,
  isStale,
  limitReachedShown,
  planLabel,
  providerName,
  resetText,
  windowReset,
  windowText,
  providerMeter,
  windowShort,
} from '../src/modules/usage/limitsText.ts';
import type { AccountLimits, WindowLimit } from '../src/lib/api/types/usage-limits.ts';

const MIN = 60_000;
const HOUR = 60 * MIN;
const NOW = Date.parse('2026-11-12T12:00:00Z');
const iso = (t: number) => new Date(t).toISOString();

function win(over: Partial<WindowLimit> = {}): WindowLimit {
  return {
    id: 'primary',
    name: '5-hour',
    window_minutes: 300,
    used_percent: 13,
    left_percent: 87,
    resets_at: iso(NOW + 2 * HOUR + 5 * MIN),
    reset_since_snapshot: false,
    ...over,
  };
}

test('spans read in days+hours, hours+minutes, minutes', () => {
  assert.equal(formatSpan(76 * HOUR), '3 d 4 h');
  assert.equal(formatSpan(48 * HOUR), '2 d');
  assert.equal(formatSpan(2 * HOUR + 5 * MIN), '2 h 5 min');
  assert.equal(formatSpan(3 * HOUR), '3 h');
  assert.equal(formatSpan(12 * MIN + 30_000), '12 min');
  assert.equal(formatSpan(10_000), '<1 min');
  assert.equal(formatSpan(Number.NaN), '<1 min');
});

test('a fresh window reads "N% left · resets in …"', () => {
  assert.equal(windowText(win(), false, iso(NOW - 2 * MIN), NOW), '87% left · resets in 2 h 5 min');
  assert.equal(windowText(win({ resets_at: null }), false, iso(NOW), NOW), '87% left');
});

test('a stale window carries the age of its reading', () => {
  const text = windowText(win({ left_percent: 8, resets_at: iso(NOW + 30 * HOUR) }), true, iso(NOW - 5 * HOUR), NOW);
  assert.equal(text, '8% left 5 h ago · resets in 1 d 6 h');
  assert.match(windowText(win(), true, null, NOW), /^87% left \(old reading\)/);
});

test('a reset window shows no percent — by the server or by the clock', () => {
  const server = win({ used_percent: null, left_percent: null, reset_since_snapshot: true, resets_at: iso(NOW - HOUR) });
  assert.equal(windowReset(server, NOW), true);
  assert.doesNotMatch(windowText(server, true, iso(NOW - 5 * HOUR), NOW), /%/);
  // The page stayed open past the reset time: the old percent is withdrawn.
  const passed = win({ resets_at: iso(NOW - MIN) });
  assert.equal(windowReset(passed, NOW), true);
  assert.doesNotMatch(windowText(passed, false, iso(NOW - 10 * MIN), NOW), /%/);
  assert.equal(resetText(passed, NOW), 'reset 1 min ago');
});

test('staleness: the server verdict, or the clock since the page loaded', () => {
  assert.equal(isStale({ status: 'stale', taken_at: iso(NOW) }, 1800, NOW), true);
  assert.equal(isStale({ status: 'ok', taken_at: iso(NOW - 10 * MIN) }, 1800, NOW), false);
  assert.equal(isStale({ status: 'ok', taken_at: iso(NOW - 31 * MIN) }, 1800, NOW), true);
  assert.equal(isStale({ status: 'unknown', taken_at: null }, 1800, NOW), false);
});

test('limit reached: on a fresh reading, or an old one while a used-up window holds', () => {
  const usedUp = win({ id: 'usage_limit', used_percent: 100, left_percent: 0, resets_at: iso(NOW + 20 * HOUR) });
  assert.equal(limitReachedShown({ limit_reached: null, windows: [usedUp] }, false, NOW), false);
  assert.equal(limitReachedShown({ limit_reached: 'usage_limit_exceeded', windows: [win()] }, false, NOW), true);
  assert.equal(limitReachedShown({ limit_reached: 'usage_limit_exceeded', windows: [win(), usedUp] }, true, NOW), true);
  assert.equal(limitReachedShown({ limit_reached: 'usage_limit_exceeded', windows: [win()] }, true, NOW), false);
  // Once the reset passes the old reading proves nothing.
  assert.equal(limitReachedShown({ limit_reached: 'usage_limit_exceeded', windows: [usedUp] }, true, NOW + 21 * HOUR), false);
});

test('age, tone, plan, provider and credits labels', () => {
  assert.equal(ageText(iso(NOW - 20_000), NOW), 'just now');
  assert.equal(ageText(iso(NOW - 5 * MIN), NOW), '5 min ago');
  assert.equal(ageText(null, NOW), '');
  assert.equal(barTone(8), 'danger');
  assert.equal(barTone(20), 'warning');
  assert.equal(barTone(87), 'ok');
  assert.equal(barTone(null), 'ok');
  assert.equal(planLabel('pro'), 'Pro');
  assert.equal(planLabel('max_20x'), 'Max 20x');
  assert.equal(planLabel(null), '');
  assert.equal(providerName('codex'), 'Codex');
  assert.equal(providerName('agy'), 'Antigravity');
  assert.equal(providerName('grok'), 'Grok');
  assert.equal(providerName('opencode'), 'OpenCode');
  assert.equal(providerName('pi'), 'Pi');
  assert.equal(providerName('claudework'), 'claudework');
  assert.equal(planLabel('SuperGrok Heavy'), 'SuperGrok Heavy');
  assert.equal(creditsText({ has_credits: true, unlimited: false, balance: '62500.0' }), 'Credits: 62,500');
  assert.equal(creditsText({ has_credits: false, unlimited: false, balance: '0' }), '');
  assert.equal(creditsText({ has_credits: false, unlimited: true, balance: null }), 'Credits: unlimited');
  assert.equal(creditsText(null), '');
});

test('an inactive window says so after its numbers', () => {
  assert.equal(
    windowText(win({ name: 'Weekly · Fable', left_percent: 77, active: false }), false, iso(NOW), NOW),
    '77% left · resets in 2 h 5 min · not in effect now',
  );
  assert.equal(windowText(win({ active: true }), false, iso(NOW), NOW), '87% left · resets in 2 h 5 min');
});

test('Claude check: due only when on, idle and older than the daemon cache', () => {
  const base = { enabled: true, checking: false, checked_at: null as string | null, error: null as string | null };
  assert.equal(checkDue(base, NOW), true, 'never checked');
  assert.equal(checkDue({ ...base, checked_at: iso(NOW - 2 * MIN) }, NOW), false, 'fresh');
  assert.equal(checkDue({ ...base, checked_at: iso(NOW - 6 * MIN) }, NOW), true, 'past 5 min');
  assert.equal(checkDue({ ...base, checking: true }, NOW), false, 'one at a time');
  assert.equal(checkDue({ ...base, enabled: false }, NOW), false, 'off never runs');
  assert.equal(checkDue(null, NOW), false);
});

test('Claude check on an open page: re-run each time it is due, never faster than the cache', () => {
  const base = { enabled: true, checking: false, checked_at: iso(NOW - 6 * MIN), error: null as string | null };
  assert.equal(autoCheckDue(base, NOW, null, false), true, 'due on open');
  // Not a one-shot: a later poll past the cache runs again.
  assert.equal(autoCheckDue(base, NOW, NOW - 6 * MIN, false), true, 'due again after 5 min');
  // A run that never reached the daemon (checked_at unchanged) is not retried every 60 s poll.
  assert.equal(autoCheckDue(base, NOW, NOW - 1 * MIN, false), false, 'our own attempt 1 min ago');
  assert.equal(autoCheckDue(base, NOW, NOW - 5 * MIN, false), true, 'exactly one cache window later');
  assert.equal(autoCheckDue({ ...base, checked_at: iso(NOW - 2 * MIN) }, NOW, null, false), false, 'fresh');
  assert.equal(autoCheckDue(base, NOW, null, true), false, 'never while the tab is hidden');
  assert.equal(autoCheckDue({ ...base, enabled: false }, NOW, null, false), false, 'never while off');
  assert.equal(autoCheckDue({ ...base, checking: true }, NOW, null, false), false, 'one at a time');
  assert.match(CHECK_CONFIRM, /at most once every 5 minutes/);
});

test('Claude check status line: off, checking, failed, checked', () => {
  const off = { enabled: false, checking: false, checked_at: null, error: null };
  assert.match(checkStatusText(off, false, NOW), /^Off — Otto never asks Anthropic/);
  const on = { ...off, enabled: true };
  assert.equal(checkStatusText(on, true, NOW), 'Checking with Anthropic…');
  assert.equal(checkStatusText({ ...on, checking: true }, false, NOW), 'Checking with Anthropic…');
  assert.equal(checkStatusText({ ...on, error: 'not logged in' }, false, NOW), 'Last check failed: not logged in');
  assert.equal(checkStatusText({ ...on, checked_at: iso(NOW - 3 * MIN) }, false, NOW), 'Checked 3 min ago');
  assert.equal(checkStatusText(on, false, NOW), 'Not checked yet');
  // The confirmation names what runs, where it goes and what is not read.
  assert.match(CHECK_CONFIRM, /own claude CLI/);
  assert.match(CHECK_CONFIRM, /Anthropic/);
  assert.match(CHECK_CONFIRM, /does not read the login/);
});

test('the Threads meter: the tightest fresh window, its tone, every window in the tooltip', () => {
  const acct = (over: Partial<AccountLimits>): AccountLimits => ({
    key: 'codex:default', provider: 'codex', account_id: null, label: 'Default Codex login', source: 'default', status: 'ok', reason: null,
    plan: 'pro', taken_at: iso(NOW - 4 * MIN), credits: null, limit_reached: null,
    windows: [
      { id: 'primary', name: '5-hour', window_minutes: 300, used_percent: 62, left_percent: 38, resets_at: iso(NOW + 2 * HOUR), reset_since_snapshot: false },
      { id: 'secondary', name: 'Weekly', window_minutes: 10080, used_percent: 41, left_percent: 59, resets_at: iso(NOW + 76 * HOUR), reset_since_snapshot: false },
    ],
    ...over,
  });
  const m = providerMeter([acct({})], 'codex', 1800, NOW)!;
  assert.deepEqual([m.short, m.used, m.tone], ['5h', 62, 'ok']);
  assert.equal(m.lines[0], 'Codex · Default Codex login');
  assert.match(m.lines[1], /^5-hour: 38% left · resets in 2 h$/);
  assert.match(m.lines[2], /^Weekly: 59% left/);
  const hot = acct({ windows: [{ ...acct({}).windows[1], used_percent: 96, left_percent: 4 }] });
  assert.deepEqual([providerMeter([hot], 'codex', 1800, NOW)!.short, providerMeter([hot], 'codex', 1800, NOW)!.tone], ['7d', 'danger']);
  assert.equal(providerMeter([acct({ windows: [{ ...acct({}).windows[0], used_percent: 80 }] })], 'codex', 1800, NOW)!.tone, 'warning');
  // No numbers: another provider, a stale or unknown reading.
  assert.equal(providerMeter([acct({})], 'claude', 1800, NOW), null);
  assert.equal(providerMeter([acct({ taken_at: iso(NOW - 3 * HOUR) })], 'codex', 1800, NOW), null);
  assert.equal(providerMeter([acct({ status: 'unknown' })], 'codex', 1800, NOW), null);
  assert.equal(windowShort({ name: 'Monthly', window_minutes: null }), 'Monthly');
});
