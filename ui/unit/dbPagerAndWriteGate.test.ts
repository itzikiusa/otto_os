// Database Explorer store: the footer pager's offset follows the page that is
// actually on screen (ui-correctness-2-05), and the typed guarded-write gate is
// titled by environment and explains a mistyped name (ux-design-2-02).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import * as resultBudgetMod from '../src/lib/stores/db-result-budget.ts';
import * as filterChips from '../src/modules/database/filter-chips.ts';
import * as sqlDialect from '../src/modules/database/sql-dialect.ts';

class ApiError extends Error {}

function setup(opts: { typed?: string | null } = {}) {
  const noop = () => {};
  const toasts: string[] = [];
  const prompts: { message: string; title: string }[] = [];
  let nextQuery: () => Promise<unknown> = async () => ({ columns: [], rows: [] });
  const queries: any[] = [];
  const api = {
    get: async () => [],
    post: async (path: string, body: any) => {
      if (path.endsWith('/query')) {
        queries.push(body);
        return nextQuery();
      }
      return {};
    },
    patch: async () => ({}),
    del: async () => ({}),
    bg: { get: async () => [], post: async () => ({}) },
  };
  const poll = loadSource(new URL('../src/lib/poll.ts', import.meta.url), {
    './api/lane': { inLane: (_l: unknown, fn: () => unknown) => fn(), tagSignal: () => {} },
  });
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
        withDeadline: (p: Promise<unknown>) => p,
        READ_DEADLINE_MS: 1,
      },
      './auth.svelte': { auth: { isRoot: true, me: null, can: () => true } },
      './resource-access.svelte': { resourceAccess: { subscribe: noop } },
      '../confirm.svelte': {
        confirmer: {
          ask: async () => true,
          promptText: async (message: string, o: { title: string }) => {
            prompts.push({ message, title: o.title });
            return opts.typed ?? null;
          },
        },
      },
      './workspace.svelte': { ws: { currentId: 'w1' } },
      '../toast.svelte': {
        toasts: {
          error: (t: string) => toasts.push(`error:${t}`),
          info: (t: string) => toasts.push(`info:${t}`),
          warn: noop,
          success: noop,
        },
      },
      '../toastError': { toastError: () => {} },
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
      '../../modules/database/filter-chips': filterChips,
      '../../modules/database/sql-dialect': sqlDialect,
      '../clipboard': { copyTextOrThrow: async () => {} },
      '../poll': poll,
      '../editor-history': { forgetEditorState: noop, forgetEditorStates: noop, hydrateEditorHistory: async () => {} },
      '../../modules/database/grid-tab-state': { dropGridState: noop, parkedEditCount: () => 0, releaseGridResult: noop },
      './db-result-budget': resultBudgetMod,
      '../../modules/database/error-normalize': { normalizeDbError: (_e: unknown, msg: string) => ({ title: msg }) },
      './clipHistory.svelte': { clipHistory: { setGuard: noop } },
      './dbPrefs.svelte': {
        dbPrefs: { warmRestored: 'background', keepAlive: false, setWarmRestored: noop, setKeepAlive: noop, onKeepAliveChange: () => noop },
        loadFlag: (_k: string, def: boolean) => def,
        saveFlag: noop,
      },
      '../lazyModule': { announceModule: noop },
      '../loadError': { loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)) },
    },
    { localStorage, crypto: globalThis.crypto, performance: globalThis.performance, sessionStorage: { getItem: () => null, setItem: noop, removeItem: noop } },
  );
  const db = mod.database;
  db.connections = [{ id: 'a', name: 'orders', kind: 'mysql', environment: 'dev' }];
  db.selectedConnId = 'a';
  db.openConnIds = ['a'];
  // A tab showing page 1 of an auto-limited result.
  const t = db.tab;
  t.result = { columns: ['id'], rows: [[1]], auto_limited: 50 };
  t.ran_statement = 'SELECT * FROM orders';
  t.ran_node = null;
  t.offset = 0;
  return { db, t, toasts, prompts, queries, setNext: (fn: () => Promise<unknown>) => (nextQuery = fn) };
}

test('a failed Next leaves the pager on the page still on screen', async () => {
  const h = setup();
  h.setNext(async () => { throw new ApiError('boom'); });
  const r = await h.db.runPage(1);
  assert.equal(r, null);
  assert.equal(h.queries[0].offset, 50, 'the page request asked for the next page');
  assert.equal(h.t.offset, 0, 'the footer must not say 51–100 over rows 1–50');
});

test('a stopped Next leaves the pager on the page still on screen', async () => {
  const h = setup();
  const pending = deferred<unknown>();
  h.setNext(() => pending.promise);
  const paging = h.db.runPage(1);
  await new Promise((r) => setTimeout(r, 0));
  h.db.abortQuery(h.t.id, { report: false });
  pending.reject(new DOMException('stopped', 'AbortError'));
  assert.equal(await paging, null);
  assert.equal(h.t.offset, 0);
});

test('a successful Next keeps the new offset', async () => {
  const h = setup();
  h.setNext(async () => ({ columns: ['id'], rows: [[51]], auto_limited: 50 }));
  const r = await h.db.runPage(1);
  assert.ok(r);
  assert.equal(h.t.offset, 50);
});

test('guarded-write gate: titled by environment, no glyph', async () => {
  const h = setup({ typed: null });
  assert.equal(await h.db.confirmGuardedWrite({ name: 'orders', environment: 'dev' }), 'declined');
  assert.equal(await h.db.confirmGuardedWrite({ name: 'orders', environment: 'prod' }), 'declined');
  assert.deepEqual(h.prompts.map((p) => p.title), ['Confirm write on a read-only connection', 'Confirm production write']);
  assert.deepEqual(h.toasts, [], 'a plain cancel is reported by the caller, not the gate');
});

test('guarded-write gate: a mistyped name is explained and answers mismatch', async () => {
  const h = setup({ typed: 'order' });
  assert.equal(await h.db.confirmGuardedWrite({ name: 'orders', environment: 'prod' }), 'mismatch');
  assert.deepEqual(h.toasts, ['info:Nothing was changed']);
  const ok = setup({ typed: '  ORDERS ' });
  assert.equal(await ok.db.confirmGuardedWrite({ name: 'orders', environment: 'prod' }), 'confirmed');
});
