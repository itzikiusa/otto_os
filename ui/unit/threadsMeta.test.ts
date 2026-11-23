// Pins / History / "seen" on the thread itself (threadMeta.svelte.ts): a write
// shows at once and rolls back with the error toast when the daemon refuses,
// the old local lists move onto the threads once, and "Done" across devices
// is "the last turn ended after thread_seen_at" (threadInbox.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { loadSource } from './sourceHarness.ts';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));
const inbox = await import('../src/modules/threads/threadInbox.ts');

function load(patch: (path: string, body: unknown) => Promise<unknown>, rows: Record<string, Record<string, unknown>> = {}) {
  const toasts: string[] = [];
  const patched: [string, unknown][] = [];
  const store = { byId: rows, get: (id: string) => rows[id] ?? null, loadAll: async () => {} };
  const ls = new Map<string, string>();
  (globalThis as { localStorage?: unknown }).localStorage = { getItem: (k: string) => ls.get(k) ?? null, setItem: (k: string, v: string) => void ls.set(k, v) };
  const { threadMeta } = loadSource(new URL('../src/modules/threads/threadMeta.svelte.ts', import.meta.url), {
    '../../lib/api/client': { api: { patch: (p: string, b: unknown) => { patched.push([p, b]); return patch(p, b); } } },
    '../../lib/stores/auth.svelte': { auth: { me: { id: 'me' } } },
    '../../lib/stores/workspace.svelte': { ws: { getSession: () => null, otherWsSessions: [], archivedSessions: [], updateSessionMeta: async () => {} } },
    '../../lib/live': { onLive: () => () => {} },
    '../../lib/toastError': { toastError: (t: string) => void toasts.push(t) },
    './runtime/runtimeThreads.svelte': { runtimeThreads: store },
    './threadInbox': inbox,
  }) as { threadMeta: { write: (id: string, p: unknown) => Promise<boolean>; pinned: (id: string) => boolean; history: (id: string) => boolean; unread: (id: string, live: boolean) => boolean; migrate: (p: string[], h: string[]) => Promise<boolean>; markSeen: (id: string) => void } };
  return { threadMeta, toasts, patched, rows, ls };
}
const row = (meta: Record<string, unknown> = {}) => ({ id: 'a', status: 'idle', created_by: 'me', last_turn_status: 'completed', last_active_at: '2026-11-23T10:00:00Z', meta });

test('a pin shows at once; a refused write rolls back with the shared error toast', async () => {
  let fail: (e: Error) => void = () => {};
  const t = load(() => new Promise((_, no) => (fail = no)), { a: row() });
  const done = t.threadMeta.write('a', { thread_pinned: true });
  assert.equal(t.threadMeta.pinned('a'), true);
  fail(new Error('403 forbidden'));
  assert.equal(await done, false);
  assert.equal(t.threadMeta.pinned('a'), false);
  assert.deepEqual(t.toasts, ['Couldn’t save that change']);
  assert.equal(JSON.stringify(t.patched), JSON.stringify([['/runtime-sessions/a', { meta: { thread_pinned: true } }]]));
});

test('a write that lands keeps the daemon’s meta on the row', async () => {
  const t = load(async () => ({ meta: { thread_history: true } }), { a: row() });
  assert.equal(await t.threadMeta.write('a', { thread_history: true }), true);
  assert.equal(t.threadMeta.history('a'), true);
  assert.equal(JSON.stringify(t.rows.a.meta), JSON.stringify({ thread_history: true }));
});

test('the local lists move onto threads that still exist, once', async () => {
  const t = load(async (_p, b) => ({ meta: (b as { meta: unknown }).meta }), { a: row(), b: { ...row(), id: 'b' } });
  assert.equal(await t.threadMeta.migrate(['a', 'gone'], ['b', 'a']), true);
  assert.equal(JSON.stringify(t.patched.sort()), JSON.stringify([
    ['/runtime-sessions/a', { meta: { thread_pinned: true, thread_history: true } }],
    ['/runtime-sessions/b', { meta: { thread_history: true } }],
  ]));
  assert.equal(await t.threadMeta.migrate(['a'], []), true);
  assert.equal(t.patched.length, 2);
});

test('Done = the last turn ended after thread_seen_at; seeing it writes once', async () => {
  assert.equal(inbox.unseen('2026-11-23T10:05:00Z', '2026-11-23T10:00:00Z'), true);
  assert.equal(inbox.unseen('2026-11-23T10:05:00Z', '2026-11-23T10:06:00Z'), false);
  assert.equal(inbox.unseen('2026-11-23T10:05:00Z', undefined), false);
  assert.equal(inbox.unseen(null, '2026-11-23T10:00:00Z'), false);
  const t = load(async (_p, b) => ({ meta: { last_turn_ended_at: '2026-11-23T10:05:00Z', ...(b as { meta: object }).meta } }), {
    a: row({ last_turn_ended_at: '2026-11-23T10:05:00Z', thread_seen_at: '2026-11-23T10:00:00Z' }),
  });
  (globalThis as { document?: unknown }).document = { hidden: false };
  assert.equal(t.threadMeta.unread('a', false), true);
  t.threadMeta.markSeen('a');
  assert.equal(t.threadMeta.unread('a', true), false); // seen elsewhere beats this window's flag
  await new Promise((r) => setTimeout(r, 0));
  t.threadMeta.markSeen('a'); // nothing new: no second write
  assert.equal(t.patched.length, 1);
  // Never marked seen anywhere: this window's own flag decides.
  const fresh = load(async () => ({}), { b: { ...row(), id: 'b' } });
  assert.equal(fresh.threadMeta.unread('b', true), true);
  assert.equal(fresh.threadMeta.unread('b', false), false);
  assert.deepEqual(inbox.migrationPatches(['x'], ['x', 'y'], (id) => id !== 'y'), [['x', { thread_pinned: true, thread_history: true }]]);
});
