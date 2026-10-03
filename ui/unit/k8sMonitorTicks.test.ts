import { test } from 'node:test';
import assert from 'node:assert/strict';
import { TickCoalescer, type TickCoalescerEnv } from '../src/modules/kubernetes/monitor/tickCoalescer.ts';

/** Manual clock + timer queue + visibility. */
function fakeEnv() {
  let now = 0;
  let hidden = false;
  const timers: { at: number; fn: () => void }[] = [];
  const visible: (() => void)[] = [];
  const env: TickCoalescerEnv = {
    now: () => now,
    setTimer: (fn, ms) => timers.push({ at: now + ms, fn }),
    hidden: () => hidden,
    onVisible: (fn) => visible.push(fn),
  };
  return {
    env,
    advance(ms: number) {
      now += ms;
      for (;;) {
        const due = timers.filter((t) => t.at <= now).sort((a, b) => a.at - b.at)[0];
        if (!due) break;
        timers.splice(timers.indexOf(due), 1);
        due.fn();
      }
    },
    setHidden(h: boolean) {
      hidden = h;
      if (!h) visible.forEach((fn) => fn());
    },
  };
}

test('the first cycle ticks at once; a burst inside the gap becomes ONE tick', () => {
  const f = fakeEnv();
  const ticks: string[][] = [];
  const c = new TickCoalescer(30_000, (cl) => ticks.push(cl), f.env);
  c.cycle('a');
  f.advance(0);
  assert.deepEqual(ticks, [['a']]);
  // Three clusters cycle within 10 s: nothing until the gap elapses.
  c.cycle('b');
  f.advance(4_000);
  c.cycle('c');
  c.cycle('b');
  f.advance(20_000);
  assert.equal(ticks.length, 1);
  f.advance(6_000);
  assert.deepEqual(ticks, [['a'], ['c', 'b']]);
  // Quiet: no cycles, no ticks.
  f.advance(120_000);
  assert.equal(ticks.length, 2);
});

test('no ticks while hidden; one on return carrying every cluster that cycled', () => {
  const f = fakeEnv();
  const ticks: string[][] = [];
  const c = new TickCoalescer(30_000, (cl) => ticks.push(cl), f.env);
  f.setHidden(true);
  c.cycle('a');
  f.advance(60_000);
  c.cycle('b');
  f.advance(600_000);
  assert.equal(ticks.length, 0, 'hidden pages never refresh');
  f.setHidden(false);
  f.advance(0);
  assert.deepEqual(ticks, [['a', 'b']]);
});

test('a tick that comes due while hidden waits for visibility', () => {
  const f = fakeEnv();
  const ticks: string[][] = [];
  const c = new TickCoalescer(30_000, (cl) => ticks.push(cl), f.env);
  c.cycle('a');
  f.advance(0);
  c.cycle('a');
  f.advance(10_000);
  f.setHidden(true);
  f.advance(30_000);
  assert.equal(ticks.length, 1);
  f.setHidden(false);
  f.advance(0);
  assert.deepEqual(ticks, [['a'], ['a']]);
});
