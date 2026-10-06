// S14-303: a guest terminal gave up after ~27 s of refused upgrades and
// never came back on its own after a longer daemon restart.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { reconnectDelay, GUEST_SLOW_RETRY_MS, GUEST_RETRY_WINDOW_MS } from '../src/lib/components/guestReconnect.ts';

/** Simulate an outage that refuses every upgrade for `outageMs`. */
function simulate(outageMs: number, guest = true): { reconnectedAt: number | null; attempts: number } {
  let now = 0;
  let attempts = 0;
  let refusals = 0;
  let first: number | null = null;
  for (;;) {
    if (now >= outageMs) return { reconnectedAt: now, attempts };
    // This attempt is refused.
    if (refusals === 0) first = now;
    refusals++;
    const d = reconnectDelay({ guest, attempts, refusals, firstRefusalAt: first, now });
    if (d === null) return { reconnectedAt: null, attempts };
    attempts++;
    now += d;
  }
}

test('a guest survives more than 30 s of refusals and reconnects (a slow deploy)', () => {
  const r = simulate(90_000);
  assert.notEqual(r.reconnectedAt, null, 'still retrying after 90 s');
  assert.ok(r.reconnectedAt! - 90_000 <= GUEST_SLOW_RETRY_MS, 'back within one slow interval');
});

test('a guest stops after the retry window (a revoked link is not hammered forever)', () => {
  const r = simulate(Number.POSITIVE_INFINITY);
  assert.equal(r.reconnectedAt, null);
  assert.ok(r.attempts < 8 + GUEST_RETRY_WINDOW_MS / GUEST_SLOW_RETRY_MS + 2, `bounded: ${r.attempts} attempts`);
});

test('owners keep the fast ladder forever', () => {
  assert.equal(reconnectDelay({ guest: false, attempts: 50, refusals: 50, firstRefusalAt: 0, now: 3_600_000 }), 5000);
  assert.equal(reconnectDelay({ guest: true, attempts: 0, refusals: 1, firstRefusalAt: 0, now: 0 }), 500);
});

test('the share page re-arms the terminal when its access re-check succeeds', () => {
  const share = readFileSync(new URL('../src/modules/share/SharePage.svelte', import.meta.url), 'utf8');
  assert.match(share, /await getSharedSession\(id, t\);\s*[\s\S]{0,200}termView\?\.reconnect\(\)/);
  const term = readFileSync(new URL('../src/lib/components/Terminal.svelte', import.meta.url), 'utf8');
  assert.match(term, /export function reconnect\(\): void/);
  // Both overlay buttons start a fresh ladder (the plain "Reconnect" used not to).
  assert.equal((term.match(/onclick=\{\(\) => \{ resetRetries\(\); connect\(\{ view: false \}\); \}\}/g) ?? []).length, 2);
});
