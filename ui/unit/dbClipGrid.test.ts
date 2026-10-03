// The clipboard ring (copies made in Otto) and the per-tab results-grid state.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { dropGridState, parkedEditCount, stashGridState, takeGridState } from '../src/modules/database/grid-tab-state.ts';

type ClipEntry = { text: string; source: string; at: number; pinned: boolean };
const { pushClip } = loadSource(new URL('../src/lib/stores/clipHistory.svelte.ts', import.meta.url), {}) as {
  pushClip: (ring: ClipEntry[], text: string, source: string, at: number) => ClipEntry[];
};

test('clip ring: newest first, deduped, pinned entries survive the cap', () => {
  let ring: ClipEntry[] = [];
  ring = pushClip(ring, 'a', 'Results', 1);
  ring = pushClip(ring, 'b', 'Editor', 2);
  ring = pushClip(ring, 'a', 'Editor', 3);
  assert.deepEqual([...ring.map((e) => e.text)], ['a', 'b']);
  assert.equal(ring[0].source, 'Editor');
  ring = ring.map((e) => (e.text === 'b' ? { ...e, pinned: true } : e));
  for (let i = 0; i < 60; i++) ring = pushClip(ring, `x${i}`, 'Results', 10 + i);
  assert.equal(ring.length, 50);
  assert.ok(ring.some((e) => e.text === 'b' && e.pinned), 'a pinned entry is never evicted');
  assert.equal(ring[0].text, 'x59');
  // Oversized and empty copies are not recorded.
  assert.equal(pushClip([], 'z'.repeat(64 * 1024 + 1), 'Results', 1).length, 0);
  assert.equal(pushClip([], '', 'Results', 1).length, 0);
});

test('grid state: parked per tab, taken once, dropped on close', () => {
  const result = {};
  stashGridState('t1', {
    result,
    colKey: 'id\u0001name',
    search: 'bob',
    sortCol: 1,
    sortDir: 'desc',
    colFilters: { 0: '>3' },
    pending: new Map([[2, { cells: new Map() }]]),
  });
  assert.equal(parkedEditCount('t1'), 1);
  const got = takeGridState('t1');
  assert.equal(got?.search, 'bob');
  assert.equal(got?.result, result);
  assert.equal(takeGridState('t1'), null, 'taken state is removed');
  stashGridState('t2', { result, colKey: '', search: '', sortCol: null, sortDir: null, colFilters: {}, pending: new Map() });
  dropGridState('t2');
  assert.equal(takeGridState('t2'), null);
});

test('editor history: memory tier outlives the editor, bounded, forgettable', async () => {
  const h = await import('../src/lib/editor-history.ts');
  h.saveEditorState('dbtab:a', { doc: 'SELECT 1', history: { done: [] } }, 40, true);
  assert.equal(h.loadEditorState('dbtab:a')?.json.doc, 'SELECT 1');
  assert.equal(h.loadEditorState('dbtab:a')?.scrollTop, 40);
  for (let i = 0; i < 60; i++) h.saveEditorState(`dbtab:x${i}`, { doc: String(i) }, 0, false);
  assert.equal(h.loadEditorState('dbtab:a'), null, 'least-recently used entries are evicted');
  assert.equal(h.loadEditorState('dbtab:x59')?.json.doc, '59');
  h.forgetEditorStates('dbtab:x');
  assert.equal(h.loadEditorState('dbtab:x59'), null);
  await h.flushEditorHistory(); // no IndexedDB in node: a no-op, never a throw
});
