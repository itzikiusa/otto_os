// Database Explorer restore: every tab chip at once, the ACTIVE connection
// dials first, the rest warm in the background at most 3 at a time without
// moving focus, and a close mid-warm writes nothing back.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

type Call = { method: string; path: string; lane: 'int' | 'bg' };

function setup(ids: string[], selected: string, opts: { onClick?: boolean } = {}) {
  const calls: Call[] = [];
  const schema = new Map<string, ReturnType<typeof deferred<unknown>>>();
  let bgInFlight = 0;
  let bgPeak = 0;
  const route = (method: string, path: string, lane: 'int' | 'bg'): Promise<unknown> => {
    calls.push({ method, path, lane });
    const m = /^\/connections\/([^/]+)\/db\/schema$/.exec(path);
    if (m) {
      const d = deferred<unknown>();
      schema.set(`${lane}:${m[1]}`, d);
      if (lane === 'bg') {
        bgInFlight++;
        bgPeak = Math.max(bgPeak, bgInFlight);
        return d.promise.finally(() => bgInFlight--);
      }
      return d.promise;
    }
    if (path.endsWith('/capabilities')) {
      return Promise.resolve({ engine: 'mysql', query_language: 'sql', joins: true });
    }
    return Promise.resolve(path.includes('/test') ? { ok: true, message: '' } : []);
  };
  class ApiError extends Error {}
  const api = {
    get: (p: string) => route('GET', p, 'int'),
    post: (p: string) => route('POST', p, 'int'),
    patch: (p: string) => route('PATCH', p, 'int'),
    del: (p: string) => route('DELETE', p, 'int'),
    bg: {
      get: (p: string) => route('GET', p, 'bg'),
      post: (p: string) => route('POST', p, 'bg'),
    },
  };
  const poll = loadSource(new URL('../src/lib/poll.ts', import.meta.url), {
    './api/lane': { inLane: (_l: unknown, fn: () => unknown) => fn(), tagSignal: () => {} },
  });
  const noop = () => {};
  const store = new Map<string, string>();
  const localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  };
  const mod = loadSource(
    new URL('../src/lib/stores/database.svelte.ts', import.meta.url),
    {
      '../api/client': {
        api,
        ApiError,
        isAbortError: (e: unknown) => e instanceof DOMException && e.name === 'AbortError',
        dbAssistStart: noop,
        dbAssistSummary: noop,
        dbAssistClose: noop,
        dbCloseConnection: () => Promise.resolve(),
      },
      './auth.svelte': { auth: { isRoot: true, me: null, can: () => true } },
      './resource-access.svelte': { resourceAccess: { subscribe: noop } },
      '../confirm.svelte': { confirmer: { ask: async () => true } },
      './workspace.svelte': { ws: { currentId: 'w1' } },
      '../toast.svelte': { toasts: { error: noop, info: noop, warn: noop, success: noop } },
      '../router.svelte': { router: { module: 'database' } },
      '../components/exporters': { downloadText: noop },
      '../../modules/database/mongo-format': { formatMongo: (s: string) => s },
      '../../modules/database/sql-util': {
        defaultVarSpec: (v: string) => ({ value: v, type: 'string', escape: true }),
        stripJavaStringConcat: (s: string) => s,
        maskQueryPlaceholders: (s: string) => s,
        unmaskQueryPlaceholders: (s: string) => s,
      },
      '../../modules/database/bson': { bsonScalar: (v: unknown) => v },
      '../clipboard': { copyTextOrThrow: async () => {} },
      '../poll': poll,
      '../editor-history': {
        forgetEditorState: noop,
        forgetEditorStates: noop,
        hydrateEditorHistory: async () => {},
      },
      '../../modules/database/grid-tab-state': { dropGridState: noop },
      '../../modules/database/error-normalize': { normalizeDbError: (_e: unknown, msg: string) => ({ title: msg }) },
      './clipHistory.svelte': { clipHistory: { setGuard: noop } },
    },
    { localStorage, crypto: globalThis.crypto, performance: globalThis.performance, sessionStorage: { getItem: () => null, setItem: noop, removeItem: noop } },
  );
  const db = mod.database;
  if (opts.onClick) db.setWarmRestored('on-click');
  db.connections = ids.map((id) => ({ id, name: id, kind: 'mysql' }));
  localStorage.setItem('otto_db_open', JSON.stringify({ open: ids, selected }));
  return { db, calls, schema, peak: () => bgPeak };
}

const flush = () => new Promise((r) => setTimeout(r, 0));
const schemaCalls = (calls: Call[]) =>
  calls.filter((c) => /\/db\/schema$/.test(c.path)).map((c) => `${c.lane}:${c.path.split('/')[2]}`);

test('restore: active connection first, the rest warm 3 at a time without stealing focus', async () => {
  const ids = ['a', 'b', 'c', 'd', 'e'];
  const { db, calls, schema, peak } = setup(ids, 'c');
  const done = db.restoreWorkbench();
  await flush();
  // Every chip at once; the others are "not connected yet" (or connecting).
  assert.deepEqual([...db.openConnIds], ids);
  assert.equal(db.selectedConnId, 'c');
  const first = schemaCalls(calls);
  assert.equal(first[0], 'int:c', 'the selected connection dials first');
  assert.deepEqual(first.slice(1).sort(), ['bg:a', 'bg:b', 'bg:d'], 'three background warms in flight');
  assert.equal(db.connStatus.get('e').phase, 'idle');
  // The active one lands; the warms keep the selection where it is.
  schema.get('int:c')!.resolve([{ id: 'db:x', label: 'x', kind: 'database' }]);
  await done;
  schema.get('bg:a')!.resolve([]);
  await flush();
  await flush();
  assert.ok(schemaCalls(calls).includes('bg:e'), 'the 4th background warm starts once a slot frees');
  for (const id of ['b', 'd', 'e']) schema.get(`bg:${id}`)!.resolve([]);
  await flush();
  await flush();
  assert.ok(peak() <= 3, `at most 3 concurrent warms (saw ${peak()})`);
  assert.equal(db.selectedConnId, 'c', 'focus never moved during the warm-up');
  assert.equal(db.connStatus.get('b').phase, 'ready');
});

test('restore: a close during the warm-up writes no snapshot', async () => {
  const { db, schema } = setup(['a', 'b'], 'a');
  const done = db.restoreWorkbench();
  await flush();
  schema.get('int:a')!.resolve([]);
  await done;
  db.closeConnection('b');
  schema.get('bg:b')?.resolve([]);
  await flush();
  await flush();
  assert.equal(db.snapshots.has('b'), false);
  assert.equal(db.connStatus.has('b'), false);
});

test('restore: "connect on click only" leaves the other tabs idle', async () => {
  const { db, calls, schema } = setup(['a', 'b', 'c'], 'b', { onClick: true });
  const done = db.restoreWorkbench();
  await flush();
  schema.get('int:b')!.resolve([]);
  await done;
  await flush();
  assert.deepEqual(schemaCalls(calls), ['int:b']);
  assert.equal(db.connStatus.get('a').phase, 'idle');
  assert.equal(db.connStatus.get('c').phase, 'idle');
});

test('a tab keeps its uid and its run start time across a reload', async () => {
  const { db, schema } = setup(['a'], 'a');
  const done = db.restoreWorkbench();
  await flush();
  schema.get('int:a')!.resolve([]);
  await done;
  const t = db.tabs[0];
  assert.match(t.uid, /\S{8,}/);
  t.pending = { queryId: 'q1', connId: 'a', sql: 'SELECT SLEEP(5)', node: null, startedAt: 1_700_000_000_000 };
  db.persistTabsNow('a');
  const back = db.restoreTabs('a');
  assert.equal(back.tabs[0].uid, t.uid, 'the undo-history key survives');
  assert.equal(back.tabs[0].pending.startedAt, 1_700_000_000_000, 'the timer keeps counting from the real start');
  assert.notEqual(back.tabs[0].id, undefined);
});

test('a daemon restart marks open connections stale and re-warms them', async () => {
  const { db, calls, schema } = setup(['a', 'b'], 'a');
  const done = db.restoreWorkbench();
  await flush();
  schema.get('int:a')!.resolve([]);
  await done;
  schema.get('bg:b')!.resolve([{ id: 'db:x', label: 'x', kind: 'database' }]);
  await flush();
  await flush();
  assert.equal(db.connStatus.get('b').phase, 'ready');
  const before = schemaCalls(calls).length;
  db.onDaemonRestart();
  await flush();
  const after = schemaCalls(calls).slice(before);
  assert.deepEqual([...after].sort(), ['bg:b', 'int:a'], 'selected reloads, the other re-warms');
  assert.equal(db.connStatus.get('b').phase, 'connecting');
  schema.get('bg:b')!.resolve([{ id: 'db:y', label: 'y', kind: 'database' }]);
  await flush();
  await flush();
  assert.equal(db.snapshots.get('b').schemaRoot[0].id, 'db:y', 'the parked tree is refilled in place');
});
