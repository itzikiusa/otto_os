import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture() {
  let token = 'first';
  const calls: { path: string; response: ReturnType<typeof deferred<any>> }[] = [];
  const { activity } = loadSource(new URL('../src/lib/stores/activity.svelte.ts', import.meta.url), {
    '../api/client': { getToken: () => token, api: { post: (path: string) => {
      const response = deferred<any>(); calls.push({ path, response }); return response.promise;
    }, get: (path: string) => {
      const response = deferred<any>(); calls.push({ path, response }); return response.promise;
    } } },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
  });
  return { activity, calls, setToken: (value: string) => { token = value; } };
}
const artifact = (id: string, at = '2026-10-18T10:00:00Z') => ({ id, produced_at: at, label: id, turn_id: at });
const artEvent = (a: any) => ({ type: 'artifact_added', session_id: 's', workspace_id: 'w', artifact: a });
const trail = (id: string) => ({ id, ts: id });
const tick = async () => { await new Promise(resolve => setImmediate(resolve)); };

test('artifact delta before hydration does not suppress the full snapshot', async () => {
  const h = fixture();
  h.activity.applyEvent(artEvent(artifact('new')));
  const pending = h.activity.loadArtifacts('s');
  assert.equal(h.calls.length, 1);
  h.calls[0].response.resolve([artifact('new'), artifact('old')]);
  await pending;
  assert.deepEqual(Array.from(h.activity.artifacts('s'), (a: any) => a.id), ['new', 'old']);
});

test('task and trail events win over a snapshot captured before their arrival', async () => {
  const h = fixture();
  const pending = h.activity.load('w', 's');
  h.activity.applyEvent({ type: 'trail_appended', session_id: 's', event: trail('B') });
  h.activity.applyEvent({ type: 'tasks_updated', session_id: 's', tasks: [{ id: 't', status: 'completed' }] });
  h.calls[0].response.resolve([trail('A')]);
  h.calls[1].response.resolve([{ id: 't', status: 'in_progress' }]);
  await pending;
  assert.deepEqual(Array.from(h.activity.trail('s'), (e: any) => e.id), ['A', 'B']);
  assert.equal(h.activity.tasks('s')[0].status, 'completed');
});

test('artifact hydration reconciles replacement deltas and newest-first ordering', async () => {
  const h = fixture();
  const pending = h.activity.loadArtifacts('s');
  h.activity.applyEvent(artEvent(artifact('same', '2026-10-18T12:00:00Z')));
  h.activity.applyEvent(artEvent(artifact('new', '2026-10-18T11:00:00Z')));
  h.calls[0].response.resolve([artifact('same'), artifact('old')]);
  await pending;
  assert.deepEqual(Array.from(h.activity.artifacts('s'), (a: any) => a.id), ['same', 'new', 'old']);
  assert.equal(h.activity.artifacts('s')[0].produced_at, '2026-10-18T12:00:00Z');
});

for (const boundary of ['reset', 'forget', 'token'] as const) {
  test(`${boundary} invalidates pending snapshots and their finalizers`, async () => {
    const h = fixture();
    const old = h.activity.loadArtifacts('s');
    if (boundary === 'reset') h.activity.reset();
    else if (boundary === 'forget') h.activity.forget('s');
    else h.setToken('second');
    h.calls[0].response.resolve([artifact('stale')]);
    await old;
    assert.equal(h.activity.artifacts('s').length, 0);
  });
}

test('late failures cannot add errors or clear a replacement load spinner', async () => {
  const h = fixture();
  const old = h.activity.loadArtifacts('s');
  h.activity.reset();
  const fresh = h.activity.loadArtifacts('s');
  h.calls[0].response.reject(new Error('old failure'));
  await old;
  assert.equal(h.activity.artifactsLoadingBySession.s, true);
  assert.equal(h.activity.artifactsErrorBySession.s, undefined);
  h.calls[1].response.resolve([artifact('fresh')]);
  await fresh;
  assert.equal(h.activity.artifacts('s')[0].id, 'fresh');
});

test('reconnect refreshes mounted consumers and invalidates unmounted hydration', async () => {
  const h = fixture();
  const off = h.activity.watchArtifacts('s');
  h.calls[0].response.resolve([artifact('old')]); await tick();
  const hidden = h.activity.loadArtifacts('hidden');
  h.calls[1].response.resolve([artifact('hidden-old')]); await hidden;
  const recovered = h.activity.resync();
  await tick();
  assert.equal(h.calls.length, 3, 'only the mounted session refreshes immediately');
  h.calls[2].response.resolve([artifact('new')]); await recovered;
  assert.equal(h.activity.artifacts('s')[0].id, 'new');
  const later = h.activity.loadArtifacts('hidden');
  assert.equal(h.calls.length, 4, 'unmounted data must rehydrate on reopen');
  h.calls[3].response.resolve([artifact('hidden-new')]); await later;
  off();
});

test('a late task mutation cannot resurrect a forgotten session', async () => {
  const h = fixture();
  const pending = h.activity.addTask('s', 'new');
  h.activity.forget('s');
  h.calls[0].response.resolve({ id: 'task', status: 'pending' });
  await pending;
  assert.equal(h.activity.tasks('s').length, 0);
});

test('task created during a snapshot read survives without a WS echo', async () => {
  const h = fixture();
  const pending = h.activity.load('w', 's');
  const added = h.activity.addTask('s', 'new');
  h.calls[2].response.resolve({ id: 'new', status: 'pending' }); await added;
  h.calls[0].response.resolve([]);
  h.calls[1].response.resolve([{ id: 'old', status: 'completed' }]); await pending;
  assert.deepEqual(Array.from(h.activity.tasks('s'), (t: any) => t.id), ['old', 'new']);
});

test('reset drops old task snapshots and old failures after a fresh load', async () => {
  const h = fixture();
  const old = h.activity.load('w', 's');
  h.activity.reset();
  const fresh = h.activity.load('w', 's');
  h.calls[2].response.resolve([trail('fresh')]); h.calls[3].response.resolve([]); await fresh;
  h.calls[0].response.resolve([trail('stale')]); h.calls[1].response.reject(new Error('obsolete')); await old;
  assert.equal(h.activity.trail('s')[0].id, 'fresh');
  assert.equal(h.activity.loadErrorBySession.s, undefined);
});

test('a newer forced read wins even when the old request resolves last', async () => {
  const h = fixture();
  const old = h.activity.loadArtifacts('s');
  const fresh = h.activity.loadArtifacts('s', true);
  assert.equal(h.calls.length, 2);
  h.calls[1].response.resolve([artifact('fresh')]); await fresh;
  h.calls[0].response.resolve([artifact('stale')]); await old;
  assert.equal(h.activity.artifacts('s')[0].id, 'fresh');
});

test('rollup snapshots preserve events received in flight and reject obsolete workspaces', async () => {
  const h = fixture();
  const first = h.activity.loadSummary('old-w');
  const next = h.activity.loadSummary('w');
  h.activity.applyEvent({ type: 'tasks_updated', session_id: 's', tasks: [{ id: 't', status: 'completed' }] });
  h.calls[1].response.resolve([{ session_id: 's', total: 1, done: 0, in_progress: 'task', last_ts: null }]); await next;
  h.calls[0].response.resolve([{ session_id: 'stale', total: 2, done: 0 }]); await first;
  assert.equal(h.activity.summary('s').done, 1);
  assert.equal(h.activity.summary('stale'), null);
});

test('multiple mounted consumers share hydration and unmount prevents gap refetch', async () => {
  const h = fixture();
  const offA = h.activity.watch('w', 's');
  const offB = h.activity.watch('w', 's');
  assert.equal(h.calls.length, 2);
  h.calls[0].response.resolve([]); h.calls[1].response.resolve([]); await tick();
  offA(); offB();
  await h.activity.resync();
  assert.equal(h.calls.length, 2);
});

test('a task mutation finishing across a reconnect still updates the same identity', async () => {
  const h = fixture();
  const pending = h.activity.addTask('s', 'new');
  await h.activity.resync();
  h.calls[0].response.resolve({ id: 'new', status: 'pending' }); await pending;
  assert.equal(h.activity.tasks('s')[0]?.id, 'new');
});

test('a failed hydration stays retryable even when an event arrived during it', async () => {
  const h = fixture();
  const first = h.activity.loadArtifacts('s');
  h.activity.applyEvent(artEvent(artifact('event')));
  h.calls[0].response.reject(new Error('offline')); await first;
  const second = h.activity.loadArtifacts('s');
  assert.equal(h.calls.length, 2);
  h.calls[1].response.resolve([artifact('event'), artifact('old')]); await second;
  assert.equal(h.activity.artifacts('s').length, 2);
  assert.equal(h.activity.artifactsErrorBySession.s, undefined);
});
