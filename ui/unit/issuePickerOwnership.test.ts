import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred, loadSource } from './sourceHarness.ts';
import { unwrapListing } from '../src/lib/listingPage.ts';

/** The REAL issue client (lib/api/issues.ts) over the test's `get` stub, so a
 *  fixture sees the exact (encoded) URLs the components request. */
function issueClient(get: (path: string) => Promise<unknown>) {
  return loadSource(new URL('../src/lib/api/issues.ts', import.meta.url), {
    './client': { api: { get }, authedBlobUrl: async () => '' },
    '../listingPage': { unwrapListing },
  });
}

// The shared issue picker (Attach issue / Review → Jira). A search or project
// list started under one account must not land after the user switched to
// another: picking from it attached account A's issue key under account B.

type Get = (url: string) => Promise<unknown>;

function picker(get: Get) {
  return componentFunctions(
    new URL('../src/modules/agents/JiraIssuePicker.svelte', import.meta.url),
    ['loadProjects', 'resetSearch', 'search', 'pick', 'loadAccounts'],
    {
      ...issueClient(get),
      issueAccounts: { load: () => get('/issue/accounts') },
      loadErrorText: (e: Error) => e.message,
      accounts: [], accountsLoading: true, accountsError: null,
      selectedAccountId: 'A', projects: [], projectsLoading: false, projectsTruncated: false,
      selectedProjectKey: '', projectsSeq: 0,
      query: '', results: [], searching: false, searchError: null, searchSeq: 0, debounceTimer: null,
      picked: [] as unknown[],
      onpick(issue: unknown) { (this as { picked: unknown[] }).picked.push(issue); },
    },
  );
}

test('a search still running for the previous account never lands after a switch', async () => {
  const oldSearch = deferred<unknown>();
  const s = picker((url) => (url.startsWith('/issue/search') ? oldSearch.promise : Promise.resolve({ items: [], truncated: false })));
  s.query = 'login';
  const pending = s.search('login');
  s.selectedAccountId = 'B';
  await s.loadProjects('B');
  oldSearch.resolve([{ key: 'A-1', summary: 'from account A', status: 'Open' }]);
  await pending;
  assert.equal(s.results.length, 0);
  assert.equal(s.searching, false);
  assert.equal(s.query, '');
});

test('an older account’s project list cannot replace the newer one’s', async () => {
  const slowA = deferred<unknown>();
  const s = picker((url) =>
    url.includes('account_id=A')
      ? slowA.promise
      : Promise.resolve({ items: [{ key: 'BEE', name: 'B project' }], truncated: false }),
  );
  const a = s.loadProjects('A');
  s.selectedAccountId = 'B';
  await s.loadProjects('B');
  slowA.resolve({ items: [{ key: 'AYE', name: 'A project' }], truncated: true });
  await a;
  assert.deepEqual(s.projects.map((p: { key: string }) => p.key), ['BEE']);
  assert.equal(s.projectsTruncated, false);
  assert.equal(s.projectsLoading, false);
});

test('a failed account load is an error with Retry, not “no accounts configured”', async () => {
  const s = picker(async () => { throw new Error('daemon unreachable'); });
  await s.loadAccounts();
  assert.equal(s.accountsError, 'daemon unreachable');
  assert.equal(s.accountsLoading, false);
  const ok = picker(async () => [{ id: 'X', label: 'Work', base_url: 'https://x' }]);
  await ok.loadAccounts();
  assert.equal(ok.accountsError, null);
  assert.equal(ok.selectedAccountId, 'X');
});
