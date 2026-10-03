import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function client() {
  return loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {}),
  }, { location: { port: '7700', origin: 'http://localhost:7700' }, fetch: async () => new Response('{}') });
}

test('a clicked commit diff stays on the interactive lane', () => {
  const { isLongPath } = client();
  // The alt host is a separate origin: each distinct diff URL would pay its own
  // CORS preflight there, pushing the huge-branch file list past its budget.
  assert.equal(isLongPath('/repos/r1/diff?target=commit:abc123&summary=true'), false);
  assert.equal(isLongPath('/repos/r1/diff?target=commit:abc123&path=src%2Fa.rs'), false);
});

test('history walks and remote git still use the long lane', () => {
  const { isLongPath } = client();
  assert.equal(isLongPath('/repos/r1/log?all=true&limit=10000'), true);
  assert.equal(isLongPath('/repos/r1/fetch'), true);
  assert.equal(isLongPath('/repos/r1/prs'), true);
});

test('a DB schema load or object open (both can dial) uses the long lane', () => {
  const { isLongPath } = client();
  // `GET …/db/schema` is the call that dials (keychain + ssh + pool connect):
  // on the interactive lane a parallel reconnect starved the whole app.
  assert.equal(isLongPath('/connections/c1/db/schema'), true);
  assert.equal(isLongPath('/connections/c1/db/schema?refresh=true'), true);
  assert.equal(isLongPath('/connections/c1/db/object'), true);
  assert.equal(isLongPath('/connections/c1/db/schema/children?path=a'), true);
  assert.equal(isLongPath('/connections/c1/db/schema-graph'), true);
  // Neighbours that merely share a prefix stay interactive.
  assert.equal(isLongPath('/connections/c1/db/objects-pinned'), false);
  assert.equal(isLongPath('/connections/c1/db/schemas'), false);
});
