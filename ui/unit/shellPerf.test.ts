// Shell main-thread multipliers (perf backlog B3): pure halves of the fixes —
// session events patch one list in place, the notices list is capped, the
// frecency map is parsed once per stored string, drag moves coalesce per frame.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { patchSessionIn } from '../src/lib/stores/sessionPatch.ts';
import { rafCoalesce } from '../src/lib/rafCoalesce.ts';
import { FRECENCY_KEY, loadFrecency, recordUsage } from '../src/lib/commandSearch.ts';
import { loadSource } from './sourceHarness.ts';

test('patchSessionIn keeps the array when the id is absent or the patch is a no-op', () => {
  const a = { id: 'a', status: 'idle' };
  const b = { id: 'b', status: 'idle' };
  const list = [a, b];
  assert.equal(patchSessionIn(list, 'zz', (s) => ({ ...s, status: 'working' })), list, 'absent → same array');
  assert.equal(patchSessionIn(list, 'a', (s) => s), list, 'unchanged → same array');
  const next = patchSessionIn(list, 'b', (s) => ({ ...s, status: 'working' }));
  assert.notEqual(next, list);
  assert.equal(next[0], a, 'other sessions keep their object');
  assert.deepEqual(next[1], { id: 'b', status: 'working' });
  assert.deepEqual(list[1], { id: 'b', status: 'idle' }, 'input not mutated');
});

test('rafCoalesce applies only the latest move per frame, and flush applies it now', () => {
  const frames: Array<() => void> = [];
  const g = globalThis as { requestAnimationFrame?: unknown; cancelAnimationFrame?: unknown };
  g.requestAnimationFrame = (cb: () => void) => frames.push(cb);
  g.cancelAnimationFrame = (id: number) => {
    frames[id - 1] = () => {};
  };
  try {
    const seen: number[] = [];
    const c = rafCoalesce((x: number) => seen.push(x));
    c.push(1);
    c.push(2);
    c.push(3);
    assert.equal(frames.length, 1, 'one frame requested for a burst');
    frames[0]();
    assert.deepEqual(seen, [3]);
    c.push(4);
    c.flush();
    assert.deepEqual(seen, [3, 4], 'flush (mouseup) applies the pending move synchronously');
    frames[1](); // the cancelled frame is a no-op
    assert.deepEqual(seen, [3, 4]);
    c.push(5);
    c.cancel();
    frames[2]();
    assert.deepEqual(seen, [3, 4]);
  } finally {
    delete g.requestAnimationFrame;
    delete g.cancelAnimationFrame;
  }
});

test('loadFrecency parses once per stored string and recordUsage never edits the shared map', () => {
  const store = new Map<string, string>();
  let gets = 0;
  const g = globalThis as { localStorage?: unknown };
  const realParse = JSON.parse;
  let parses = 0;
  JSON.parse = ((text: string, reviver?: (this: unknown, key: string, value: unknown) => unknown) => {
    parses++;
    return realParse(text, reviver);
  }) as typeof JSON.parse;
  g.localStorage = {
    getItem: (k: string) => (gets++, store.get(k) ?? null),
    setItem: (k: string, v: string) => void store.set(k, v),
  };
  try {
    store.set(FRECENCY_KEY, JSON.stringify({ x: { count: 1, lastUsed: 1 } }));
    parses = 0;
    const a = loadFrecency();
    const b = loadFrecency();
    assert.equal(a, b, 'same stored string → the memoized map');
    assert.equal(parses, 1);
    recordUsage('y', 5);
    assert.equal(a.y, undefined, 'the memo handed out earlier is not mutated');
    const c = loadFrecency();
    assert.notEqual(c, a, 'a new stored string re-parses');
    assert.deepEqual(c.y, { count: 1, lastUsed: 5 });
    assert.ok(gets >= 3);
  } finally {
    JSON.parse = realParse;
    delete g.localStorage;
  }
});

function notificationsModule() {
  return loadSource(new URL('../src/lib/stores/notifications.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../api/client': { api: { get: async () => [], post: async () => ({}), del: async () => ({}) } },
    '../toast.svelte': { toasts: { warn() {}, info() {} } },
    '../external': { openExternal: async () => {} },
    './workspace.svelte': { ws: { sessions: [] } },
    '../desktop': { isEmbedded: false },
  });
}

test('capNotices drops the oldest read notices first, unread only past the cap', () => {
  const { capNotices } = notificationsModule();
  const n = (id: string, read: boolean) => ({ id, read });
  const list = [n('1', false), n('2', true), n('3', false), n('4', true), n('5', true)];
  assert.equal(capNotices(list, 5), list, 'at the cap → untouched');
  assert.deepEqual(capNotices(list, 3).map((x: { id: string }) => x.id), ['1', '2', '3'], 'oldest read (5, 4) go first');
  assert.deepEqual(capNotices(list, 1).map((x: { id: string }) => x.id), ['1'], 'then the oldest unread');
});

test('ingest keeps at most NOTICE_CAP notices', () => {
  const { notifications, NOTICE_CAP } = notificationsModule();
  notifications.settings.native_enabled = false;
  for (let i = 0; i < NOTICE_CAP + 50; i++) {
    notifications.ingest({ id: `n${i}`, created_at: `2026-09-24T10:00:${String(i % 60).padStart(2, '0')}Z`, kind: 'info', severity: 'info', title: 't', body: 'b', read: i < 100, source_key: null, action: null });
  }
  assert.equal(notifications.notices.length, NOTICE_CAP);
  assert.equal(notifications.notices[0].id, `n${NOTICE_CAP + 49}`, 'newest kept on top');
  assert.equal(notifications.notices.filter((x: { read: boolean }) => x.read).length, 50, 'the 50 dropped were the oldest READ ones');
});
