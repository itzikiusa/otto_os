import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { randomUUID } from 'node:crypto';
import ts from 'typescript';
import { deferred, loadSource } from './sourceHarness.ts';
import * as resultBudget from '../src/lib/stores/db-result-budget.ts';
import { strictRequire, unused } from './strictRequire.ts';

class ApiError extends Error {}
const result = { columns: [], rows: [], affected_rows: 1, elapsed_ms: 1 };
function setup(options: { holdCancel?: boolean; holdReplacement?: boolean } = {}) {
  const refused = deferred<any>();
  const cancelled = deferred<any>();
  const replacement = deferred<any>();
  const cancels: { url: string; body: any; signal?: AbortSignal }[] = [];
  const approval = deferred<string | null>();
  const prompted = deferred<void>();
  const queries: { url: string; body: any; signal?: AbortSignal }[] = [];
  const prompts: { text: string; placeholder: string }[] = [];
  let expireCancel!: () => void;
  const deadlines: number[] = [];
  const transport = loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, {
    // Drive the real deadline helper deterministically, without a wall-clock wait.
    setTimeout: (fn: () => void, ms: number) => { expireCancel = fn; deadlines.push(ms); return 1; },
    clearTimeout() {},
  });
  const api = {
    get: async () => [],
    post: async (url: string, body: any, signal?: AbortSignal) => {
      if (url.endsWith('/cancel')) {
        cancels.push({ url, body, signal });
        return options.holdCancel ? new Promise((resolve, reject) => {
          signal?.addEventListener('abort', () => reject(signal.reason), { once: true });
          cancelled.promise.then(resolve, reject);
        }) : { status: 'cancelled' };
      }
      if (!url.endsWith('/query')) return {};
      queries.push({ url, body, signal });
      return queries.length === 1 ? refused.promise : options.holdReplacement ? replacement.promise : result;
    },
  };
  let source = readFileSync(new URL('../src/lib/stores/database.svelte.ts', import.meta.url), 'utf8');
  // Evaluate each production derived expression when read, since the identity
  // rune shim cannot track selection changes. All methods/initializers remain
  // production source, including abortQuery and the access-revocation handler.
  const ast = ts.createSourceFile('store.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const edits: { start: number; end: number; text: string }[] = [];
  function visit(node: ts.Node) {
    if (ts.isPropertyDeclaration(node) && node.initializer && ts.isCallExpression(node.initializer) && node.initializer.expression.getText(ast) === '$derived') {
      edits.push({ start: node.getStart(ast), end: node.end, text: `get ${node.name.getText(ast)}() { return ${node.initializer.arguments[0].getText(ast)}; }` });
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  for (const edit of edits.sort((a, b) => b.start - a.start)) source = source.slice(0, edit.start) + edit.text + source.slice(edit.end);
  const storage = new Map<string, string>();
  const context = {
    exports: {} as Record<string, any>, Error, DOMException, AbortController, crypto: { randomUUID },
    setTimeout, clearTimeout, console,
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value), removeItem: (key: string) => storage.delete(key) },
    $state: Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v }),
    $derived: Object.assign((v: unknown) => v, { by: (fn: () => unknown) => fn() }),
    require: (path: string) => {
      if (path.endsWith('/client')) return { api, ApiError, withDeadline: transport.withDeadline, READ_DEADLINE_MS: transport.READ_DEADLINE_MS, isAbortError: (e: Error) => e.name === 'AbortError' };
      if (path.endsWith('/workspace.svelte')) return { ws: { currentId: 'workspace' } };
      if (path.endsWith('/auth.svelte')) return { auth: { user: { id: 'user' } } };
      if (path.endsWith('/resource-access.svelte')) return { resourceAccess: { subscribe() {}, can: () => true } };
      if (path.endsWith('/confirm.svelte')) return { confirmer: { promptText: (text: string, options: any) => { prompts.push({ text, placeholder: options.placeholder }); prompted.resolve(); return approval.promise; } } };
      if (path.endsWith('/toast.svelte')) return { toasts: { error() {}, info() {}, warn() {} } };
      if (path.endsWith('/router.svelte')) return { router: { module: 'database' } };
      if (path.endsWith('/clipHistory.svelte')) return { clipHistory: { setGuard() {} } };
      if (path.endsWith('/dbPrefs.svelte')) return {
        dbPrefs: { warmRestored: 'background', keepAlive: true, onKeepAliveChange() {}, setKeepAlive() {}, setWarmRestored() {} },
        loadFlag: (_key: string, def: boolean) => def,
        saveFlag() {},
      };
      if (path.endsWith('/lazyModule')) return { announceModule() {} };
      if (path.endsWith('/db-result-budget')) return resultBudget;
      if (path.endsWith('/grid-tab-state')) return { parkedEditCount: () => 0 };
      // Reachable only from flows this file doesn't drive: any use throws.
      return strictRequire(['/api/types', '/components/exporters', '/mongo-format', '/sql-util', '/bson', '/filter-chips', '/sql-dialect', '/error-normalize',
        '/clipboard', '/poll', '/editor-history', '/loadError', '/toastError'].map((p) => [p, unused(p)] as const))(path);
    },
  };
  runInNewContext(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const v = context.exports.database;
  v.connections = [{ id: 'A', name: 'Production A', environment: 'prod', kind: 'postgres' }, { id: 'B', name: 'Development B', environment: 'dev', kind: 'postgres' }];
  v.selectedConnId = 'A'; v.openConnIds = ['A', 'B']; v.activeDb = 'db:app';
  v.capabilities = { engine: 'postgres' };
  return { v, refused, approval, prompted, queries, prompts, cancelled, replacement, cancels, deadlines, storage, expireCancel: () => expireCancel() };
}

for (const entry of ['runManagedStatement', 'runQuery']) {
  test(`${entry}: switching connections before refusal never asks to approve B for a write to A`, async () => {
    const h = setup();
    const pending = h.v[entry]('DELETE FROM orders', 'db:app');
    h.v.selectedConnId = 'B';
    h.refused.reject(new ApiError('write_blocked: production'));
    // An implementation may cancel the superseded run before asking. Resolve
    // ahead of time so either valid policy completes without a hanging test.
    h.approval.resolve('Production A');
    await pending;
    for (const prompt of h.prompts) {
      assert.match(prompt.text, /Production A/);
      assert.match(prompt.text, /PRODUCTION/);
      assert.doesNotMatch(prompt.text, /Development B/);
      assert.equal(prompt.placeholder, 'Production A');
    }
    for (const query of h.queries) assert.equal(query.url, '/connections/A/db/query');
    if (h.queries.length > 1) assert.equal(h.queries[1].body.confirm_write, true);
  });

  test(`${entry}: accepted confirmation retries exactly the origin statement and scope`, async () => {
    const h = setup();
    const pending = h.v[entry]('DELETE FROM orders', 'db:app');
    h.refused.reject(new ApiError('write_blocked: production'));
    await h.prompted.promise;
    h.approval.resolve('Production A');
    assert.ok(await pending);
    assert.equal(h.queries.length, 2);
    assert.equal(h.queries[1].body.statement, 'DELETE FROM orders');
    assert.equal(h.queries[1].body.node, 'db:app');
    assert.equal(h.queries[1].body.confirm_write, true);
  });

  test(`${entry}: declined typed confirmation never retries`, async () => {
    const h = setup();
    const pending = h.v[entry]('DELETE FROM orders');
    h.refused.reject(new ApiError('write_blocked: production'));
    await h.prompted.promise; h.approval.resolve(null);
    assert.equal(await pending, null); assert.equal(h.queries.length, 1);
  });

  test(`${entry}: revoking access while confirmation waits prevents the retry`, async () => {
    const h = setup();
    const pending = h.v[entry]('DELETE FROM orders');
    h.refused.reject(new ApiError('write_blocked: production'));
    await h.prompted.promise;
    h.v.onAccessChange({ type: 'decision', kind: 'connection', id: 'A', before: { operations: { discover: { allowed: true }, db_query: { allowed: true } } }, after: { operations: { discover: { allowed: true }, db_query: { allowed: false } } } });
    h.approval.resolve('Production A');
    assert.equal(await pending, null); assert.equal(h.queries.length, 1);
  });
}

test('runQuery: Stop during confirmation prevents a late accepted retry', async () => {
  const h = setup();
  const pending = h.v.runQuery('DELETE FROM orders');
  h.refused.reject(new ApiError('write_blocked: production'));
  await h.prompted.promise; h.v.abortQuery(); h.approval.resolve('Production A');
  assert.equal(await pending, null); assert.equal(h.queries.length, 1);
});

test('agent read-only refusal retains the original production guard after switching to development', async () => {
  const h = setup(); let untypedApprovals = 0;
  const pending = h.v.runQuery('DELETE FROM orders', null, { readOnly: true, confirmWrite: async () => { untypedApprovals++; return true; }, agentLabel: 'Review agent' });
  h.v.selectedConnId = 'B';
  h.refused.reject(new ApiError('read_only: writes refused'));
  h.approval.resolve('Production A');
  await pending;
  assert.equal(untypedApprovals, 0, 'switching to development must not downgrade the production gate');
  for (const prompt of h.prompts) { assert.match(prompt.text, /Production A/); assert.match(prompt.text, /Review agent/); }
  if (h.queries.length > 1) assert.equal(h.queries[1].body.confirm_write, true);
});


test('Stop keeps the query transport alive for native cancel, then aborts only that run', async () => {
  const h = setup({ holdCancel: true, holdReplacement: true });
  const oldRun = h.v.runQuery('SELECT 1');
  const oldQuery = h.queries[0];
  h.v.abortQuery();
  assert.equal(h.v.tab.running, false);
  assert.equal(h.v.tab.pending, null);
  assert.equal(h.cancels[0].url, '/connections/A/db/cancel');
  assert.equal(h.cancels[0].body.query_id, oldQuery.body.query_id);
  assert.equal(oldQuery.signal?.aborted, false, 'native cancellation must arrive before aborting its HTTP owner');
  const newRun = h.v.runQuery('SELECT 2');
  h.cancelled.resolve({ status: 'cancelled' });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(oldQuery.signal?.aborted, true);
  assert.equal(h.queries[1].signal?.aborted, false, 'late cancel must not abort the replacement');
  h.refused.reject(new ApiError('query interrupted'));
  await oldRun;
  assert.equal(h.v.tab.pending?.queryId, h.queries[1].body.query_id);
  assert.equal(h.v.tab.error, null);
  h.replacement.resolve(result);
  assert.ok(await newRun);
});

test('a failed cancellation still releases the old HTTP wait', async () => {
  const h = setup({ holdCancel: true });
  const run = h.v.runQuery('SELECT 1');
  h.v.abortQuery();
  assert.equal(h.queries[0].signal?.aborted, false);
  h.cancelled.reject(new Error('cancel endpoint unavailable'));
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(h.queries[0].signal?.aborted, true);
  h.refused.reject(new DOMException('Aborted', 'AbortError'));
  assert.equal(await run, null);
});

for (const error of ['query interrupted', 'write_blocked: production']) {
  test(`a stopped run's late ${error} cannot alter or prompt for its replacement`, async () => {
    const h = setup({ holdCancel: true, holdReplacement: true });
    const oldRun = h.v.runQuery('DELETE FROM orders');
    h.v.abortQuery();
    const newRun = h.v.runQuery('SELECT 2');
    h.refused.reject(new ApiError(error));
    h.approval.resolve('Production A');
    assert.equal(await oldRun, null);
    assert.equal(h.prompts.length, 0);
    assert.equal(h.queries.length, 2);
    assert.equal(h.v.tab.pending?.queryId, h.queries[1].body.query_id);
    assert.equal(h.v.tab.error, null);
    assert.equal(h.v.tab.running, true);
    h.cancelled.resolve({ status: 'cancelled' });
    h.replacement.resolve(result);
    assert.ok(await newRun);
  });
}

test('Stop during confirmation preserves a replacement run even before cancel responds', async () => {
  const h = setup({ holdCancel: true, holdReplacement: true });
  const oldRun = h.v.runQuery('DELETE FROM orders');
  h.refused.reject(new ApiError('write_blocked: production'));
  await h.prompted.promise;
  h.v.abortQuery();
  const newRun = h.v.runQuery('SELECT 2');
  h.approval.resolve('Production A');
  assert.equal(await oldRun, null);
  assert.equal(h.queries.length, 2);
  assert.equal(h.v.tab.pending?.queryId, h.queries[1].body.query_id);
  h.cancelled.resolve({ status: 'cancelled' });
  h.replacement.resolve(result);
  assert.ok(await newRun);
});


test('a stalled cancel reaches its transport deadline and releases the original query', async () => {
  const h = setup({ holdCancel: true });
  const run = h.v.runQuery('SELECT 1');
  h.v.abortQuery();
  assert.equal(h.queries[0].signal?.aborted, false);
  assert.deepEqual(h.deadlines, [20_000]);
  h.expireCancel();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(h.cancels[0].signal?.aborted, true);
  assert.equal(h.queries[0].signal?.aborted, true);
  h.refused.reject(new DOMException('Aborted', 'AbortError'));
  assert.equal(await run, null);
});


for (const late of ['result', 'write_blocked: production', 'read_only: writes refused']) {
  test(`Stop by captured ID clears a parked tab and ignores its late ${late}`, async () => {
    const h = setup({ holdCancel: true, holdReplacement: true });
    const oldTab = h.v.tab;
    const oldRun = h.v.runQuery('DELETE FROM orders', null, { readOnly: true });
    // Use the production snapshot boundary: connection switches retain tab
    // objects by reference while replacing the active connection's tab list.
    h.v.captureSnapshot();
    h.v.selectedConnId = 'B'; h.v.tabs = []; h.v.newTab('SELECT 2');
    const newRun = h.v.runQuery();
    h.v.flushTabDrafts();
    const savedB = h.storage.get(h.v.tabsKey('B'));
    assert.notEqual(h.v.tab.id, oldTab.id, 'tab IDs are unique across connections');
    h.v.abortQuery(oldTab.id);
    try {
      assert.equal(oldTab.running, false);
      assert.equal(oldTab.pending, null);
      assert.equal(JSON.parse(h.storage.get(h.v.tabsKey('A'))!).tabs[0].pending, undefined,
        'the stopped marker is removed from the owning connection on disk');
      assert.equal(h.storage.get(h.v.tabsKey('B')), savedB);
      assert.equal(h.queries[0].signal?.aborted, false);
      if (late === 'result') h.refused.resolve(result);
      else h.refused.reject(new ApiError(late));
      h.approval.resolve('Production A');
      assert.equal(await oldRun, null);
      assert.equal(h.prompts.length, 0);
      assert.equal(oldTab.result, null);
      assert.equal(oldTab.error, null);
      assert.equal(h.v.tab.pending?.queryId, h.queries[1].body.query_id);
      assert.equal(h.v.tab.running, true);
      assert.equal(h.queries[1].signal?.aborted, false);
    } finally {
      h.refused.resolve(result); h.approval.resolve(null);
      h.cancelled.resolve({ status: 'cancelled' }); h.replacement.resolve(result);
      await Promise.all([oldRun, newRun]);
    }
  });
}

test('Stop without a retained tab revokes run ownership before delayed cancellation completes', async () => {
  const h = setup({ holdCancel: true });
  const oldTab = h.v.tab;
  const oldRun = h.v.runQuery('DELETE FROM orders');
  // The controller fallback also covers tabs no longer retained in snapshots.
  h.v.tabs = [];
  h.v.abortQuery(oldTab.id);
  h.refused.reject(new ApiError('write_blocked: production'));
  h.approval.resolve('Production A');
  try {
    assert.equal(await oldRun, null);
    assert.equal(h.prompts.length, 0);
    assert.equal(h.queries.length, 1);
  } finally {
    h.cancelled.resolve({ status: 'cancelled' });
  }
});
