import { test } from 'node:test';
import assert from 'node:assert/strict';
import { appendCapped, appendFiltered } from '../src/modules/kubernetes/logRing.ts';

test('the ring appends in place and keeps the newest `cap` lines', () => {
  const buf: string[] = [];
  const same = buf;
  assert.deepEqual(appendCapped(buf, ['a', 'b', 'c'], 4), []);
  const trimmed = appendCapped(buf, ['d', 'e', 'f'], 4);
  assert.equal(buf, same, 'no copy: the same array grows');
  assert.deepEqual(buf, ['c', 'd', 'e', 'f']);
  assert.deepEqual(trimmed, ['a', 'b']);
});

test('a chunk bigger than the cap keeps only its tail', () => {
  const buf = ['x'];
  const add = Array.from({ length: 50_000 }, (_, i) => `l${i}`);
  const trimmed = appendCapped(buf, add, 20_000);
  assert.equal(buf.length, 20_000);
  assert.equal(buf[0], 'l30000');
  assert.equal(buf[19_999], 'l49999');
  assert.equal(trimmed.length, 30_001);
});

test('the filtered view drops exactly the matches the ring trimmed', () => {
  const keep = (l: string) => l.includes('err');
  const buf: string[] = [];
  const view: string[] = [];
  let t = appendCapped(buf, ['err1', 'ok1', 'err2'], 3);
  appendFiltered(view, ['err1', 'ok1', 'err2'], t, keep);
  assert.deepEqual(view, ['err1', 'err2']);
  t = appendCapped(buf, ['ok2', 'err3'], 3);
  appendFiltered(view, ['ok2', 'err3'], t, keep);
  assert.deepEqual(buf, ['err2', 'ok2', 'err3']);
  assert.deepEqual(view, buf.filter(keep));
});

test('pending buffer stays capped while the window is hidden (S16-12)', async () => {
  const { pushPendingCapped } = await import('../src/modules/kubernetes/logRing.ts');
  const pending: string[] = [];
  // Many chunks with no flush in between (rAF paused) never exceed the cap.
  for (let c = 0; c < 50; c++) pushPendingCapped(pending, Array.from({ length: 1000 }, (_, i) => `${c}:${i}`), 20_000);
  assert.equal(pending.length, 20_000);
  assert.equal(pending[pending.length - 1], '49:999');
  assert.equal(pending[0], '30:0');
  // One huge chunk (no spread → no stack overflow).
  const big: string[] = [];
  pushPendingCapped(big, new Array(300_000).fill('x'), 5);
  assert.equal(big.length, 5);
});
