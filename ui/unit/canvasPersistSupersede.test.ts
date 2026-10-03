// F5 (perf): a queued canvas autosave whose doc a newer draft superseded is
// skipped — only the first in-flight body and the newest one go out, not every
// multi-MB intermediate doc.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

test('queued, superseded scene writes are dropped; the newest doc is sent', async () => {
  const sent: unknown[] = [];
  let release!: () => void;
  const gate = new Promise<void>((r) => { release = r; });
  const api = {
    put: async (_path: string, body: { doc: unknown }) => {
      sent.push(body.doc);
      if (sent.length === 1) await gate; // hold the first write in flight
      return {};
    },
  };
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': { api, getToken: () => 'tok' },
    './workspace.svelte': { ws: { currentId: 'w1' } },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../../modules/canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  const d = (n: number) => ({ type: 'otto-canvas', version: 1, format: 'excalidraw', source: `s${n}` });
  const docs = [d(1), d(2), d(3), d(4)];
  const writes = [c.persistDoc('scene-a', docs[0])];
  await new Promise((r) => setImmediate(r)); // the first PUT is now in flight
  for (const doc of docs.slice(1)) writes.push(c.persistDoc('scene-a', doc));
  release();
  await Promise.all(writes);
  assert.deepEqual(sent, [docs[0], docs[3]]);
});

test('drafts staged in one tick collapse into a single PUT', async () => {
  const sent: unknown[] = [];
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': { api: { put: async (_p: string, b: { doc: unknown }) => { sent.push(b.doc); return {}; } }, getToken: () => 'tok' },
    './workspace.svelte': { ws: { currentId: 'w1' } },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../../modules/canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  const a = { source: 'a' };
  const b = { source: 'b' };
  await Promise.all([c.persistDoc('scene-a', a), c.persistDoc('scene-a', b)]);
  assert.deepEqual(sent, [b]);
});
