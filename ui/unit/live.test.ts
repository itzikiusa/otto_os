import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

// liveQuery (src/lib/live.ts): event-driven refetch with a slow safety net
// while the event socket is up and the old poll cadence while it is down.

/** Manual timers + clock shared by poll.ts and live.ts. */
function fakeClock() {
  let seq = 0;
  let now = 1_000_000;
  const pending = new Map<number, { fn: () => void; ms: number; at: number }>();
  return {
    setTimeout: (fn: () => void, ms: number) => {
      pending.set(++seq, { fn, ms, at: now + ms });
      return seq;
    },
    clearTimeout: (id: number) => {
      pending.delete(id);
    },
    pending,
    delays: () => [...pending.values()].map((t) => t.ms).sort((a, b) => a - b),
    /** Advance the clock by `ms`, firing every timer that comes due (in order). */
    advance(ms: number) {
      const target = now + ms;
      for (;;) {
        const due = [...pending.entries()].filter(([, t]) => t.at <= target).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        pending.delete(due[0]);
        now = due[1].at;
        due[1].fn();
      }
      now = target;
    },
    Date: class {
      static now() {
        return now;
      }
    },
  };
}

const flush = () => new Promise((r) => setImmediate(r));

function load(clock: ReturnType<typeof fakeClock>) {
  const globals = { setTimeout: clock.setTimeout, clearTimeout: clock.clearTimeout, Date: clock.Date };
  const poll = loadSource(new URL('../src/lib/poll.ts', import.meta.url), { './api/lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}) }, globals);
  return loadSource(new URL('../src/lib/live.ts', import.meta.url), { './poll': poll }, globals);
}

function setup(opts: Record<string, unknown> = {}, connected = true) {
  const clock = fakeClock();
  const live = load(clock);
  const reg = new live.LiveRegistry();
  reg.setConnected(connected);
  let runs = 0;
  const q = live.liveQuery(
    {
      run: async () => {
        runs++;
      },
      on: ['thing_changed'],
      fallbackMs: 5000,
      jitter: 0,
      ...opts,
    },
    reg,
  );
  return { clock, live, reg, q, runs: () => runs };
}

test('runs once, then only on events (coalesced), with the safety net while connected', async () => {
  const { clock, reg, q, runs } = setup();
  await flush();
  assert.equal(runs(), 1, 'initial load');
  assert.deepEqual(clock.delays(), [300_000], 'connected → safety cadence, not the 5 s poll');
  for (let i = 0; i < 5; i++) reg.dispatch({ type: 'thing_changed' });
  reg.dispatch({ type: 'other_event' });
  clock.advance(249);
  await flush();
  assert.equal(runs(), 1, 'debounced');
  clock.advance(1);
  await flush();
  assert.equal(runs(), 2, 'a burst of 5 events costs one refetch');
  assert.deepEqual(clock.delays(), [300_000]);
  q.stop();
});

test('socket down → catch-up run, then the fallback cadence; resync refetches', async () => {
  const { clock, reg, q, runs } = setup();
  await flush();
  reg.setConnected(false);
  clock.advance(250);
  await flush();
  assert.equal(runs(), 2, 'catch-up after the socket dropped');
  assert.deepEqual(clock.delays(), [5000], 'offline → the old poll cadence');
  clock.advance(5000);
  await flush();
  assert.equal(runs(), 3);
  reg.setConnected(true);
  reg.resync();
  clock.advance(1750);
  await flush();
  assert.equal(runs(), 4, 'reconnect resync refetches (staggered ≤ 1.5 s)');
  assert.deepEqual(clock.delays(), [300_000], 'and restores the safety net');
  q.stop();
});

test('socket coming up pushes a pending fallback tick out to the safety net', async () => {
  const { clock, reg, q, runs } = setup({}, false);
  await flush();
  assert.equal(runs(), 1);
  assert.deepEqual(clock.delays(), [5000], 'offline start → fallback cadence');
  reg.setConnected(true);
  assert.deepEqual(clock.delays(), [300_000], 're-armed at the safety net, no fetch');
  assert.equal(runs(), 1);
  q.stop();
});

test('match narrows events; stop unsubscribes and cancels timers', async () => {
  const { clock, reg, q, runs } = setup({ match: (ev: { id?: string }) => ev.id === 'mine' });
  await flush();
  reg.dispatch({ type: 'thing_changed', id: 'theirs' });
  clock.advance(1000);
  await flush();
  assert.equal(runs(), 1, 'non-matching event ignored');
  reg.dispatch({ type: 'thing_changed', id: 'mine' });
  q.stop();
  clock.advance(1000);
  await flush();
  assert.equal(runs(), 1, 'stopped: the pending debounce never fires');
  assert.equal(clock.pending.size, 0);
  assert.deepEqual([...reg.types()], [], 'listener removed');
});

test('a continuous stream still refreshes within maxWaitMs', async () => {
  const { clock, reg, q, runs } = setup({ debounceMs: 500, maxWaitMs: 2000 });
  await flush();
  for (let t = 0; t < 3000; t += 200) {
    reg.dispatch({ type: 'thing_changed' });
    clock.advance(200);
    await flush();
  }
  assert.ok(runs() >= 2, `refreshed during the stream (runs=${runs()})`);
  q.stop();
});

test('minIntervalMs caps event-driven refetches under churn', async () => {
  const { clock, reg, q, runs } = setup({ debounceMs: 100, maxWaitMs: 500, minIntervalMs: 10_000 });
  await flush();
  for (let t = 0; t < 30_000; t += 1000) {
    reg.dispatch({ type: 'thing_changed' });
    clock.advance(1000);
    await flush();
  }
  // initial + at most one per 10 s over 30 s of events every second
  assert.ok(runs() <= 4, `rate capped (runs=${runs()})`);
  assert.ok(runs() >= 3, `still refreshing (runs=${runs()})`);
  q.stop();
});

test('numeric hidden cadence relaxes while connected', async () => {
  const clock = fakeClock();
  const doc = {
    visibilityState: 'hidden',
    addEventListener() {},
    removeEventListener() {},
  };
  const globals = { setTimeout: clock.setTimeout, clearTimeout: clock.clearTimeout, Date: clock.Date, document: doc };
  const poll = loadSource(new URL('../src/lib/poll.ts', import.meta.url), { './api/lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}) }, globals);
  const live = loadSource(new URL('../src/lib/live.ts', import.meta.url), { './poll': poll }, globals);
  const reg = new live.LiveRegistry();
  reg.setConnected(true);
  const q = live.liveQuery({ run: async () => {}, on: [], fallbackMs: 20_000, hidden: 20_000, jitter: 0 }, reg);
  await flush();
  assert.deepEqual(clock.delays(), [300_000], 'hidden tray + live socket → safety net');
  q.stop();
  reg.setConnected(false);
  const q2 = live.liveQuery({ run: async () => {}, on: [], fallbackMs: 20_000, hidden: 20_000, jitter: 0 }, reg);
  await flush();
  assert.deepEqual(clock.delays(), [20_000], 'hidden + socket down → the old hidden cadence');
  q2.stop();
});

test('no source → a plain poll at fallbackMs', async () => {
  const clock = fakeClock();
  const live = load(clock);
  let runs = 0;
  const q = live.liveQuery({ run: async () => { runs++; }, on: ['x'], fallbackMs: 7000, jitter: 0 }, null);
  await flush();
  assert.equal(runs, 1);
  assert.deepEqual(clock.delays(), [7000]);
  q.stop();
});

test('LiveRegistry isolates a throwing listener and reports its types', () => {
  const clock = fakeClock();
  const live = load(clock);
  const reg = new live.LiveRegistry();
  let got = 0;
  const off1 = reg.on(['a'], () => {
    throw new Error('boom');
  });
  reg.on(['a', 'b'], () => got++);
  reg.dispatch({ type: 'a' });
  reg.dispatch({ type: 'b' });
  assert.equal(got, 2);
  assert.deepEqual([...reg.types()], ['a', 'b']);
  off1();
  let conn: boolean[] = [];
  reg.onConnection((c: boolean) => conn.push(c));
  reg.setConnected(true);
  reg.setConnected(true);
  reg.setConnected(false);
  assert.deepEqual(conn, [true, false], 'transitions only');
});
