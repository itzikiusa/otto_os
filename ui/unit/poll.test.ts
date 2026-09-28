import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

/** A controllable document: visibility + visibilitychange listeners. */
function fakeDocument() {
  const listeners = new Set<() => void>();
  return {
    visibilityState: 'visible' as 'visible' | 'hidden',
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
    set(state: 'visible' | 'hidden') {
      this.visibilityState = state;
      for (const fn of [...listeners]) fn();
    },
    listeners,
  };
}

/** Manual timers so cadence/backoff are asserted without real waiting. */
function fakeTimers() {
  let seq = 0;
  const pending = new Map<number, { fn: () => void; ms: number }>();
  return {
    setTimeout: (fn: () => void, ms: number) => { pending.set(++seq, { fn, ms }); return seq; },
    clearTimeout: (id: number) => { pending.delete(id); },
    pending,
    /** Fire every armed timer once; returns the delays that were armed. */
    fire(): number[] {
      const due = [...pending.values()];
      pending.clear();
      for (const t of due) t.fn();
      return due.map((t) => t.ms);
    },
  };
}

const flush = () => new Promise((r) => setImmediate(r));

function load(doc?: ReturnType<typeof fakeDocument>, timers?: ReturnType<typeof fakeTimers>) {
  return loadSource(new URL('../src/lib/poll.ts', import.meta.url), { './api/lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}) }, {
    ...(doc ? { document: doc } : {}),
    ...(timers ? { setTimeout: timers.setTimeout, clearTimeout: timers.clearTimeout } : {}),
  });
}

test('in-flight guard: a due tick never overlaps a pending run', async () => {
  const timers = fakeTimers();
  const { pollWhileVisible } = load(undefined, timers);
  let calls = 0;
  const releases: (() => void)[] = [];
  const p = pollWhileVisible(() => { calls++; return new Promise<void>((r) => releases.push(r)); }, { ms: 1000, jitter: 0 });
  assert.equal(calls, 1);
  assert.equal(timers.pending.size, 0, 'no timer armed while the run is pending');
  p.now(); p.now();
  assert.equal(calls, 1);
  releases[0](); await flush();
  assert.equal(calls, 2, 'one coalesced rerun');
  releases[1](); await flush();
  assert.deepEqual([...timers.pending.values()].map((t) => t.ms), [1000], 'then exactly one chain');
  p.stop();
  assert.equal(timers.pending.size, 0);
});

test('failures back off x2 up to maxBackoff, success resets', async () => {
  const timers = fakeTimers();
  const { pollWhileVisible } = load(undefined, timers);
  let fail = true;
  const p = pollWhileVisible(async () => { if (fail) throw new Error('down'); return true; }, { ms: 1000, jitter: 0, maxBackoff: 4 });
  await flush();
  assert.deepEqual(timers.fire(), [2000]); await flush();
  assert.deepEqual(timers.fire(), [4000]); await flush();
  assert.deepEqual(timers.fire(), [4000], 'capped'); await flush();
  fail = false;
  timers.fire(); await flush();
  assert.deepEqual([...timers.pending.values()].map((t) => t.ms), [1000], 'reset after success');
  p.stop();
});

test('resolving false counts as a failure', async () => {
  const timers = fakeTimers();
  const { pollWhileVisible } = load(undefined, timers);
  const p = pollWhileVisible(async () => false, { ms: 500, floorMs: 500, jitter: 0 });
  await flush();
  assert.deepEqual([...timers.pending.values()].map((t) => t.ms), [1000]);
  p.stop();
});

test('hidden: pause skips ticks with no timer, and runs once on return', async () => {
  const doc = fakeDocument();
  const timers = fakeTimers();
  const { pollWhileVisible } = load(doc, timers);
  let calls = 0;
  const p = pollWhileVisible(async () => { calls++; }, { ms: 1000, jitter: 0 });
  await flush();
  assert.equal(calls, 1);
  doc.set('hidden');
  timers.fire(); await flush();
  assert.equal(calls, 1, 'no run while hidden');
  assert.equal(timers.pending.size, 0, 'and no timer chain while hidden');
  doc.set('visible'); await flush();
  assert.equal(calls, 2, 'the owed tick runs on return');
  assert.equal(timers.pending.size, 1);
  p.stop();
  assert.equal(doc.listeners.size, 0, 'stop removes the visibility listener');
});

test('hidden: a numeric cadence keeps polling slower while hidden', async () => {
  const doc = fakeDocument();
  const timers = fakeTimers();
  const { pollWhileVisible } = load(doc, timers);
  let calls = 0;
  const p = pollWhileVisible(async () => { calls++; }, { ms: 1000, hidden: 5000, jitter: 0 });
  await flush();
  doc.set('hidden');
  assert.deepEqual(timers.fire(), [1000]); await flush();
  assert.equal(calls, 2, 'still runs while hidden');
  assert.deepEqual([...timers.pending.values()].map((t) => t.ms), [5000], 'at the hidden cadence');
  doc.set('visible');
  assert.deepEqual([...timers.pending.values()].map((t) => t.ms), [1000], 're-armed at the visible cadence');
  p.stop();
});

test('stop aborts the in-flight run and never reschedules', async () => {
  const timers = fakeTimers();
  const { pollWhileVisible } = load(undefined, timers);
  let seen: AbortSignal | null = null;
  let release!: () => void;
  const p = pollWhileVisible((signal: AbortSignal) => { seen = signal; return new Promise<void>((r) => { release = r; }); }, { ms: 1000 });
  p.stop();
  assert.equal(seen!.aborted, true);
  release(); await flush();
  assert.equal(timers.pending.size, 0);
});

test('an owner signal stops the poller; immediate:false waits one cadence', async () => {
  const timers = fakeTimers();
  const { pollWhileVisible } = load(undefined, timers);
  const owner = new AbortController();
  let calls = 0;
  pollWhileVisible(async () => { calls++; }, { ms: 2000, jitter: 0, immediate: false, signal: owner.signal });
  assert.equal(calls, 0);
  assert.deepEqual([...timers.pending.values()].map((t) => t.ms), [2000]);
  owner.abort();
  assert.equal(timers.pending.size, 0);
});

test('jitter stays within ±fraction', async () => {
  const timers = fakeTimers();
  const { pollWhileVisible } = load(undefined, timers);
  for (let i = 0; i < 20; i++) {
    const p = pollWhileVisible(async () => {}, { ms: 10_000, jitter: 0.1 });
    await flush();
    const [ms] = [...timers.pending.values()].map((t) => t.ms);
    assert.ok(ms >= 9000 && ms <= 11_000, `delay ${ms}`);
    p.stop();
  }
});

test('mapLimit caps concurrency and keeps order', async () => {
  const { mapLimit } = load();
  let active = 0, peak = 0;
  const out = await mapLimit([1, 2, 3, 4, 5, 6], 2, async (n: number) => {
    active++; peak = Math.max(peak, active);
    await new Promise((r) => setTimeout(r, 1));
    active--;
    return n * 10;
  });
  assert.equal(peak, 2);
  assert.deepEqual([...out], [10, 20, 30, 40, 50, 60]);
});
