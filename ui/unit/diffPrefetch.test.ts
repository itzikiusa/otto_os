// Neighbour prefetch for the lazy PR diff (src/modules/git/diff-prefetch.ts):
// nearest-first ordering, the concurrency + byte caps, cancellation on a new
// focus, and visible loads taking slots from prefetches.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  NET_SLOTS,
  PREFETCH_AHEAD,
  PREFETCH_BEHIND,
  PREFETCH_MAX,
  PrefetchScheduler,
  neighbours,
  searchPlan,
} from '../src/modules/git/diff-prefetch.ts';

const files = (n: number): string[] => Array.from({ length: n }, (_, i) => `f${i}`);
const flush = () => new Promise((r) => setImmediate(r));

/** A scheduler whose runs stay pending until released. */
function harness(opts: { busy?: number; full?: boolean; wanted?: (p: string) => boolean } = {}) {
  const started: string[] = [];
  const aborted: string[] = [];
  const release = new Map<string, () => void>();
  const state = { busy: opts.busy ?? 0, full: opts.full ?? false };
  const s = new PrefetchScheduler({
    run: (p, signal) =>
      new Promise<void>((resolve) => {
        started.push(p);
        signal.addEventListener('abort', () => {
          aborted.push(p);
          resolve();
        });
        release.set(p, resolve);
      }),
    wanted: opts.wanted ?? (() => true),
    busy: () => state.busy,
    full: () => state.full,
  });
  return { s, started, aborted, release, state };
}

test('neighbours: the file itself, then two ahead per one behind, clipped at the ends', () => {
  const order = files(40);
  const n = neighbours(order, 20);
  assert.equal(n[0], 'f20');
  assert.deepEqual(n.slice(1, 7), ['f21', 'f22', 'f19', 'f23', 'f24', 'f18']);
  assert.equal(n.length, 1 + PREFETCH_AHEAD + PREFETCH_BEHIND);
  assert.equal(new Set(n).size, n.length);
  // At the top there is nothing behind; at the end nothing ahead.
  assert.deepEqual(neighbours(order, 0), files(PREFETCH_AHEAD + 1));
  const end = neighbours(order, 39);
  assert.equal(end.length, 1 + PREFETCH_BEHIND);
  assert.ok(end.every((p) => Number(p.slice(1)) <= 39 && Number(p.slice(1)) >= 39 - PREFETCH_BEHIND));
  assert.deepEqual(neighbours([], 3), []);
});

test('searchPlan: wanted files from the reader on, wrapping, capped with a total', () => {
  const order = files(10);
  const wanted = (p: string) => p !== 'f5';
  const { paths, total } = searchPlan(order, 8, wanted, 4);
  assert.deepEqual(paths, ['f8', 'f9', 'f0', 'f1']);
  assert.equal(total, 9);
});

test('concurrency: never more than PREFETCH_MAX, nor more than the free connections', async () => {
  const h = harness();
  h.s.plan(files(20));
  assert.equal(h.started.length, PREFETCH_MAX);
  // Visible loads hold all but one connection: only one prefetch fits.
  const v = harness({ busy: NET_SLOTS - 1 });
  v.s.plan(files(20));
  assert.equal(v.started.length, 1);
  // Settling one starts the next, in plan order.
  h.release.get('f0')!();
  await flush();
  assert.deepEqual(h.started, ['f0', 'f1', 'f2', 'f3', 'f4']);
});

test('priority: a visible load starting aborts the newest prefetches and requeues them first', async () => {
  const h = harness();
  h.s.plan(files(10));
  assert.deepEqual(h.started, ['f0', 'f1', 'f2', 'f3']);
  h.state.busy = 4; // visible loads now hold 4 of 6 connections
  h.s.yield();
  assert.deepEqual(h.aborted, ['f2', 'f3']);
  assert.equal(h.s.pending, 10); // f0, f1 in flight + 8 queued (f2, f3 back at the front)
  h.state.busy = 0;
  h.release.get('f0')!();
  await flush();
  assert.deepEqual(h.started.slice(4), ['f2', 'f3', 'f4']);
});

test('a new focus drops queued work and aborts in-flight work it no longer wants', () => {
  const h = harness();
  h.s.plan(['a', 'b', 'c', 'd', 'e', 'f']);
  assert.deepEqual(h.started, ['a', 'b', 'c', 'd']);
  h.s.plan(['c', 'x', 'y']);
  assert.deepEqual(h.aborted.sort(), ['a', 'b', 'd']);
  // 'c' kept going (not restarted), 'x' and 'y' started; 'e' and 'f' never will.
  assert.deepEqual(h.started, ['a', 'b', 'c', 'd', 'x', 'y']);
  assert.ok(h.s.active('c'));
});

test('unwanted files are skipped; a full cache stops prefetching', () => {
  const h = harness({ wanted: (p) => p !== 'b' });
  h.s.plan(['a', 'b', 'c']);
  assert.deepEqual(h.started, ['a', 'c']);
  const f = harness({ full: true });
  f.s.plan(files(5));
  assert.deepEqual(f.started, []);
  assert.equal(f.s.pending, 0);
});

test('drop and reset cancel; planSoon debounces a fast scroll into one plan', async () => {
  const h = harness();
  h.s.plan(['a', 'b']);
  h.s.drop('a');
  assert.deepEqual(h.aborted, ['a']);
  h.s.reset();
  assert.deepEqual(h.aborted.sort(), ['a', 'b']);
  assert.equal(h.s.pending, 0);

  const d = harness();
  for (let i = 0; i < 50; i++) d.s.planSoon(() => [`f${i}`], 5);
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(d.started, ['f49']);
  d.s.planSoon(() => ['late'], 5);
  d.s.reset();
  await new Promise((r) => setTimeout(r, 20));
  assert.ok(!d.started.includes('late'));
});
