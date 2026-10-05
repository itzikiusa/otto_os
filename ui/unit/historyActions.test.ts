import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';

function setup(status = 'working') {
  const text = readFileSync(new URL('../src/modules/agents/history/HistoryPage.svelte', import.meta.url), 'utf8');
  const script = text.slice(text.indexOf('>') + 1, text.indexOf('</script>'));
  const ast = ts.createSourceFile('history.ts', script, ts.ScriptTarget.Latest, true);
  const functions = ast.statements.filter(ts.isFunctionDeclaration).map(s => s.getText(ast)).join('\n');
  const restarts: string[] = [], resumes: string[] = [], opened: string[] = [], errors: unknown[] = [];
  const views = new Map([['a', 'terminal']]);
  const current = {id: 'a', status};
  const context: Record<string, any> = {wsId: 'w', busy: false, canEdit: true,
    ws: {getSession: () => current, refreshSessions: async () => {},
      restartSession: async (id: string) => {restarts.push(id);},
      resumeSession: async (id: string) => {resumes.push(id); return current;},
      setViewMode: () => {}, navigateToSession: (id: string) => opened.push(id)},
    history: {importEntry: async () => ({id: 'a'}), patchSession: () => {}},
    transcript: {setView: (id: string, view: string) => views.set(id, view), view: (id: string) => views.get(id) ?? 'chat'},
    localStorage: {setItem: () => {}}, winKey: (s: string) => s,
    toasts: {error: (...args: unknown[]) => errors.push(args)},
  };
  runInNewContext(ts.transpileModule(functions, {compilerOptions: {target: ts.ScriptTarget.ES2022}}).outputText, context);
  return {context, restarts, resumes, opened, views, errors};
}
for (const status of ['working', 'running', 'idle']) test(`History opens ${status} without restarting its PTY`, async () => {
  const h = setup(status), row = {session_id: 'a', status, resumable: true};
  await h.context.resume(row);
  assert.deepEqual(h.restarts, []);
  assert.deepEqual(h.opened, ['a']);
  assert.deepEqual(h.errors, []);
  assert.equal(h.context.resumeLabel(row), 'Open in Otto');
});
test('History never unconditionally restarts a stale inactive row whose PTY is now working', async () => {
  const h = setup('working');
  await h.context.resume({session_id: 'a', status: 'exited', resumable: true});
  assert.deepEqual(h.restarts, []);
  assert.deepEqual(h.opened, ['a']);
  assert.deepEqual(h.errors, []);
});
test('History Open in Chat updates a warmed Terminal view preference', () => {
  const h = setup('idle');
  h.context.openInChat('a');
  assert.equal(h.views.get('a'), 'chat');
  assert.deepEqual(h.opened, ['a']);
});

test('History opens a stale live row through safe resume if its process has since exited', async () => {
  const h = setup('exited');
  await h.context.resume({session_id: 'a', status: 'running', resumable: true});
  assert.deepEqual(h.resumes, ['a'], 'the server must atomically ensure liveness for every explicit open');
  assert.deepEqual(h.restarts, []);
  assert.deepEqual(h.opened, ['a']);
});
