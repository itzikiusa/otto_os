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

function fixture(get: (path: string) => Promise<unknown>, mode: 'story' | 'rfc') {
  return componentFunctions(new URL('../src/modules/product/PublishDialog.svelte', import.meta.url),
    ['loadProjects', 'loadSpaces', 'loadIssueTypes'], {
      ...issueClient(get), mode, accountId: 'A', projects: [], projectsLoading: false, projectKey: '',
      issueTypes: [], issueTypesLoading: false, issueType: 'Story', spaces: [], spacesLoading: false,
      spaceKey: '', parentId: '123', rfcTitle: 'My RFC title', setError() {},
      projectsSequence: 0, spacesSequence: 0, issueTypesSequence: 0,
      projectsError: '', spacesError: '', formError: '',
      projectsTruncated: false, spacesTruncated: false, unwrapListing,
      providerErrorText: (e: unknown) => e instanceof Error ? e.message : String(e),
    });
}

for (const mode of ['story', 'rfc'] as const) {
  test(`late ${mode} destinations cannot replace the newly selected account`, async () => {
    const request = deferred<unknown>();
    const state = fixture(async path => path.includes('issue-types') ? ['Story']
      : path.includes('account_id=A') ? request.promise : [{ key: 'B', name: 'Account B' }], mode);
    const load = mode === 'story' ? 'loadProjects' : 'loadSpaces';
    const pending = state[load]();
    state.accountId = 'B'; await state[load]();
    request.resolve([{ key: 'A', name: 'Account A' }]); await pending;
    assert.equal(mode === 'story' ? state.projectKey : state.spaceKey, 'B');
  });
}

test('destination retry preserves the typed RFC title and parent', async () => {
  let attempt = 0;
  const state = fixture(async () => { if (++attempt === 1) throw new Error('Offline'); return [{ key: 'RFC', name: 'RFCs' }]; }, 'rfc');
  await state.loadSpaces(); await state.loadSpaces();
  assert.equal(state.spaceKey, 'RFC');
  assert.equal(state.rfcTitle, 'My RFC title'); assert.equal(state.parentId, '123');
});
