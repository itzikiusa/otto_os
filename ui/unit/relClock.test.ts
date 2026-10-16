// The shared clock (lib/stores/now.svelte.ts): `rel(ts)` subscribes only as
// finely as its label can change. A label under an hour old re-runs on the
// 1 s tick (it flips at arbitrary second offsets); an hours/days-old one
// re-runs once a wall minute — dozens of list rows used to re-run every second.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { importRunes, svelteClient } from './runesHarness.ts';

type Clock = { rel: (ts: number) => string };

// The store starts its interval at import only when `window` exists: stand
// in for the browser, capture the tick, and drive Date.now by hand.
const START = Date.UTC(2026, 9, 16, 12, 0, 0);
let fakeNow = START;
let tick: (() => void) | null = null;
const realNow = Date.now;
const realSetInterval = globalThis.setInterval;
const g = globalThis as Record<string, unknown>;
g.window = {};
g.document = { hidden: false, addEventListener() {} };
globalThis.setInterval = ((fn: () => void) => {
  tick = fn;
  return 0;
}) as unknown as typeof setInterval;
Date.now = () => fakeNow;
const mod = await importRunes<Clock>(new URL('../src/lib/stores/now.svelte.ts', import.meta.url));
globalThis.setInterval = realSetInterval;
delete g.window;
delete g.document;

function advance(ms: number): void {
  fakeNow += ms;
  tick!();
}

test('labels follow the clock at their own granularity', async (t) => {
  t.after(() => {
    Date.now = realNow;
  });
  assert.ok(tick, 'the clock installed its 1 s interval');
  const $ = await svelteClient();
  const recent = START - 10_000; // "10s ago"
  const old = START - 2 * 3_600_000 - 5_000; // "2h ago"
  let recentRuns = 0;
  let oldRuns = 0;
  const labels = { recent: '', old: '' };
  const stop = $.effectRoot(() => {
    $.effect(() => {
      labels.recent = mod.rel(recent);
      recentRuns++;
    });
    $.effect(() => {
      labels.old = mod.rel(old);
      oldRuns++;
    });
  });
  $.flushSync();
  assert.equal(labels.recent, '10s ago');
  assert.equal(labels.old, '2h ago');
  assert.equal(recentRuns, 1);
  assert.equal(oldRuns, 1);

  // 30 one-second ticks inside the same wall minute.
  for (let i = 0; i < 30; i++) {
    advance(1000);
    $.flushSync();
  }
  assert.equal(labels.recent, '40s ago');
  assert.equal(recentRuns, 31, 'a seconds label re-runs every tick');
  assert.equal(oldRuns, 1, 'an hours label waits for the minute');

  // Cross the wall minute: the hours label refreshes once.
  advance(30_000);
  $.flushSync();
  assert.equal(oldRuns, 2);
  assert.equal(labels.recent, '1m ago');

  // An hour of minute ticks later the hours label is exact again.
  for (let i = 0; i < 60; i++) {
    advance(60_000);
    $.flushSync();
  }
  assert.equal(labels.old, '3h ago');
  assert.equal(labels.recent, '1h ago');
  stop();
});
