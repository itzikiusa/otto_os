// A usage limit as a state with a next step (runtime/usageLimit.ts + turnFailure.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resetFromLimits, resetFromText } from '../src/modules/threads/runtime/usageLimit.ts';
import { failureCause } from '../src/modules/threads/runtime/turnFailure.ts';

const NOW = new Date(2026, 10, 23, 12, 0, 0).getTime(); // 12:00 local

test('each provider’s limit error is recognised', () => {
  for (const f of [
    { class: 'provider_error', code: 'usageLimitExceeded', message: 'You have reached your usage limit.' }, // Codex, structured
    { class: 'provider_error', message: 'Claude AI usage limit reached|1763906400' },
    { class: 'provider_error', message: '5-hour limit reached ∙ resets 3pm' },
    { class: 'provider_error', message: "You've hit your usage limit. Upgrade to Pro or try again in 2 hours 5 minutes." },
    { class: 'provider_error', message: 'Quota exceeded for quota metric \'Gemini 2.5 Pro Requests\' (RESOURCE_EXHAUSTED). Please retry in 41.2s.' },
    { class: 'provider_error', message: '429 Too Many Requests: rate limit exceeded' },
  ]) assert.equal(failureCause(f), 'usage_limit', f.message);
  assert.equal(failureCause({ class: 'provider_error', message: 'Model overloaded' }), 'provider');
});

test('the reset time from the provider’s own words', () => {
  assert.equal(resetFromText('Claude AI usage limit reached|1763906400', NOW), 1763906400_000);
  assert.equal(resetFromText('5-hour limit reached ∙ resets 3pm', NOW), new Date(2026, 10, 23, 15, 0).getTime());
  assert.equal(resetFromText('limit reached, resets at 09:30', NOW), new Date(2026, 10, 24, 9, 30).getTime(), 'past today: tomorrow');
  assert.equal(resetFromText('try again in 2 hours 5 minutes', NOW), NOW + 2 * 3_600_000 + 5 * 60_000);
  assert.equal(resetFromText('Please retry in 41.2s.', NOW), NOW + 41_200);
  assert.equal(resetFromText('limit resets at 2026-11-23T14:00:00Z', NOW), Date.parse('2026-11-23T14:00:00Z'));
  assert.equal(resetFromText('usage limit reached', NOW), null);
  assert.equal(resetFromText('resets 3', NOW), null);
});

test('the reset time from the usage meter: the soonest future reset of a spent window', () => {
  const at = (h: number) => new Date(NOW + h * 3_600_000).toISOString();
  const acct = (provider: string, windows: unknown[], limit_reached: string | null = null) => ({ provider, windows, limit_reached }) as never;
  const w = (used: number, h: number) => ({ used_percent: used, left_percent: 100 - used, resets_at: at(h) });
  assert.equal(resetFromLimits([acct('claude', [w(100, 2), w(40, 1)])], 'claude', NOW), NOW + 2 * 3_600_000);
  assert.equal(resetFromLimits([acct('claude', [w(40, 1)])], 'claude', NOW), null);
  assert.equal(resetFromLimits([acct('claude', [w(40, 3), w(10, 1)], 'five_hour')], 'claude', NOW), NOW + 3_600_000);
  assert.equal(resetFromLimits([acct('codex', [w(100, 2)])], 'claude', NOW), null);
  assert.equal(resetFromLimits(null, 'claude', NOW), null);
});
