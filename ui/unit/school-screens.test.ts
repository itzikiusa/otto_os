/// <reference lib="dom" />
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pollScreens, type ScreenFeed } from '../src/modules/home/school/screens.ts';

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

test('hidden documents abort screen reads, suppress late feeds, and resume one nonoverlapping batch', async (t) => {
  const doc = new EventTarget() as EventTarget & { hidden: boolean };
  doc.hidden = false;
  const original = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: doc });
  t.after(() => { if (original) Object.defineProperty(globalThis, 'document', original); else Reflect.deleteProperty(globalThis, 'document'); });
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const requests: { signal: AbortSignal; resolve: (feed: ScreenFeed) => void }[] = [];
  const feeds: ScreenFeed[] = [];
  const p = pollScreens({
    wanted: () => ['kid'],
    fetch: (_id, signal) => new Promise((resolve) => requests.push({ signal, resolve })),
    onFeed: (_id, f) => feeds.push(f),
  }, 100);
  t.after(() => p.stop());
  t.mock.timers.tick(1);
  assert.equal(requests.length, 1);
  doc.hidden = true;
  doc.dispatchEvent(new Event('visibilitychange'));
  assert.equal(requests[0].signal.aborted, true, 'hide aborts the active read');
  doc.hidden = false;
  doc.dispatchEvent(new Event('visibilitychange'));
  doc.dispatchEvent(new Event('visibilitychange'));
  assert.equal(requests.length, 1, 'an abort-insensitive transport cannot overlap the next batch');
  requests[0].resolve({ live: true, lines: ['obsolete'] });
  await flush();
  t.mock.timers.tick(1);
  assert.equal(feeds.length, 0, 'an aborted result must never paint after reveal');
  assert.equal(requests.length, 2, 'exactly one fresh batch after reveal');
  requests[1].resolve({ live: true, lines: ['fresh'] });
  await flush();
  assert.deepEqual(feeds.map((f) => f.lines), [['fresh']]);
  doc.hidden = true;
  doc.dispatchEvent(new Event('visibilitychange'));
  t.mock.timers.tick(1000);
  assert.equal(requests.length, 2, 'no scheduled work while hidden');
  p.stop();
  doc.hidden = false;
  doc.dispatchEvent(new Event('visibilitychange'));
  p.now();
  t.mock.timers.tick(1000);
  assert.equal(requests.length, 2, 'stop removes the reveal listener and timer');
});

test('a poller created while hidden waits for reveal and still caps each batch at twelve', async (t) => {
  const doc = new EventTarget() as EventTarget & { hidden: boolean };
  doc.hidden = true;
  const original = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: doc });
  t.after(() => { if (original) Object.defineProperty(globalThis, 'document', original); else Reflect.deleteProperty(globalThis, 'document'); });
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const ids: string[] = [];
  const p = pollScreens({ wanted: () => Array.from({ length: 20 }, (_, i) => `${i}`), fetch: async (id) => { ids.push(id); return { live: true, lines: [] }; }, onFeed: () => {} });
  t.after(() => p.stop());
  t.mock.timers.tick(5000);
  p.now();
  await flush();
  assert.equal(ids.length, 0);
  doc.hidden = false;
  doc.dispatchEvent(new Event('visibilitychange'));
  t.mock.timers.tick(1);
  await flush();
  assert.equal(ids.length, 12);
});
