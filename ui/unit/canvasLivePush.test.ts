// Canvas live-edit pushes are consumed once (r3-02-01). The editors' live
// effect used to re-run whenever `dirty` flipped, so after the user's SECOND
// save the same (stale) agent push was re-applied and the board reverted to
// the agent's old drawing. `ingestPush` spends a bus tick the first time it
// is seen, whatever the outcome.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function store() {
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': { api: {}, getToken: () => 'tok' },
    './workspace.svelte': { ws: { currentId: 'w1' } },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../../modules/canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  c.currentId = 'scene-a';
  return c;
}

const agentDoc = { type: 'otto-canvas', version: 1, format: 'excalidraw', source: 'S_agent' };

test('a push applies once; saves (dirty → false) never re-apply it', () => {
  const c = store();
  // 1. The agent push lands.
  assert.equal(c.ingestPush(1, 'scene-a', agentDoc), true);
  assert.equal(c.source, 'S_agent');
  // 2. The user draws, 3. the first save lands; the effect re-runs with the
  //    same tick (the old bug path) — nothing is re-applied.
  c.dirty = true;
  c.markSaved({ elements: [] });
  c.source = 'doc1'; // saveNow records the user's own source
  assert.equal(c.ingestPush(1, 'scene-a', agentDoc), false);
  assert.equal(c.source, 'doc1');
  // 4. The second save: still the user's scene, not the stale agent one.
  c.dirty = true;
  c.markSaved({ elements: [1] });
  c.source = 'doc2';
  assert.equal(c.ingestPush(1, 'scene-a', agentDoc), false);
  assert.equal(c.source, 'doc2');
  // A NEW push (next tick) still applies.
  assert.equal(c.ingestPush(2, 'scene-a', { ...agentDoc, source: 'S_agent2' }), true);
  assert.equal(c.source, 'S_agent2');
});

test('a push skipped while dirty is spent, not replayed once the save lands', () => {
  const c = store();
  c.dirty = true;
  assert.equal(c.ingestPush(5, 'scene-a', agentDoc), false);
  c.markSaved({});
  assert.equal(c.ingestPush(5, 'scene-a', agentDoc), false);
  assert.equal(c.source, null);
});

test('another scene\'s push, docs without source, and tick 0 are ignored', () => {
  const c = store();
  assert.equal(c.ingestPush(0, 'scene-a', agentDoc), false);
  assert.equal(c.ingestPush(1, 'scene-b', agentDoc), false);
  assert.equal(c.ingestPush(2, 'scene-a', { type: 'otto-canvas' }), false);
  assert.equal(c.ingestPush(3, 'scene-a', null), false);
  assert.equal(c.source, null);
  // Opening scene-b later does not resurrect its already-seen push.
  c.currentId = 'scene-b';
  assert.equal(c.ingestPush(1, 'scene-b', agentDoc), false);
});
