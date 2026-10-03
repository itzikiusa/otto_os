import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CompactQueue, type CompactClient } from '../src/lib/components/termCompactQueue.ts';

type TimerId = ReturnType<typeof setTimeout>;

function harness() {
  const timers: { fn: () => void; ms: number; live: boolean }[] = [];
  const q = new CompactQueue(
    5000,
    (fn, ms) => {
      timers.push({ fn, ms, live: true });
      return (timers.length - 1) as unknown as TimerId;
    },
    (id) => {
      const t = timers[id as unknown as number];
      if (t) t.live = false;
    },
  );
  const fire = () => {
    for (const t of timers) if (t.live) { t.live = false; t.fn(); }
  };
  return { q, fire };
}

function pane(name: string, focus: number, log: string[], opts: { eligible?: () => boolean; accept?: boolean } = {}): CompactClient {
  return {
    lastFocus: () => focus,
    eligible: opts.eligible ?? (() => true),
    run: () => {
      log.push(name);
      return opts.accept ?? true;
    },
  };
}

test('one compact in flight per window; the focused pane goes first', () => {
  const { q } = harness();
  const log: string[] = [];
  const a = pane('a', 1, log);
  const b = pane('b', 9, log);
  const c = pane('c', 5, log);
  // A burst from a window resize: the first request starts immediately, the
  // rest wait for its reply, then go most-recently-focused first.
  q.request(a);
  q.request(b);
  q.request(c);
  assert.deepEqual(log, ['a']);
  q.done(a);
  assert.deepEqual(log, ['a', 'b']);
  q.done(b);
  assert.deepEqual(log, ['a', 'b', 'c']);
  q.done(c);
  assert.equal(q.active, null);
});

test('a repeated request while queued or in flight is one compact', () => {
  const { q } = harness();
  const log: string[] = [];
  const a = pane('a', 1, log);
  q.request(a);
  q.request(a);
  q.request(a);
  q.done(a);
  assert.deepEqual(log, ['a']);
});

test('panes that left the screen are dropped, not run', () => {
  const { q } = harness();
  const log: string[] = [];
  let visible = true;
  const a = pane('a', 1, log);
  const hidden = pane('hidden', 10, log, { eligible: () => visible });
  q.request(a);
  q.request(hidden);
  visible = false;
  q.done(a);
  assert.deepEqual(log, ['a']);
  assert.equal(q.pending, 0);
});

test('a pane that declines at run time frees the slot at once', () => {
  const { q } = harness();
  const log: string[] = [];
  const reading = pane('reading', 9, log, { accept: false });
  const b = pane('b', 1, log);
  q.request(reading);
  assert.equal(q.active, null);
  q.request(b);
  assert.deepEqual(log, ['reading', 'b']);
});

test('a lost reply frees the slot after the timeout; cancel frees it at once', () => {
  const { q, fire } = harness();
  const log: string[] = [];
  const a = pane('a', 1, log);
  const b = pane('b', 1, log);
  const c = pane('c', 1, log);
  q.request(a);
  q.request(b);
  fire(); // a's reply never came
  assert.deepEqual(log, ['a', 'b']);
  q.request(c);
  q.cancel(b); // b's socket closed
  assert.deepEqual(log, ['a', 'b', 'c']);
  // A late reply from a no-longer-active pane changes nothing.
  q.done(a);
  assert.equal(q.active, c);
});
