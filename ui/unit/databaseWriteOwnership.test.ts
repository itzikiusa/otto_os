import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { randomUUID } from 'node:crypto';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';
import * as resultBudget from '../src/lib/stores/db-result-budget.ts';

class ApiError extends Error {}
const result = { columns: [], rows: [], affected_rows: 1, elapsed_ms: 1 };
function setup() {
  const refused = deferred<any>();
  const approval = deferred<string | null>();
  const prompted = deferred<void>();
  const queries: { url: string; body: any }[] = [];
  const prompts: { text: string; placeholder: string }[] = [];
  const api = {
    get: async () => [],
    post: async (url: string, body: any) => {
      if (!url.endsWith('/query')) return {};
      queries.push({ url, body });
      return queries.length === 1 ? refused.promise : result;
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
  const context = {
    exports: {} as Record<string, any>, Error, DOMException, AbortController, crypto: { randomUUID },
    setTimeout, clearTimeout, console,
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    $state: Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v }),
    $derived: Object.assign((v: unknown) => v, { by: (fn: () => unknown) => fn() }),
    require: (path: string) => {
      if (path.endsWith('/client')) return { api, ApiError, isAbortError: (e: Error) => e.name === 'AbortError' };
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
      return {};
    },
  };
  runInNewContext(ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const v = context.exports.database;
  v.connections = [{ id: 'A', name: 'Production A', environment: 'prod', kind: 'postgres' }, { id: 'B', name: 'Development B', environment: 'dev', kind: 'postgres' }];
  v.selectedConnId = 'A'; v.openConnIds = ['A', 'B']; v.activeDb = 'db:app';
  v.capabilities = { engine: 'postgres' };
  return { v, refused, approval, prompted, queries, prompts };
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
