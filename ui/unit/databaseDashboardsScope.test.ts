// DB dashboards/widgets loads are workspace-scoped and latest-wins: a late
// response from a workspace the user has left must never land under the new
// one (rename/delete/add-widget act on those ids by id alone), a switch drops
// the old lists, and a superseded load never clears the newer load's spinner.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { randomUUID } from 'node:crypto';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';
import * as resultBudget from '../src/lib/stores/db-result-budget.ts';
import { loadErrorText } from '../src/lib/loadError.ts';
import { strictRequire, unused } from './strictRequire.ts';

class ApiError extends Error {}

function setup() {
  const pending = new Map<string, ReturnType<typeof deferred<any>>[]>();
  const gets: string[] = [];
  const toasts: string[] = [];
  const api = {
    get: (url: string) => {
      gets.push(url);
      const d = deferred<any>();
      pending.set(url, [...(pending.get(url) ?? []), d]);
      return d.promise;
    },
    patch: async () => { throw new ApiError('boom'); },
  };
  const ws = { currentId: 'A' as string | null };
  let source = readFileSync(new URL('../src/lib/stores/database.svelte.ts', import.meta.url), 'utf8');
  // Evaluate each production $derived when read (the identity rune shim can't track).
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
      if (path.endsWith('/client')) return { api, ApiError, withDeadline: (p: unknown) => p, READ_DEADLINE_MS: 1, isAbortError: (e: Error) => e.name === 'AbortError' };
      if (path.endsWith('/workspace.svelte')) return { ws };
      if (path.endsWith('/auth.svelte')) return { auth: { user: { id: 'user' } } };
      if (path.endsWith('/resource-access.svelte')) return { resourceAccess: { subscribe() {}, can: () => true } };
      if (path.endsWith('/toast.svelte')) return { toasts: { error: (t: string) => toasts.push(t), info() {}, warn() {}, success() {} } };
      if (path.endsWith('/toastError')) return { toastError: (t: string, e: unknown) => toasts.push(`${t}: ${loadErrorText(e)}`) };
      if (path.endsWith('/loadError')) return { loadErrorText };
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
      if (path.endsWith('/sql-dialect')) return { obviousWriteVerb: (s: string) => (/^\s*update\b/i.test(s) ? 'UPDATE' : null), tableRefFromNodeId() {} };
      return strictRequire(['/api/types', '/components/exporters', '/mongo-format', '/sql-util', '/bson', '/filter-chips', '/error-normalize',
        '/clipboard', '/poll', '/editor-history', '/confirm.svelte'].map((p) => [p, unused(p)] as const))(path);
    },
  };
  runInNewContext(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const db = context.exports.database;
  /** Resolve the oldest pending GET for `url`. */
  const answer = async (url: string, value: unknown) => {
    const q = pending.get(url) ?? [];
    const d = q.shift();
    assert.ok(d, `no pending GET ${url}`);
    d.resolve(value);
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  const fail = async (url: string, e: unknown) => {
    const d = (pending.get(url) ?? []).shift();
    assert.ok(d, `no pending GET ${url}`);
    d.reject(e);
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  return { db, ws, gets, toasts, answer, fail };
}

const A = '/workspaces/A/db';
const B = '/workspaces/B/db';

test('a late dashboards response from the previous workspace never lands after a switch', async () => {
  const h = setup();
  const a = h.db.loadDashboards();
  h.ws.currentId = 'B';
  const b = h.db.loadDashboards();
  assert.deepEqual(h.gets, [`${A}/dashboards`, `${B}/dashboards`], 'a switch starts its own load');
  await h.answer(`${B}/dashboards`, [{ id: 'b1', name: 'B one' }]);
  await h.answer(`${B}/widgets`, [{ id: 'wb', dashboard_id: 'b1' }]);
  await b;
  // A resolves last: it must not replace B's list, selection or spinner.
  await h.answer(`${A}/dashboards`, [{ id: 'a1', name: 'A one' }]);
  await a;
  assert.deepEqual(h.db.dashboards.map((d: any) => d.id), ['b1']);
  assert.equal(h.db.selectedDashboardId, 'b1');
  assert.deepEqual(h.db.widgets.map((w: any) => w.id), ['wb']);
  assert.equal(h.gets.includes(`${A}/widgets`), false, 'the superseded load stops before fetching widgets');
  assert.equal(h.db.dashboardsLoading, false);
});

test('switching workspace drops the old dashboards and a superseded load keeps the new spinner', async () => {
  const h = setup();
  const a = h.db.loadDashboards();
  await h.answer(`${A}/dashboards`, [{ id: 'a1', name: 'A one' }]);
  await h.answer(`${A}/widgets`, [{ id: 'wa', dashboard_id: 'a1' }]);
  await a;
  assert.equal(h.db.selectedDashboardId, 'a1');

  // Start a refresh in A, then switch to B: B's load clears A's state at once.
  const aAgain = h.db.loadDashboards();
  h.ws.currentId = 'B';
  const b = h.db.loadDashboards();
  assert.equal(h.db.dashboards.length, 0);
  assert.equal(h.db.widgets.length, 0);
  assert.equal(h.db.selectedDashboardId, null);
  assert.equal(h.db.dashboardsLoading, true);
  // A's refresh fails late: no error and no spinner change for B.
  await h.fail(`${A}/dashboards`, new ApiError('A broke'));
  await aAgain;
  assert.equal(h.db.dashboardsError, null);
  assert.equal(h.db.dashboardsLoading, true, 'B is still loading');
  // B fails: its own error shows, and A's list is not left behind.
  await h.fail(`${B}/dashboards`, new TypeError('Load failed'));
  await b;
  assert.equal(h.db.dashboardsError, 'Otto can’t reach the daemon.');
  assert.equal(h.db.dashboards.length, 0);
  assert.equal(h.db.dashboardsLoading, false);
});

test('a late widgets response from the previous workspace is dropped', async () => {
  const h = setup();
  const w = h.db.loadWidgets();
  h.ws.currentId = 'B';
  const wb = h.db.loadWidgets();
  await h.answer(`${B}/widgets`, [{ id: 'wb', dashboard_id: 'b1' }]);
  await h.answer(`${A}/widgets`, [{ id: 'wa', dashboard_id: 'a1' }]);
  await Promise.all([w, wb]);
  assert.deepEqual(h.db.widgets.map((x: any) => x.id), ['wb']);
});

test('updateWidget returns null on a rejected or failed save, so the editor stays open', async () => {
  const h = setup();
  h.db.widgets = [{ id: 'w1', title: 'old', statement: 'SELECT 1' }];
  assert.equal(await h.db.updateWidget('w1', { statement: 'UPDATE t SET a = 1' }), null);
  assert.equal(await h.db.updateWidget('w1', { title: 'new' }), null);
  assert.equal(h.db.widgets[0].title, 'old');
  assert.ok(h.toasts.some((t) => t.startsWith('Couldn’t update the widget')));
});
