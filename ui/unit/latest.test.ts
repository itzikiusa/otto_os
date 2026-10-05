import { test } from 'node:test';
import assert from 'node:assert/strict';
import { latestByKey, latestOnly } from '../src/lib/latest.ts';

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test('an older response that lands last is not current', async () => {
  const l = latestOnly();
  const a = l.begin();
  const b = l.begin();
  assert.equal(a.current, false);
  assert.equal(b.current, true);
  assert.equal(a.signal.aborted, true, 'superseded ticket is aborted');
  assert.equal(b.signal.aborted, false);
  assert.ok(b.gen > a.gen);
});

test('cancel invalidates the in-flight ticket (dispose / unmount)', () => {
  const l = latestOnly();
  const t = l.begin();
  l.cancel();
  assert.equal(t.current, false);
  assert.equal(t.signal.aborted, true);
});

test('run() drops the superseded result and keeps the latest', async () => {
  const l = latestOnly();
  const slow = deferred<string>();
  const fast = deferred<string>();
  const first = l.run(() => slow.promise);
  const second = l.run(() => fast.promise);
  fast.resolve('B');
  slow.resolve('A');
  assert.deepEqual(await second, { ok: true, value: 'B' });
  assert.deepEqual(await first, { ok: false });
});

test('run() swallows a superseded rejection but propagates the current one', async () => {
  const l = latestOnly();
  const old = deferred<string>();
  const p1 = l.run(() => old.promise);
  const p2 = l.run(() => Promise.reject(new Error('boom')));
  old.reject(new Error('stale'));
  assert.deepEqual(await p1, { ok: false });
  await assert.rejects(p2, /boom/);
});

test('latestByKey keeps slots independent', () => {
  const k = latestByKey<string>();
  const a1 = k.begin('a');
  const b1 = k.begin('b');
  const a2 = k.begin('a');
  assert.equal(a1.current, false);
  assert.equal(a2.current, true);
  assert.equal(b1.current, true);
  k.cancelAll();
  assert.equal(a2.current, false);
  assert.equal(b1.current, false);
});
