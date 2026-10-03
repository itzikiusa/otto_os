import { test } from 'node:test';
import assert from 'node:assert/strict';
import { KeyLatency, ProbeClock, RollingStats, fmtMs, fmtPair } from '../src/lib/components/termLatency.ts';

test('RollingStats keeps the last N and reports nearest-rank percentiles', () => {
  const s = new RollingStats(4);
  assert.equal(s.pct(50), null);
  for (const v of [100, 1, 2, 3, 4]) s.push(v); // 100 falls out
  assert.equal(s.count, 4);
  assert.equal(s.last, 4);
  assert.equal(s.pct(50), 2);
  assert.equal(s.pct(95), 4);
  s.push(Number.NaN);
  s.push(-1);
  assert.equal(s.count, 4, 'junk samples are ignored');
});

test('KeyLatency splits one keystroke into wire, parse and paint', () => {
  const k = new KeyLatency();
  let clock = 0;
  k.input(10);
  k.input(12); // same burst: measured from the first key
  const parsed = k.frame(40, () => clock);
  assert.ok(parsed, 'the answering frame gets a parse callback');
  assert.equal(k.frame(41, () => clock), undefined, 'later frames are not samples');
  clock = 55;
  parsed!();
  k.rendered(70);
  k.rendered(90); // a second render pass is not another sample
  assert.equal(k.wire.last, 30);
  assert.equal(k.parse.last, 15);
  assert.equal(k.paint.last, 15);
  assert.equal(k.paint.count, 1);
});

test('KeyLatency ignores output with no keystroke pending, and reset drops a sample', () => {
  const k = new KeyLatency();
  assert.equal(k.frame(5, () => 5), undefined);
  k.input(1);
  k.reset();
  assert.equal(k.frame(9, () => 9), undefined);
  assert.equal(k.wire.count, 0);
});

test('ProbeClock pairs acks with their probe and forgets stale ones', () => {
  const p = new ProbeClock();
  const a = p.next(0);
  const b = p.next(10);
  p.ack(b, 14);
  p.ack(b, 99); // duplicate ack: ignored
  p.ack(12345, 20); // unknown id
  assert.equal(p.rtt.last, 4);
  assert.equal(p.rtt.count, 1);
  assert.equal(p.outstanding, 1);
  p.next(40_000); // `a` is now older than 30 s
  p.ack(a, 40_001);
  assert.equal(p.rtt.count, 1);
});

test('HUD formatting', () => {
  assert.equal(fmtMs(null), '–');
  assert.equal(fmtMs(3.14159), '3.1');
  assert.equal(fmtMs(123.6), '124');
  const s = new RollingStats();
  assert.equal(fmtPair(s), '–');
  s.push(2);
  s.push(40);
  assert.equal(fmtPair(s), '2.0/40');
});
