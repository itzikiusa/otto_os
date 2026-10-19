// Read-only probe of the real Canvas store via the existing unit source harness.
// Run: node docs/reviews/product-wide-2026-10-18/canvas-scene-guard-probe.mjs
// No daemon, user data, or production files are touched.
import assert from 'node:assert/strict';
import { loadSource } from '../../../ui/unit/sourceHarness.ts';
const doc = source => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
const { canvas } = loadSource(new URL('../../../ui/src/lib/stores/canvas.svelte.ts', import.meta.url), {
  '../api/client': {
    api: {
      put: async () => ({}), post: async () => ({}),
      get: async () => ({ id: 'b', doc_json: JSON.stringify(doc('B')), title: 'B', updated_at: new Date().toISOString() }),
    },
    getToken: () => 'fixture-token',
  },
  './workspace.svelte': { ws: { currentId: 'fixture-workspace' } },
  '../labels': { NO_WORKSPACE: 'workspace' },
  '../providers': { defaultAgentProvider: () => 'codex' },
  '../loadError': { loadErrorText: String },
  '../../modules/canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: value => value },
});
canvas.currentId = 'a';
await canvas.assist('change');
await canvas.persistDoc('a', doc('A-saved'));
canvas.finishAssistTurn();
await canvas.open('b');
assert.equal(canvas.currentId, 'b');
assert.equal(canvas.source, 'B');
assert.equal(canvas.ingestDoc(doc('B-live'), 'b'), true, 'scene A save protection must not block scene B');
assert.equal(canvas.source, 'B-live');
console.log('PASS: scene B receives its own live document after an assisted save in A');
