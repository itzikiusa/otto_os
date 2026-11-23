import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// Pure helpers of the keep-awake store, with the shell bridge stubbed out.
const mod = loadSource(new URL('../src/lib/keepAwake.svelte.ts', import.meta.url), {
  './desktop': { isTauri: false },
  './confirm.svelte': { confirmer: {} },
  './toastError': { toastError: () => 0 },
}) as typeof import('../src/lib/keepAwake.svelte.ts');

test('time left reads like the menu-bar title, rounded up', () => {
  const at = (ms: number) => mod.timeLeft({ active: true, until_ms: 1_000 + ms }, 1_000);
  assert.equal(at(42 * 60_000), '42m');
  assert.equal(at(42 * 60_000 - 1), '42m');
  assert.equal(at(60 * 60_000), '1h');
  assert.equal(at(65 * 60_000), '1h 5m');
  assert.equal(at(0), '1m', 'never 0m while still on');
  assert.equal(mod.timeLeft({ active: true, until_ms: null }, 0), null);
});

test('a custom duration reads minutes, hours or h:mm', () => {
  for (const [input, minutes] of [
    ['90', 90], ['45m', 45], ['45 min', 45], ['2h', 120], ['1h 30m', 90], ['1h30', 90], ['1:30', 90], ['3 hours', 180],
  ] as const) assert.equal(mod.parseMinutes(input), minutes, input);
  for (const bad of ['', '0', 'soon', '-5', '1:75', 'h']) assert.equal(mod.parseMinutes(bad), null, bad);
});
