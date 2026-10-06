import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';

function setup(fail: (title: string, attempt: number) => boolean) {
  const text = readFileSync(new URL('../src/modules/agents/NewSession.svelte', import.meta.url), 'utf8');
  const ast = ts.createSourceFile('new-session.ts', text.slice(text.indexOf('>') + 1, text.indexOf('</script>')), ts.ScriptTarget.Latest, true);
  const functions = ast.statements.filter(ts.isFunctionDeclaration).map(s => s.getText(ast)).join('\n');
  const calls: any[] = [], successes: string[] = [], opened: string[] = [], errors: unknown[] = [];
  let closes = 0;
  const launch = async (request: any, options: any) => {
    const copy = JSON.parse(JSON.stringify({request, options})); calls.push(copy);
    const attempt = calls.filter(c => c.request.title === request.title).length;
    if (fail(request.title, attempt)) throw new Error('temporary provider failure');
    successes.push(request.title); return {id: request.title};
  };
  const context: Record<string, any> = {busy: false, total: 2, chosen: ['claude'], counts: {claude: 2}, provider: 'claude',
    providerReadiness: () => ({available: true}), extraDirs: ['/shared'], dirDraft: '/pending', title: 'Review', prompt: 'Check ownership', cwd: '~/repo',
    accountIds: {claude: 'account-a'}, networkProfileId: 'net-a', browser: true, supportsModel: true, model: 'model-a', scratchMode: true,
    pendingSpawns: [], batchFailures: [], batchSuccesses: [],
    // The typed-folder pre-check (resume-missing-folder fix): the folder exists.
    checkingCwd: false, cwdError: '', checkFolder: async () => ({ok: true}), api: {get: async () => ({})}, browsePath: (p: string) => p,
    ws: {scratch: {root_path: '/home/owner'}, openSessionWithPrompt: launch, createSessionQuiet: launch,
      setViewMode: () => {}, openSession: (id: string) => opened.push(id), navigateToSession: (id: string) => opened.push(id)},
    toasts: {error: (...args: unknown[]) => errors.push(args)}, toastError: (...args: unknown[]) => errors.push(args),
    localStorage: {setItem: () => {}}, PREFS_KEY: 'fixture', onclose: () => {closes++;},
  };
  runInNewContext(ts.transpileModule(functions, {compilerOptions: {target: ts.ScriptTarget.ES2022}}).outputText, context);
  return {context, calls, successes, opened, errors, closes: () => closes};
}
test('mixed batch keeps recovery open and retries only the failed submitted configuration', async () => {
  const h = setup((title, attempt) => title === 'Review 2' && attempt === 1);
  await h.context.create();
  assert.equal(h.closes(), 0, 'partial success must keep its recovery surface');
  const failedRequest = h.calls[1];
  Object.assign(h.context, {title: 'Changed', prompt: 'Different', cwd: '/different', model: 'different', accountIds: {}, networkProfileId: '', browser: false, extraDirs: []});
  await (h.context.retryFailed ? h.context.retryFailed() : h.context.create());
  assert.equal(h.calls.length, 3, 'successful members are not spawned twice');
  assert.deepEqual(h.calls[2], failedRequest, 'retry uses the submitted request, not changed form fields');
  assert.deepEqual(h.successes, ['Review 1', 'Review 2']);
  assert.equal(h.closes(), 1);
});
test('all-success batch opens each session once and closes', async () => {
  const h = setup(() => false); await h.context.create();
  assert.deepEqual(h.successes, ['Review 1', 'Review 2']); assert.equal(h.closes(), 1);
});
test('all-failed batch stays open and can complete on retry', async () => {
  const h = setup((_title, attempt) => attempt === 1); await h.context.create();
  assert.equal(h.closes(), 0); assert.equal(h.successes.length, 0);
  await (h.context.retryFailed ? h.context.retryFailed() : h.context.create());
  assert.deepEqual(h.successes, ['Review 1', 'Review 2']); assert.equal(h.closes(), 1);
});
test('workspace-bound failed member waits for its original workspace before retry', async () => {
  const h = setup((title, attempt) => title === 'Review 2' && attempt === 1);
  h.context.scratchMode = false;
  h.context.ws.currentId = 'workspace-a';
  await h.context.create();
  const failed = h.calls[1];
  h.context.ws.currentId = 'workspace-b';
  await h.context.retryFailed();
  assert.equal(h.calls.length, 2, 'switching workspace cannot redirect a retained submission');
  assert.equal(h.closes(), 0);
  assert.match(h.context.batchFailures[0], /original workspace/);
  h.context.ws.currentId = 'workspace-a';
  await h.context.retryFailed();
  assert.equal(h.calls.length, 3);
  assert.deepEqual(h.calls[2], failed);
  assert.equal(h.calls[2].options.scratch, false);
  assert.deepEqual(h.successes, ['Review 1', 'Review 2']);
  assert.equal(h.closes(), 1);
});
