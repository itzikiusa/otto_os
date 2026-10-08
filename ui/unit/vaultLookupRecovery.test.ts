import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

function switcher(lookup: (query: string) => Promise<unknown>) {
  const created: string[] = [];
  const opened: string[] = [];
  const vault = { current: { id: 1 }, wsId: 'A', switcherOpen: true, switcherQuery: lookup,
    createNote: (path: string) => { created.push(path); }, open(path: string) { opened.push(path); } };
  const state = componentFunctions(new URL('../src/modules/vault/Switcher.svelte', import.meta.url),
    ['refresh', 'close', 'pick', 'createFromQuery', 'onKey'], { vault, query: 'Existing note', hits: [], sel: 0, seq: 0,
      loading: false, lookupError: '', resolvedQuery: null });
  return { state, vault, created, opened };
}
const enter = { key: 'Enter', shiftKey: false, preventDefault() {} };

test('ordinary Enter cannot create while the current switcher lookup is pending', async () => {
  const request = deferred<unknown>();
  const { state, created } = switcher(() => request.promise);
  const pending = state.refresh(state.query);
  state.onKey(enter);
  request.resolve([{ path: 'Existing note.md', title: 'Existing note' }]); await pending;
  assert.equal(created.length, 0);
});

test('ordinary Enter cannot create after the current switcher lookup failed', async () => {
  const { state, created } = switcher(async () => { throw new Error('Offline'); });
  // An old component lets the exception escape; the essential assertion is
  // still that failed availability must never authorize implicit creation.
  await state.refresh(state.query).catch(() => {});
  state.onKey(enter);
  assert.equal(created.length, 0);
});

test('a switcher result from the previous vault cannot populate the current vault', async () => {
  const request = deferred<unknown>();
  const { state, vault } = switcher(() => request.promise);
  const pending = state.refresh(state.query);
  vault.current = { id: 2 };
  request.resolve([{ path: 'private-A.md', title: 'Only in A' }]); await pending;
  assert.equal(state.hits.length, 0);
});

test('successful empty current lookup allows deliberate Enter creation', async () => {
  const { state, created } = switcher(async () => []);
  await state.refresh(state.query); state.onKey(enter);
  assert.deepEqual(created, ['Existing note.md']);
});

test('Down during a pending lookup still lets Enter open the first arriving result', async () => {
  const request = deferred<unknown>();
  const { state, opened, created } = switcher(() => request.promise);
  const pending = state.refresh(state.query);
  state.onKey({ ...enter, key: 'ArrowDown' });
  request.resolve([{ path: 'first.md', title: 'First' }]); await pending;
  state.onKey(enter);
  assert.deepEqual(opened, ['first.md']);
  assert.equal(created.length, 0);
});

test('successful multiple switcher results retain bounded keyboard movement', async () => {
  const { state, opened } = switcher(async () => [{ path: 'first.md' }, { path: 'second.md' }]);
  await state.refresh(state.query);
  state.onKey({ ...enter, key: 'ArrowDown' });
  state.onKey({ ...enter, key: 'ArrowDown' });
  assert.equal(state.sel, 1);
  state.onKey({ ...enter, key: 'ArrowUp' });
  state.onKey(enter);
  assert.deepEqual(opened, ['first.md']);
});

test('Down during pending empty lookup does not prevent deliberate creation after success', async () => {
  const request = deferred<unknown>();
  const { state, opened, created } = switcher(() => request.promise);
  const pending = state.refresh(state.query);
  state.onKey({ ...enter, key: 'ArrowDown' }); state.onKey(enter);
  assert.equal(created.length, 0);
  request.resolve([]); await pending;
  state.onKey(enter);
  assert.equal(opened.length, 0);
  assert.deepEqual(created, ['Existing note.md']);
});

test('refine does not dispatch after the before-snapshot outlives its workspace', async () => {
  const snapshot = deferred<unknown>();
  let sends = 0, polls = 0;
  const vault = {wsId: 'A', current: {id: 1}, notePath: 'a.md', dirty: false, editing: false, open() {}};
  const state = componentFunctions(new URL('../src/modules/vault/RefineDrawer.svelte', import.meta.url), ['send'], {
    vault, path: 'a.md', prompt: 'Refine this note', provider: 'claude', sending: false, queued: false,
    result: null, epoch: 0, sessionId: null, vaultNote: () => snapshot.promise,
    refineNote: async () => {sends++; return {session_id: 'session-A', reply: 'done'};},
    startSessionPoll() { polls++; }, stopPolling() {}, toasts: {success() {}}, toastError() {},
  });
  const running = state.send();
  vault.wsId = 'B';
  snapshot.resolve({raw: 'original'});
  await running;
  assert.equal(sends, 0, 'a departed workspace must not launch an agent');
  assert.equal(polls, 0);
});

test('refine result cannot adopt a previous workspace session', async () => {
  const response = deferred<unknown>();
  const started = deferred<void>();
  const vault = {wsId: 'A', current: {id: 1}, notePath: 'a.md', dirty: false, editing: false, open() {}};
  const state = componentFunctions(new URL('../src/modules/vault/RefineDrawer.svelte', import.meta.url), ['send'], {
    vault, path: 'a.md', prompt: 'Refine this note', provider: 'claude', sending: false, queued: false,
    result: null, epoch: 0, sessionId: null, vaultNote: async () => ({raw: 'original'}),
    refineNote: () => {started.resolve(); return response.promise;},
    startSessionPoll() {}, stopPolling() {}, toasts: {success() {}}, toastError() {},
  });
  const running = state.send();
  await started.promise;
  vault.wsId = 'B';
  response.resolve({session_id: 'session-A', reply: 'done'});
  await running;
  assert.equal(state.sessionId, null);
});

test('docs start response cannot select the old run in a newly selected vault', async () => {
  const pending = deferred<unknown>();
  const vault = {wsId: 'A', current: {id: 1}, lookupGeneration: 1, docsRun: null, refreshDocsRuns() {}};
  const state = componentFunctions(new URL('../src/modules/vault/DocsAgentsView.svelte', import.meta.url), ['start'], {
    vault, prompt: 'Document', targetDir: '', agents: [{provider: 'claude', model: ''}], starting: false,
    tplSkills: [], reviewEnabled: false, openTerminals: new Set(), runDocsAgents: () => pending.promise,
    startPoll() {}, toastError() {},
  });
  const running = state.start();
  vault.current = {id: 2}; vault.lookupGeneration++;
  pending.resolve({id: 'old-run', vault_id: 1});
  await running;
  assert.equal(vault.docsRun, null);
});

test('resolving one docs run cannot replace a newly selected run', async () => {
  const pending = deferred<unknown>();
  const vault = {docsRun: {id: 'A'}, refreshDocsRuns() {}};
  const state = componentFunctions(new URL('../src/modules/vault/DocsAgentsView.svelte', import.meta.url), ['resolveRun'], {
    vault, resolving: false, resolveDocsRun: () => pending.promise, toasts: {success() {}}, toastError() {},
  });
  const running = state.resolveRun('fixed');
  vault.docsRun = {id: 'B'};
  pending.resolve({id: 'A'});
  await running;
  assert.equal(vault.docsRun.id, 'B');
});
