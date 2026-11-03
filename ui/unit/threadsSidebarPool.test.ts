// Threads sidebar membership (threadsHelpers.isThreadSession): the sidebar
// passes an id index so a delegated worker's parent lookup is O(1); the
// answer must be identical to the array scan.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const helpers = loadSource(new URL('../src/modules/threads/threadsHelpers.ts', import.meta.url), {
  '../../lib/stores/workspace.svelte': {
    ws: { sessions: [] },
    isThreadSurfaceSession: (s: { meta?: Record<string, unknown> }) => s.meta?.surface === 'threads',
  },
});

const s = (id: string, meta: Record<string, unknown> = {}) => ({ id, kind: 'agent', meta });

test('isThreadSession answers the same from an id index as from the session list', () => {
  const pool = [s('lead', { surface: 'threads' }), s('worker', { delegated_by: 'lead' }), s('agent'), s('orphan', { delegated_by: 'gone' }), s('side', { side_of: 'lead', surface: 'threads' })];
  const index = new Map(pool.map((x) => [x.id, x] as const));
  for (const x of pool) {
    assert.equal(helpers.isThreadSession(x, index), helpers.isThreadSession(x, pool), x.id);
  }
  assert.deepEqual(pool.filter((x) => helpers.isThreadSession(x, index)).map((x) => x.id), ['lead', 'worker']);
});
