import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

// ui-correctness-6-01: the DB Assistant's first turn mints its assist id on
// the server. Closing (or Stop → Close, or reopening) before that id reached
// the panel used to leave a hidden agent session running with no DELETE, and a
// reopened panel adopted the old turn's answer. These drive the real sub-store
// against a scripted transport.

function setup() {
  const posts: Array<ReturnType<typeof deferred<unknown>>> = [];
  const closed: string[] = [];
  const { DbAssistStore } = loadSource(new URL('../src/lib/stores/database/assist.svelte.ts', import.meta.url), {
    '../../toast.svelte': { toasts: { error: () => {}, success: () => {} } },
    '../../toastError': { toastError: () => {} },
    '../../api/client': {
      dbAssistStart: (_conn: string, _body: unknown, signal: AbortSignal) => {
        const d = deferred<unknown>();
        signal.addEventListener('abort', () => d.reject(new Error('aborted')));
        posts.push(d);
        return d.promise;
      },
      dbAssistSummary: async () => ({}),
      dbAssistClose: async (conn: string, aid: string) => { closed.push(`${conn}/${aid}`); },
    },
    '../auth.svelte': { auth: { can: () => true } },
    '../../confirm.svelte': { confirmer: { ask: async () => true } },
    '../../components/exporters': { downloadText: () => {} },
  });
  const a = new DbAssistStore({
    selectedConnId: () => 'c1',
    activeDb: () => null,
    currentStatement: () => '',
    insertIntoQuery: () => {},
    runReadOnly: async () => {},
  });
  return { a, posts, closed };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test('close mid-POST: the late response is discarded with its minted id and the panel stays closed', async () => {
  const h = setup();
  h.a.openAssist('nl');
  const turn = h.a.startAssist('how many orders?');
  assert.equal(h.a.assistBusy, true);
  await h.a.closeAssist();
  assert.deepEqual(h.closed, [], 'no id yet: nothing to DELETE at close time');
  h.posts[0].resolve({ assist_id: 'a-old', session_id: 's-old', sql: 'SELECT 1', note: '' });
  await turn;
  await tick();
  assert.deepEqual(h.closed, ['c1/a-old'], 'the orphaned session is discarded');
  assert.equal(h.a.assistOpen, false);
  assert.equal(h.a.assistId, null);
  assert.equal(h.a.assistSessionId, null);
  assert.equal(h.a.assistProposedSql, '');
});

test('reopen mid-POST: the old turn never binds the new panel or releases its busy flag', async () => {
  const h = setup();
  h.a.openAssist('nl');
  const old = h.a.startAssist('first');
  h.a.openAssist('ask');
  const fresh = h.a.startAssist('second');
  assert.equal(h.posts.length, 2);
  h.posts[0].resolve({ assist_id: 'a-old', session_id: 's-old', sql: 'SELECT old', note: '' });
  await old;
  await tick();
  assert.equal(h.a.assistBusy, true, 'the new turn is still in flight');
  assert.equal(h.a.assistId, null, 'the old assist id must not become the new panel’s');
  assert.equal(h.a.assistProposedSql, '');
  assert.deepEqual(h.closed, ['c1/a-old']);
  h.posts[1].resolve({ assist_id: 'a-new', session_id: 's-new', sql: 'SELECT new', note: '' });
  await fresh;
  assert.equal(h.a.assistId, 'a-new');
  assert.equal(h.a.assistProposedSql, 'SELECT new');
  assert.equal(h.a.assistBusy, false);
});

test('the orphan’s session event arrives first: discarded on sight, once', async () => {
  const h = setup();
  h.a.openAssist('nl');
  const turn = h.a.startAssist('q');
  await h.a.closeAssist();
  h.a.setAssistSession('a-old', 'c1', 's-old');
  await tick();
  assert.deepEqual(h.closed, ['c1/a-old']);
  assert.equal(h.a.assistSessionId, null);
  h.posts[0].resolve({ assist_id: 'a-old', session_id: 's-old' });
  await turn;
  await tick();
  assert.deepEqual(h.closed, ['c1/a-old'], 'the late response does not DELETE it again');
});

test('Stop then Close: the aborted turn’s session is still found by its event and discarded', async () => {
  const h = setup();
  h.a.openAssist('nl');
  const turn = h.a.startAssist('q');
  h.a.stopAssist();
  await turn;
  assert.equal(h.a.assistBusy, false);
  await h.a.closeAssist();
  h.a.setAssistSession('a-stopped', 'c1', 's-stopped');
  await tick();
  assert.deepEqual(h.closed, ['c1/a-stopped']);
});

test('Stop keeps the panel able to adopt the turn’s session (and Close then DELETEs it)', async () => {
  const h = setup();
  h.a.openAssist('nl');
  const turn = h.a.startAssist('q');
  h.a.stopAssist();
  await turn;
  h.a.setAssistSession('a-1', 'c1', 's-1');
  assert.equal(h.a.assistId, 'a-1');
  assert.equal(h.a.assistSessionId, 's-1');
  await h.a.closeAssist();
  assert.deepEqual(h.closed, ['c1/a-1']);
});
