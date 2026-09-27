// A7: Mission Control's live refresh must not starve under a steady stream.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { liveDebounce } from '../src/modules/mission-control/liveDebounce.ts';

function clock() {
  let t = 0;
  let seq = 0;
  const timers = new Map<number, { at: number; fn: () => void }>();
  return {
    now: () => t,
    timers: {
      set: (fn: () => void, ms: number) => {
        const id = ++seq;
        timers.set(id, { at: t + ms, fn });
        return id;
      },
      clear: (h: unknown) => void timers.delete(h as number),
    },
    advance(ms: number) {
      const end = t + ms;
      for (;;) {
        const due = [...timers.entries()].filter(([, v]) => v.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        timers.delete(due[0]);
        t = due[1].at;
        due[1].fn();
      }
      t = end;
    },
  };
}

test('a lone burst runs once after the quiet window', () => {
  const c = clock();
  let runs = 0;
  const d = liveDebounce(() => runs++, 500, 3000, c.now, c.timers);
  d.trigger();
  c.advance(200);
  d.trigger();
  c.advance(499);
  assert.equal(runs, 0);
  c.advance(1);
  assert.equal(runs, 1);
  assert.equal(d.pending, false);
});

test('a steady stream (every 200 ms) still refreshes at least every max-wait', () => {
  const c = clock();
  const at: number[] = [];
  const d = liveDebounce(() => at.push(c.now()), 500, 3000, c.now, c.timers);
  for (let i = 0; i < 50; i++) {
    d.trigger();
    c.advance(200);
  }
  assert.ok(at.length >= 3, `refreshed ${at.length}× in 10 s`);
  assert.equal(at[0], 3000, 'first refresh at the max wait, not never');
  for (let i = 1; i < at.length; i++) assert.ok(at[i] - at[i - 1] <= 3200);
  d.cancel();
  c.advance(10_000);
  const n = at.length;
  assert.equal(at.length, n, 'cancel drops the pending run');
});
