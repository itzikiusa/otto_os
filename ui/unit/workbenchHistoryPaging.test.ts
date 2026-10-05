import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource, deferred } from './sourceHarness.ts';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

test('Workbench revision wrapper transmits bounded limit and exclusive cursor without changing direct revision URLs', async () => {
  const urls: string[] = [];
  const api = loadSource(new URL('../src/lib/api/workbench.ts', import.meta.url), {
    './client': { api: { get: async (url: string) => { urls.push(url); return []; } } },
  });
  await api.listWorkbenchRevisions('workspace with spaces', 'doc/id', { limit: 100, before_seq: 500 });
  const page = new URL(urls[0], 'http://fixture');
  assert.equal(page.pathname, '/workspaces/workspace%20with%20spaces/workbench/docs/doc%2Fid/revisions');
  assert.equal(page.searchParams.get('limit'), '100');
  assert.equal(page.searchParams.get('before_seq'), '500');
  await api.getWorkbenchRevision('workspace with spaces', 'doc/id', 1);
  assert.equal(urls[1], '/workspaces/workspace%20with%20spaces/workbench/docs/doc%2Fid/revisions/1');
});

// Execute the production handlers and their actual primitive initializers.
// Async transport is controlled; this does not claim mounted Svelte coverage.
function history(overrides: Record<string, any> = {}) {
  const source = readFileSync(new URL('../src/modules/workbench/HistoryPanel.svelte', import.meta.url), 'utf8')
    .split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('history.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const body = file.statements.filter(node => {
    if (ts.isFunctionDeclaration(node)) return true;
    if (ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)) return node.expression.expression.getText(file) === 'onDestroy';
    return ts.isVariableStatement(node) && node.declarationList.declarations.every(d =>
      d.initializer && ([ts.SyntaxKind.TrueKeyword, ts.SyntaxKind.FalseKeyword, ts.SyntaxKind.NumericLiteral, ts.SyntaxKind.StringLiteral].includes(d.initializer.kind)
        || (ts.isCallExpression(d.initializer) && d.initializer.expression.getText(file) === '$state')));
  }).map(node => node.getText(file).replace(/^(let|const) /, 'var ')).join('\n');
  const destroy: (() => void)[] = [];
  const c: Record<string, any> = {
    ws: 'workspace-A', docId: 'doc-A', rev: 305, current: 'current',
    revs: [], loading: false, error: null, selected: null, compareTo: 'current', detail: null, other: null, restoring: false,
    listWorkbenchRevisions: async () => [], restoreWorkbenchRevision: async () => ({ id: 'doc-A' }),
    onrestored() {}, confirmer: { ask: async () => true }, toasts: { success() {}, error() {} },
    loadErrorText: (error: Error) => error.message,
    $state: (value: unknown) => value,
    onDestroy: (fn: () => void) => destroy.push(fn), destroy: () => destroy.forEach(fn => fn()),
    ...overrides,
  };
  runInNewContext(ts.transpileModule(body, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, c);
  // Simulate the actual initial document effect before the user's selection.
  c.syncDocument?.();
  Object.assign(c, overrides);
  return c;
}

const revisions = (newest: number, count = 100) => Array.from({ length: count }, (_, i) => ({ seq: newest - i }));

test('History requests one bounded exclusive page and replaces the rendered window', async () => {
  const calls: any[][] = [];
  const c = history({ listWorkbenchRevisions: async (...args: any[]) => { calls.push(args); return revisions(calls.length === 1 ? 305 : 205); } });
  await c.load();
  await c.load(206);
  assert.equal(calls[0][2]?.limit, 100, 'initial timeline must explicitly request a bounded page');
  assert.equal(calls[1][2]?.before_seq, 206, 'older page starts strictly below its prior oldest row');
  assert.equal(c.revs.length, 100, 'old page rows cannot accumulate in DOM state');
  assert.equal(c.revs[0].seq, 205);
  assert.equal(c.revs.at(-1).seq, 106);
});

for (const failure of [false, true]) {
  test(`History superseded ${failure ? 'failure' : 'success'} cannot publish rows, error, or loading`, async () => {
    const old = deferred<any[]>(), current = deferred<any[]>();
    let calls = 0;
    const c = history({ listWorkbenchRevisions: () => ++calls === 1 ? old.promise : current.promise });
    const loadingOld = c.load();
    c.docId = 'doc-B';
    const loadingCurrent = c.load();
    if (failure) old.reject(Error('old document offline')); else old.resolve(revisions(305));
    await loadingOld;
    assert.equal(c.revs.length, 0);
    assert.equal(c.error, null);
    assert.equal(c.loading, true, 'old finalizer cannot hide active document loading');
    current.resolve(revisions(7, 7));
    await loadingCurrent;
    assert.equal(c.revs.length, 7);
    assert.equal(c.loading, false);
  });
}

test('History A-B-A navigation rejects the first A response', async () => {
  const old = deferred<any[]>();
  let calls = 0;
  const c = history({ listWorkbenchRevisions: () => ++calls === 1 ? old.promise : Promise.resolve(revisions(calls === 2 ? 20 : 400, 2)) });
  const loadingOld = c.load();
  c.docId = 'doc-B'; await c.load();
  c.docId = 'doc-A'; await c.load();
  old.resolve(revisions(305)); await loadingOld;
  assert.equal(c.revs[0].seq, 400);
  assert.equal(c.revs.length, 2);
});

test('History restore approval stays bound to the selected revision', async () => {
  const approval = deferred<boolean>();
  const mutations: any[][] = [];
  const c = history({ selected: 1, confirmer: { ask: () => approval.promise },
    restoreWorkbenchRevision: async (...args: any[]) => { mutations.push(args); return { id: 'doc-A' }; } });
  const restoring = c.restore();
  c.selected = 5;
  approval.resolve(true); await restoring;
  assert.equal(mutations.length, 1);
  assert.deepEqual(mutations[0], ['workspace-A', 'doc-A', 1], 'approval for revision 1 cannot restore the later selection');
});

test('History changed document cancels pending restore approval before mutation', async () => {
  const approval = deferred<boolean>();
  const mutations: any[][] = [];
  const c = history({ selected: 1, confirmer: { ask: () => approval.promise },
    restoreWorkbenchRevision: async (...args: any[]) => { mutations.push(args); return {}; } });
  const restoring = c.restore();
  c.docId = 'doc-B'; c.selected = 9;
  approval.resolve(true); await restoring;
  assert.equal(mutations.length, 0);
});

test('History old restore completion cannot replace a newly selected document', async () => {
  const pending = deferred<any>();
  const applied: any[] = [];
  const c = history({ selected: 1, restoreWorkbenchRevision: () => pending.promise, onrestored: (doc: any) => applied.push(doc) });
  const restoring = c.restore();
  await Promise.resolve();
  c.docId = 'doc-B'; c.selected = 9;
  pending.resolve({ id: 'doc-A' }); await restoring;
  assert.equal(applied.length, 0);
  assert.equal(c.selected, 9);
});


test('History actual destroy callback cancels pending restore approval', async () => {
  const approval = deferred<boolean>();
  let mutations = 0;
  const c = history({ selected: 1, confirmer: { ask: () => approval.promise },
    restoreWorkbenchRevision: async () => { mutations++; return {}; } });
  const restoring = c.restore();
  c.destroy();
  approval.resolve(true); await restoring;
  assert.equal(mutations, 0);
});

test('History A-B-A visit cancels earlier restore approval', async () => {
  const approval = deferred<boolean>();
  let mutations = 0;
  const c = history({ selected: 1, confirmer: { ask: () => approval.promise },
    restoreWorkbenchRevision: async () => { mutations++; return {}; } });
  const restoring = c.restore();
  c.docId = 'doc-B'; await c.load();
  c.docId = 'doc-A'; await c.load();
  c.selected = 8;
  approval.resolve(true); await restoring;
  assert.equal(mutations, 0);
  assert.equal(c.selected, 8);
});

test('History navigation goes back without retaining prior page rows', async () => {
  const calls: any[] = [];
  const c = history({ listWorkbenchRevisions: async (_ws: string, _doc: string, opts: any) => {
    calls.push(opts); return revisions((opts.before_seq ?? 306) - 1);
  } });
  await c.load();
  await c.older();
  assert.equal(c.revs[0].seq, 205);
  await c.newer();
  assert.equal(c.revs[0].seq, 305);
  assert.equal(c.revs.length, 100);
  assert.equal(calls.at(-1).before_seq, undefined);
});
