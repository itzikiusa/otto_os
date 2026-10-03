import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const backoff = loadSource(new URL('../src/lib/pollBackoff.ts', import.meta.url), {});
const refresh = loadSource(new URL('../src/modules/aws/refresh.ts', import.meta.url), { '../../lib/api/aws': { ALL_REGIONS: 'all' } });

/** Manual timers so cadences are asserted without real waiting. */
function fakeTimers() {
  let seq = 0;
  const pending = new Map<number, { fn: () => void; ms: number }>();
  return {
    setTimeout: (fn: () => void, ms: number) => { pending.set(++seq, { fn, ms }); return seq; },
    clearTimeout: (id: number) => { pending.delete(id); },
    pending,
    fire(): number[] {
      const due = [...pending.values()];
      pending.clear();
      for (const t of due) t.fn();
      return due.map((t) => t.ms);
    },
  };
}
const flush = () => new Promise((r) => setImmediate(r));

test('status backoff (Athena / Logs Insights): 1,1,2,2,3,5 then capped at 5 s', () => {
  const seq = Array.from({ length: 9 }, (_, n) => backoff.statusPollMs(n, false));
  assert.deepEqual(seq, [1000, 1000, 2000, 2000, 3000, 5000, 5000, 5000, 5000]);
});

test('status backoff: 15 s at every step while the window is hidden', () => {
  for (let n = 0; n < 8; n++) assert.equal(backoff.statusPollMs(n, true), 15_000);
});

test('adaptive delay (Logs tail): 2 → 4 → 8 → 10 cap, reset to 2 on data', () => {
  const b = { min: 2000, max: 10_000 };
  let ms = b.min;
  const seen = [ms];
  for (let i = 0; i < 4; i++) seen.push((ms = backoff.adaptiveDelay(ms, false, b)));
  assert.deepEqual(seen, [2000, 4000, 8000, 10_000, 10_000]);
  assert.equal(backoff.adaptiveDelay(ms, true, b), 2000, 'new events snap back to the live cadence');
  assert.equal(backoff.adaptiveDelay(2000, true, b), 2000, 'stays at 2 s while events keep arriving');
});

test('adaptive cadence (Kafka tail): 3 → 6 → 12 → 15 cap, reset on new messages', () => {
  const c = backoff.adaptiveCadence({ min: 3000, max: 15_000 });
  const seen = [c.ms];
  for (let i = 0; i < 4; i++) { c.record(false); seen.push(c.ms); }
  assert.deepEqual(seen, [3000, 6000, 12_000, 15_000, 15_000]);
  c.record(true);
  assert.equal(c.ms, 3000);
  c.record(false); c.reset();
  assert.equal(c.ms, 3000, 'reset() returns to min');
});

test('effectiveRefreshMs: all-regions mode is floored at 30 s, single region unchanged', () => {
  assert.equal(refresh.AUTO_REFRESH_MS, 10_000);
  assert.equal(refresh.effectiveRefreshMs(10_000, 'all'), 30_000);
  assert.equal(refresh.effectiveRefreshMs(60_000, 'all'), 60_000, 'a slower interval is kept');
  assert.equal(refresh.effectiveRefreshMs(10_000, 'eu-west-1'), 10_000);
  assert.equal(refresh.effectiveRefreshMs(10_000, undefined), 10_000);
});

test('pollWhileVisible honours a `get ms()` getter fed by adaptiveCadence', async () => {
  const timers = fakeTimers();
  const doc = { visibilityState: 'visible', addEventListener() {}, removeEventListener() {} };
  const { pollWhileVisible } = loadSource(
    new URL('../src/lib/poll.ts', import.meta.url),
    { './api/lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}) },
    { document: doc, setTimeout: timers.setTimeout, clearTimeout: timers.clearTimeout },
  );
  const cadence = backoff.adaptiveCadence({ min: 2000, max: 10_000 });
  // Ticks 1-3 empty, tick 4 has data, tick 5 empty.
  const data = [false, false, false, true, false];
  let tick = 0;
  const p = pollWhileVisible(async () => { cadence.record(data[tick++] ?? false); }, {
    get ms() { return cadence.ms; },
    jitter: 0,
    immediate: false,
  });
  const armed: number[] = [];
  for (let i = 0; i < 6; i++) { armed.push(...timers.fire()); await flush(); }
  assert.deepEqual(armed, [2000, 4000, 8000, 10_000, 2000, 4000]);
  p.stop();
});

test('createLimiter(2) (AccountsOverview probe stagger): never more than 2 in flight, all run', async () => {
  const { createLimiter } = loadSource(new URL('../src/lib/poll.ts', import.meta.url), {
    './api/lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  });
  const gate = createLimiter(2);
  let active = 0;
  let peak = 0;
  const releases: (() => void)[] = [];
  const done = Array.from({ length: 5 }, () =>
    gate(async () => {
      active++; peak = Math.max(peak, active);
      await new Promise<void>((r) => releases.push(r));
      active--;
    }),
  );
  await flush();
  assert.equal(releases.length, 2, 'only two probes start');
  while (releases.length) { releases.shift()!(); await flush(); }
  await Promise.all(done);
  assert.equal(peak, 2);
});

test('quiet cadence (Kafka Overview metrics): 4 s, then 8 → 10 s after 3 unchanged samples, snaps back on change', () => {
  const c = backoff.quietCadence({ min: 4000, max: 10_000, quietAfter: 3 });
  const seq: number[] = [];
  for (const changed of [true, false, false, false, false, false, true, false]) {
    c.sample(changed);
    seq.push(c.ms);
  }
  assert.deepEqual(seq, [4000, 4000, 4000, 8000, 10_000, 10_000, 4000, 4000]);
  c.sample(false); c.sample(false); c.reset();
  assert.equal(c.ms, 4000);
});
