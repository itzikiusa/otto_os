import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
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

test('the DB keep-alive ping rides the long lane, not a background slot', () => {
  // A db/test over a tunnel takes ~1 s (up to 10 s in the field); two of them
  // on `api.bg` held both of the window's background slots each round, so
  // every poll behind them queued (ui.request.queue max 751 ms).
  const src = readFileSync(new URL('../src/lib/stores/database/connections.svelte.ts', import.meta.url), 'utf8');
  const start = src.indexOf('async keepAliveTick');
  const tick = src.slice(start, src.indexOf('\n  }\n', start));
  assert.match(tick, /api\.long\.post<DbTestResult>\(`\$\{this\.db\.connBase\(id\)\}\/test`/);
  assert.doesNotMatch(tick, /api\.bg\./);
});

test('a slow background read on a long path never holds the fast polls\' two slots', async () => {
  // A poll tick inherits the bg lane; a long path in it (a usage summary, a
  // k8s / Kafka read, repo status) took ~1 s and held one of the two slots.
  const lane = loadSource(new URL('../src/lib/api/lane.ts', import.meta.url), {});
  const fetched: string[] = [];
  const c = loadSource(new URL('../src/lib/api/client.ts', import.meta.url), {
    '../stores/serviceHealth.svelte': { serviceHealth: { report() {} } },
    './lane': lane,
  }, {
    location: { port: '7700', origin: 'http://localhost:7700' },
    fetch: (url: string) => {
      fetched.push(String(url));
      return /\/usage\//.test(String(url)) ? new Promise(() => {}) : Promise.resolve(new Response('{}'));
    },
  });
  lane.inLane('bg', () => {
    void c.api.get('/usage/summary?days=7').catch(() => {});
    void c.api.get('/usage/status').catch(() => {});
  });
  const fast = lane.inLane('bg', () => c.api.get('/notifications'));
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(fetched.some((u) => u.includes('/notifications')), 'the fast poll went out: ' + fetched.join(' '));
  await fast;
});
