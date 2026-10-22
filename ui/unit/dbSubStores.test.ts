import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

// (Arrays a loaded store creates live in the harness's VM realm, so they are
// compared by content/length, not with deepEqual against this realm's [].)
// The DatabaseStore façade delegates history, saved queries and the DB
// Assistant to sub-stores in lib/stores/database/. These run each one against
// a fake host (the façade's side of the contract) and a scripted transport.

const toastsSeen: string[] = [];
const toasts = {
  error: (t: string) => void toastsSeen.push(`error:${t}`),
  success: (t: string) => void toastsSeen.push(`success:${t}`),
};
const common = {
  '../../toast.svelte': { toasts },
  '../../toastError': { toastError: (t: string) => void toastsSeen.push(`error:${t}`) },
  '../../loadError': { loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)) },
};

test('history: a late response for a connection the user left never lands', async () => {
  const pending = deferred<unknown>();
  const urls: string[] = [];
  const { DbHistoryStore } = loadSource(new URL('../src/lib/stores/database/history.svelte.ts', import.meta.url), {
    ...common,
    '../../api/client': { api: { get: (url: string) => { urls.push(url); return pending.promise; } } },
  });
  let selected: string | null = 'a';
  const opened: string[] = [];
  const h = new DbHistoryStore({ selectedConnId: () => selected, accessEpoch: () => 0, newTab: (s: string) => opened.push(s) });
  const load = h.loadHistory();
  assert.equal(urls[0], '/connections/a/db/history?limit=100');
  selected = 'b';
  pending.resolve([{ id: '1', statement: 'SELECT a' }]);
  await load;
  assert.equal(h.history.length, 0, 'connection a’s rows must not show under b');
  // A load for a connection that is not the visible one is skipped outright.
  await h.loadHistory('a');
  assert.equal(urls.length, 1);
});

test('history: "Load more" grows the window by 100 up to the API cap', async () => {
  const urls: string[] = [];
  const { DbHistoryStore } = loadSource(new URL('../src/lib/stores/database/history.svelte.ts', import.meta.url), {
    ...common,
    '../../api/client': { api: { get: async (url: string) => { urls.push(url); return []; } } },
  });
  const h = new DbHistoryStore({ selectedConnId: () => 'a', accessEpoch: () => 0, newTab: () => {} });
  h.history = Array.from({ length: 100 }, (_, i) => ({ id: String(i), statement: '' }));
  assert.equal(h.canLoadMoreHistory, true);
  await h.loadMoreHistory();
  assert.equal(urls.at(-1), '/connections/a/db/history?limit=200');
  h.historyLimit = 1000;
  await h.loadMoreHistory();
  assert.equal(urls.length, 1, 'no fetch past the 1000-row cap');
});

test('saved queries: a save made before a workspace switch does not land in the new list', async () => {
  const post = deferred<unknown>();
  const { DbSavedQueriesStore } = loadSource(new URL('../src/lib/stores/database/savedQueries.svelte.ts', import.meta.url), {
    ...common,
    '../../api/client': { api: { post: () => post.promise } },
  });
  let base = '/workspaces/w1/db';
  const tab = { id: 7, name: 'Query', statement: 'SELECT 1' };
  const persisted: string[] = [];
  const s = new DbSavedQueriesStore({
    wsBase: () => base,
    accessEpoch: () => 0,
    selectedConnId: () => 'c1',
    tabs: () => [tab],
    activeTab: () => tab,
    locateTab: () => ({ connId: 'c1', tab }),
    persistTabsNow: (c: string) => persisted.push(c),
    persistTabs: () => {},
    focusQueryTab: () => {},
    newTab: () => {},
  });
  const save = s.saveQuery('Mine', 'SELECT 1');
  base = '/workspaces/w2/db';
  post.resolve({ id: 'q1', name: 'Mine', statement: 'SELECT 1' });
  await save;
  assert.equal(s.savedQueries.length, 0);
  assert.deepEqual(persisted, [], 'the tab is not re-bound to a query of another workspace');
});

test('assist: inserting the agent’s SQL asks before replacing the editor text, and keeps it on No', async () => {
  const answers = [false, true];
  const asked: string[] = [];
  const { DbAssistStore } = loadSource(new URL('../src/lib/stores/database/assist.svelte.ts', import.meta.url), {
    ...common,
    '../../api/client': { dbAssistStart: async () => ({}), dbAssistSummary: async () => ({}), dbAssistClose: async () => {} },
    '../auth.svelte': { auth: { can: () => true } },
    '../../confirm.svelte': { confirmer: { ask: async (msg: string) => { asked.push(msg); return answers.shift(); } } },
    '../../components/exporters': { downloadText: () => {} },
  });
  const inserted: string[] = [];
  const a = new DbAssistStore({
    selectedConnId: () => 'c1',
    activeDb: () => null,
    currentStatement: () => 'SELECT mine',
    insertIntoQuery: (sql: string) => inserted.push(sql),
    runReadOnly: async () => {},
  });
  a.openAssist('nl');
  assert.equal(a.assistConnId, 'c1');
  a.assistProposedSql = 'SELECT theirs';
  assert.equal(await a.insertAssistSql(), false);
  assert.deepEqual(inserted, []);
  assert.equal(await a.insertAssistSql(), true);
  assert.deepEqual(inserted, ['SELECT theirs']);
  assert.equal(asked.length, 2);
});
