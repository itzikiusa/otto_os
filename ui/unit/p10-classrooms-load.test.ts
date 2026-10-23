// ui-correctness-6-02: the Classrooms box's load (home/boxes/classrooms/
// load.svelte.ts) must not report an aborted read as fresh data, and an
// aborted first load keeps the skeleton (loaded stays false).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function setup(get: (path: string, signal?: AbortSignal) => Promise<unknown>) {
  const { SchoolData } = loadSource(new URL('../src/modules/home/boxes/classrooms/load.svelte.ts', import.meta.url), {
    '../../../../lib/api/client': { api: { bg: { get } } },
    '../../../../lib/stores/workspace.svelte': { visibleOnThisDevice: (s: { hidden?: boolean }) => !s.hidden },
  });
  return new SchoolData();
}

test('a successful load assigns the visible rows and reports fresh', async () => {
  const d = setup(async (path) => (path.includes('archived=true') ? [{ id: 'a1' }] : [{ id: 's1' }, { id: 's2', hidden: true }]));
  assert.equal(await d.load(), true);
  assert.deepEqual(d.fetched.map((s: { id: string }) => s.id), ['s1']);
  assert.deepEqual(d.archivedRows.map((s: { id: string }) => s.id), ['a1']);
  assert.equal(d.loaded, true);
  assert.equal(d.error, '');
});

test('an aborted load is not fresh, records no error and keeps the first-load skeleton', async () => {
  const ac = new AbortController();
  const d = setup(async (_path, signal) => {
    ac.abort();
    assert.equal(signal, ac.signal);
    throw new DOMException('aborted', 'AbortError');
  });
  assert.equal(await d.load(ac.signal), false, 'freshness must not stamp an aborted run');
  assert.equal(d.loaded, false);
  assert.equal(d.error, '');
  assert.equal(d.fetched.length, 0);
});

test('a real failure is not fresh, shows the load-error copy and ends the skeleton', async () => {
  const d = setup(async () => {
    throw new TypeError('Load failed');
  });
  assert.equal(await d.load(), false);
  assert.equal(d.loaded, true);
  assert.notEqual(d.error, '');
  assert.ok(!/^Load failed$/.test(d.error), `raw transport text leaked: ${d.error}`);
});
