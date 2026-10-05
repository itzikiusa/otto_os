import { plural, pluralNoun } from '../src/lib/plural.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred } from './sourceHarness.ts';

// Run the production handlers, including their sibling helpers. State and HTTP
// boundaries are controlled here; these tests do not claim mounted DOM coverage.
function component(path: string, state: Record<string, any>) {
  const source = readFileSync(new URL(path, import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('component.ts', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const lifecycle: (() => void)[] = [];
  const functions = file.statements.filter(node => {
    if (ts.isFunctionDeclaration(node)) return true;
    // Execute real lifecycle registration and its primitive state initializers.
    // Prop/rune state stays controlled by the fixture; no mocked alive flag.
    if (ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)) return node.expression.expression.getText(file) === 'onDestroy';
    return ts.isVariableStatement(node) && node.declarationList.declarations.every(d =>
      d.initializer && [ts.SyntaxKind.TrueKeyword, ts.SyntaxKind.FalseKeyword, ts.SyntaxKind.NumericLiteral].includes(d.initializer.kind),
    );
  }).map(node => node.getText(file)).join('\n');
  const context: Record<string, any> = { plural, pluralNoun, toastError: (message: string, cause: unknown) => { state.toasts?.error(message, cause); }, Error, DOMException, AbortController, onDestroy: (callback: () => void) => lifecycle.push(callback), destroy: () => { for (const callback of lifecycle) callback(); }, ws: { currentId: 'workspace-A' }, $state: { snapshot: (v: unknown) => structuredClone(v) }, ...state };
  runInNewContext(ts.transpileModule(functions, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return context;
}

for (const draft of ['DELETE FROM imported', 'INSERT INTO audit VALUES (1)', 'SELECT * FROM imported; DELETE FROM imported']) {
  test(`successful import never submits the editor draft: ${draft}`, async () => {
    const queries: string[] = [];
    const imports: unknown[] = [];
    const database = {
      selectedConnId: 'A', importDialogOpen: true, tab: { statement: draft, ran_statement: 'SELECT * FROM imported' },
      selectedObjectPath: null,
      runQuery: async () => { queries.push(database.tab.statement); },
    };
    const context = component('../src/modules/database/ImportDialog.svelte', {
      database, importing: false, importAbort: null, progress: null,
      filePath: '/fixture/import.csv', table: 'imported', batchSize: '500', format: 'csv', canImport: true,
      toasts: { success() {}, info() {}, error(message: string) { assert.fail(message); } },
      postNdjsonStream: async (_url: string, body: unknown, onmessage: (v: unknown) => void) => {
        imports.push(body); onmessage({ done: true, rows: 2, batches: 1 });
      },
    });
    await context.runImport();
    assert.equal(imports.length, 1);
    assert.equal(database.importDialogOpen, false);
    assert.deepEqual(queries, [], 'Import authorizes its file only, never the current query draft');
    assert.equal(database.tab.statement, draft);
  });
}

test('request-save retry adopts the successfully created collection before the failed second step', async () => {
  let created = 0, closed = 0;
  const destinations: string[] = [];
  const context = component('../src/modules/api/SaveRequestDialog.svelte', {
    NEW: '__new__', target: '__new__', name: 'List customers', newCollection: 'Payments API', valid: true, busy: false, error: '',
    apiClient: { saveCollection: async () => ({ id: `collection-${++created}` }) },
    onsave: async (_name: string, id: string) => { destinations.push(id); return destinations.length === 2; },
    onclose: () => { closed++; },
  });
  await context.submit();
  assert.equal(closed, 0);
  assert.equal(context.target, 'collection-1', 'the sheet must retain the completed first step');
  await context.submit();
  assert.equal(created, 1);
  assert.deepEqual(destinations, ['collection-1', 'collection-1']);
  assert.equal(closed, 1);
  assert.equal(context.name, 'List customers');
});

function recovery() {
  const requests: string[] = [];
  let fail = true;
  let history = [{ selector: 'HEAD@{0}', sha: 'a'.repeat(40), subject: 'Recovered change' }];
  const context = component('../src/modules/git/RecoveryTools.svelte', {
    id: 'repo', alive: true, mode: 'history', busy: false, error: '', entries: [], more: true, bisect: null,
    git: { refreshStatus: async () => {} },
    api: { get: async (url: string) => {
      requests.push(url);
      if (url.includes('/reflog')) { if (fail) throw Error('History unavailable'); return history; }
      return { active: false };
    } },
  });
  return { context, requests, succeed: () => { fail = false; }, fail: () => { fail = true; }, setHistory: (rows: typeof history) => { history = rows; } };
}

test('Recovery Refresh retries failed active history from offset zero', async () => {
  const { context: c, requests, succeed } = recovery();
  await c.run(() => c.loadHistory(true));
  assert.equal(c.error, 'History unavailable');
  succeed();
  await c.run(c.refresh);
  assert.equal(requests.filter(url => url.endsWith('/reflog?limit=50&skip=0')).length, 2);
  assert.equal(c.entries[0]?.subject, 'Recovered change');
  assert.equal(c.error, '');
});

test('Recovery Refresh replaces stale first page and retains it on a later error', async () => {
  const { context: c, succeed, fail, setHistory } = recovery();
  succeed(); await c.run(() => c.loadHistory(true));
  setHistory([{ selector: 'HEAD@{0}', sha: 'b'.repeat(40), subject: 'New recovery point' }]);
  await c.run(c.refresh);
  assert.equal(c.entries[0]?.subject, 'New recovery point');
  fail(); await c.run(c.refresh);
  assert.equal(c.error, 'History unavailable');
  assert.equal(c.entries[0]?.subject, 'New recovery point');
});

function environmentEditor(saveEnvironment: (...args: any[]) => Promise<any>) {
  const env = { id: 'A', name: 'A', variables: { base_url: 'before' }, secret_keys: [] };
  const c = component('../src/modules/api/EnvironmentsView.svelte', {
    selected: env, selectedId: 'A', loadedFor: null, rows: [], dirty: false, saving: false, canEdit: true,
    apiClient: { saveEnvironment }, ws: { currentId: 'workspace-A' }, confirmer: { ask: async () => true },
  });
  c.seed(env);
  return c;
}

test('environment save preserves newer typing and the next Save submits that draft', async () => {
  const pending = deferred<any>();
  const bodies: any[] = [];
  const c = environmentEditor(async (body: any, id: string) => {
    bodies.push(structuredClone(body));
    return bodies.length === 1 ? pending.promise : { id, ...body };
  });
  c.updateRow(0, { value: 'first' });
  const saving = c.save();
  c.updateRow(0, { value: 'second' });
  pending.resolve({ id: 'A', name: 'A', variables: { base_url: 'first' }, secret_keys: [] });
  await saving;
  assert.equal(c.rows[0].value, 'second');
  assert.equal(c.dirty, true);
  await c.save();
  assert.equal(bodies[0].variables.base_url, 'first');
  assert.equal(bodies[1].variables.base_url, 'second');
  assert.equal(c.dirty, false);
});

test('environment save completion cannot replace a newly selected environment', async () => {
  const pending = deferred<any>();
  const c = environmentEditor(() => pending.promise);
  c.updateRow(0, { value: 'first' });
  const saving = c.save();
  const next = { id: 'B', name: 'B', variables: { base_url: 'workspace-B' }, secret_keys: [] };
  await c.pick(next); c.selected = next;
  pending.resolve({ id: 'A', name: 'A', variables: { base_url: 'first' }, secret_keys: [] });
  await saving;
  assert.equal(c.selectedId, 'B');
  assert.equal(c.loadedFor, 'B');
  assert.equal(c.rows[0].value, 'workspace-B');
});

test('environment failed save retains its draft for retry', async () => {
  const c = environmentEditor(async () => null);
  c.updateRow(0, { value: 'unsaved' }); await c.save();
  assert.equal(c.rows[0].value, 'unsaved'); assert.equal(c.dirty, true); assert.equal(c.saving, false);
});

test('environment Save in a former workspace cannot reseed the current draft', async () => {
  const pending = deferred<any>(); const c = environmentEditor(() => pending.promise);
  c.updateRow(0, { value: 'submitted' }); const saving = c.save();
  c.ws.currentId = 'workspace-B';
  pending.resolve({ id: 'A', name: 'A', variables: { base_url: 'server' }, secret_keys: [] }); await saving;
  assert.equal(c.rows[0].value, 'submitted'); assert.equal(c.dirty, true);
});

test('successful import after a connection change never refreshes another connection metadata', async () => {
  const pending = deferred<void>(); let refreshes = 0, queries = 0;
  const database = { selectedConnId: 'A', selectedObjectPath: 'db:app/table:imported', importDialogOpen: true,
    tab: { statement: 'DELETE FROM imported' }, refreshObject: async () => { refreshes++; }, runQuery: async () => { queries++; } };
  const c = component('../src/modules/database/ImportDialog.svelte', {
    database, importing: false, importAbort: null, progress: null,
    filePath: '/fixture/import.csv', table: 'imported', batchSize: '500', format: 'csv', canImport: true,
    toasts: { success() {}, info() {}, error(message: string) { assert.fail(message); } },
    postNdjsonStream: async (_url: string, _body: unknown, onmessage: (v: unknown) => void) => { await pending.promise; onmessage({ done: true, rows: 1 }); },
  });
  const importing = c.runImport(); database.selectedConnId = 'B'; pending.resolve(); await importing;
  assert.equal(queries, 0); assert.equal(refreshes, 0);
});

for (const renameAgain of [false, true]) {
  test(`environment saved secret rename updates its stored marker while preserving ${renameAgain ? 'another secret rename' : 'newer unrelated edits'}`, async () => {
    const pending = deferred<any>(); const bodies: any[] = [];
    const c = environmentEditor(async (body: any, id: string) => {
      bodies.push(structuredClone(body));
      return bodies.length === 1 ? pending.promise : { id, ...body };
    });
    c.seed({ id: 'A', name: 'A', variables: { base_url: 'before' }, secret_keys: ['old_token'] });
    c.updateRow(1, { key: 'first_token' });
    const saving = c.save();
    if (renameAgain) c.updateRow(1, { key: 'second_token' });
    else c.updateRow(0, { value: 'typed during save' });
    pending.resolve({ id: 'A', name: 'A', variables: { base_url: 'before' }, secret_keys: ['first_token'] });
    await saving;
    assert.equal(c.dirty, true);
    assert.equal(c.rows[1].key, renameAgain ? 'second_token' : 'first_token');
    assert.equal(c.rows[1].storedKey, 'first_token', 'the next Save must refer to the name now persisted in Keychain');
    await c.save();
    assert.deepEqual(bodies[0].secret_renames, { old_token: 'first_token' });
    assert.deepEqual(bodies[1].secret_renames, renameAgain ? { first_token: 'second_token' } : {});
    if (!renameAgain) assert.equal(bodies[1].variables.base_url, 'typed during save');
  });
}

test('request-save sheet does not continue its second mutation after changing workspace', async () => {
  const pending = deferred<any>(); let saves = 0, closed = 0;
  const c = component('../src/modules/api/SaveRequestDialog.svelte', {
    NEW: '__new__', target: '__new__', name: 'List customers', newCollection: 'Payments API', valid: true, busy: false, error: '',
    ws: { currentId: 'A' }, apiClient: { saveCollection: () => pending.promise },
    onsave: async () => { saves++; return true; }, onclose: () => { closed++; },
  });
  const saving = c.submit(); c.ws.currentId = 'B'; pending.resolve({ id: 'collection-A' }); await saving;
  assert.equal(saves, 0, 'the B request must not receive the newly created A collection');
  assert.equal(closed, 0);
});

function guardedImport() {
  const first = deferred<void>(), approval = deferred<string | null>(), prompted = deferred<void>();
  const requests: { url: string; body: any }[] = [], prompts: string[] = [];
  const database = { selectedConnId: 'A', selectedConn: { id: 'A', name: 'Production A', environment: 'prod' }, selectedObjectPath: null,
    importDialogOpen: true, tab: { statement: '' } };
  const c = component('../src/modules/database/ImportDialog.svelte', {
    database, importing: false, importAbort: null, progress: null,
    filePath: '/fixture/import.csv', table: 'imported', batchSize: '500', format: 'csv', canImport: true,
    toasts: { success() {}, info() {}, error(message: string) { assert.fail(message); } },
    confirmer: { promptText: (text: string) => { prompts.push(text); prompted.resolve(); return approval.promise; } },
    postNdjsonStream: async (url: string, body: unknown, onmessage: (v: unknown) => void) => {
      requests.push({ url, body });
      if (requests.length === 1) { await first.promise; onmessage({ error: 'write_blocked: production' }); }
      else onmessage({ done: true, rows: 1 });
    },
  });
  return { c, database, first, approval, prompted, requests, prompts };
}

test('guarded import approval remains bound to its captured connection', async () => {
  const h = guardedImport(); const importing = h.c.runImport();
  h.database.selectedConnId = 'B'; h.database.selectedConn = { id: 'B', name: 'Development B', environment: 'dev' };
  h.first.resolve(); h.approval.resolve('Production A'); await importing;
  for (const prompt of h.prompts) { assert.match(prompt, /Production A/); assert.doesNotMatch(prompt, /Development B/); }
  for (const request of h.requests) assert.equal(request.url, '/connections/A/db/import');
});

test('canceled import while typed confirmation waits cannot retry after late approval', async () => {
  const h = guardedImport(); const importing = h.c.runImport();
  h.first.resolve(); await h.prompted.promise;
  // The footer Cancel calls this controller, even after the first stream has ended.
  h.c.importAbort.abort(); h.approval.resolve('Production A'); await importing;
  assert.equal(h.requests.length, 1);
});

test('old import completion leaves a replacement table dialog open', async () => {
  const pending = deferred<void>();
  const database = { selectedConnId: 'A', importTable: 'first_table', selectedObjectPath: null, importDialogOpen: true, tab: { statement: '' } };
  const c = component('../src/modules/database/ImportDialog.svelte', {
    database, importing: false, importAbort: null, progress: null,
    filePath: '/fixture/import.csv', table: 'first_table', batchSize: '500', format: 'csv', canImport: true,
    toasts: { success() {}, info() {}, error(message: string) { assert.fail(message); } },
    postNdjsonStream: async (_url: string, _body: unknown, onmessage: (v: unknown) => void) => { await pending.promise; onmessage({ done: true, rows: 1 }); },
  });
  const importing = c.runImport();
  // DatabasePage keys ImportDialog by database.importTable; another table
  // therefore has a new component while the old async handler can still settle.
  database.importTable = 'replacement_table'; pending.resolve(); await importing;
  assert.equal(database.importDialogOpen, true);
});

test('guarded import retry uses the submitted format/path/table despite pending form edits', async () => {
  const h = guardedImport(); const importing = h.c.runImport();
  h.first.resolve(); await h.prompted.promise;
  h.c.format = 'json'; h.c.filePath = '/different.json'; h.c.table = 'another_table'; h.c.batchSize = '7';
  h.approval.resolve('Production A'); await importing;
  assert.equal(h.requests.length, 2);
  const retry = h.requests[1].body;
  assert.equal(retry.local_path, '/fixture/import.csv'); assert.equal(retry.format, 'csv');
  assert.equal(retry.table, 'imported'); assert.equal(retry.batch_size, 500); assert.equal(retry.confirm_write, true);
});

test('unmounting Save request while collection creation waits prevents the unsent request mutation', async () => {
  const pending = deferred<any>(); let saves = 0, closed = 0;
  const c = component('../src/modules/api/SaveRequestDialog.svelte', {
    NEW: '__new__', target: '__new__', name: 'List customers', newCollection: 'Payments API', valid: true, busy: false, error: '',
    ws: { currentId: 'A' }, apiClient: { draft: { tabId: 'same-tab' }, saveCollection: () => pending.promise },
    onsave: async () => { saves++; return true; }, onclose: () => { closed++; },
  });
  const saving = c.submit();
  // Invoke the actual onDestroy callbacks registered by the component source,
  // as Svelte does after the caller handles Cancel by setting saveOpen=false.
  c.destroy(); pending.resolve({ id: 'created-before-cancel' }); await saving;
  assert.equal(saves, 0); assert.equal(closed, 0);
});

for (const secret of [false, true]) {
  test(`environment A-B-A selection during save reconciles the clean ${secret ? 'secret name' : 'value'}`, async () => {
    const pending = deferred<any>(); const bodies: any[] = [];
    const c = environmentEditor(async (body: any, id: string) => {
      bodies.push(structuredClone(body));
      return bodies.length === 1 ? pending.promise : { id, ...body };
    });
    const original = { id: 'A', name: 'A', variables: { base_url: 'before' }, secret_keys: secret ? ['old_token'] : [] };
    c.selected = original; c.seed(original);
    c.updateRow(secret ? 1 : 0, secret ? { key: 'first_token' } : { value: 'first' });
    const saving = c.save();
    const other = { id: 'B', name: 'B', variables: {}, secret_keys: [] };
    await c.pick(other); c.selected = other;
    await c.pick(original); c.selected = original;
    assert.equal(c.dirty, false, 'reselecting seeds the old persisted row while its save is pending');
    pending.resolve({ ...original, variables: { base_url: secret ? 'before' : 'first' }, secret_keys: secret ? ['first_token'] : [] });
    await saving;
    assert.equal(c.dirty, false);
    if (secret) {
      assert.equal(c.rows[1].key, 'first_token');
      assert.equal(c.rows[1].storedKey, 'first_token');
      c.updateRow(0, { value: 'unrelated later edit' }); await c.save();
      assert.deepEqual(bodies[1].secret_keys, ['first_token']);
      assert.deepEqual(bodies[1].secret_renames, {});
    } else assert.equal(c.rows[0].value, 'first');
  });
}

test('Cancel synchronously invalidates Save request before Svelte destruction runs', async () => {
  const pending = deferred<any>(); let saves = 0, closed = 0;
  const c = component('../src/modules/api/SaveRequestDialog.svelte', {
    NEW: '__new__', target: '__new__', name: 'List customers', newCollection: 'Payments API', valid: true, busy: false, error: '',
    ws: { currentId: 'A' }, apiClient: { draft: { tabId: 'same-tab' }, saveCollection: () => pending.promise },
    onsave: async () => { saves++; return true; }, onclose: () => { closed++; },
  });
  const saving = c.submit(); c.close(); pending.resolve({ id: 'created-before-cancel' }); await saving;
  assert.equal(saves, 0); assert.equal(closed, 1);
});

test('environment A-B-A return preserves actual new typing and reconciles saved secret marker', async () => {
  const pending = deferred<any>(); const c = environmentEditor(() => pending.promise);
  const original = { id: 'A', name: 'A', variables: { base_url: 'before' }, secret_keys: ['old_token'] };
  c.selected = original; c.seed(original); c.updateRow(1, { key: 'first_token' });
  const saving = c.save();
  const other = { id: 'B', name: 'B', variables: {}, secret_keys: [] };
  await c.pick(other); c.selected = other;
  await c.pick(original); c.selected = original;
  c.updateRow(0, { value: 'new typing after returning' }); c.updateRow(1, { key: 'second_token' });
  pending.resolve({ ...original, secret_keys: ['first_token'] }); await saving;
  assert.equal(c.dirty, true); assert.equal(c.rows[0].value, 'new typing after returning');
  assert.equal(c.rows[1].key, 'second_token'); assert.equal(c.rows[1].storedKey, 'first_token');
});
