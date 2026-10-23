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
    './workspace.svelte': { ws: { currentId: 'w1' } }, '../labels': { NO_WORKSPACE: 'Select a workspace first' },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
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

function guardedStore() {
  let release!: (value: unknown) => void;
  const pending = new Promise((resolve) => { release = resolve; });
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': {
      api: { put: async () => ({}), post: () => pending },
      getToken: () => 'tok',
    },
    './workspace.svelte': { ws: { currentId: 'w1' } }, '../labels': { NO_WORKSPACE: 'Select a workspace first' },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  c.currentId = 'scene-a';
  return {
    c,
    doc: (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source }),
    settle: () => { release({}); return pending; },
  };
}

test('a save of B during Ask Otto rejects a later preview of A, including after the turn', async () => {
  const { c, doc, settle } = guardedStore();
  await c.persistDoc('scene-a', doc('A'));
  c.source = 'A';
  const turn = c.assist('redraw');
  await c.persistDoc('scene-a', doc('B'));
  c.source = 'B';
  assert.equal(c.assistKeptSaved('agent preview'), true);
  assert.equal(c.ingestPush(1, 'scene-a', doc('A')), false);
  assert.equal(c.source, 'B');
  assert.equal(c.ingestDoc(doc('B'), 'scene-a'), true);
  c.finishAssistTurn();
  assert.equal(c.ingestPush(2, 'scene-a', doc('A')), false);
  assert.equal(c.source, 'B');
  assert.equal(c.ingestPush(3, 'scene-a', doc('B')), true);
  await settle();
  await turn;
});

test('a live preview still applies when the turn saved nothing', async () => {
  const { c, doc, settle } = guardedStore();
  await c.persistDoc('scene-a', doc('A'));
  c.source = 'A';
  const turn = c.assist('redraw');
  assert.equal(c.assistKeptSaved(''), false);
  assert.equal(c.ingestPush(1, 'scene-a', doc('preview')), true);
  assert.equal(c.source, 'preview');
  c.finishAssistTurn();
  assert.equal(c.ingestPush(2, 'scene-a', doc('final')), true);
  await settle();
  await turn;
});

test('a stop reloads after an in-flight save and drops the snapshot from before that save', async () => {
  let releasePut!: (value: unknown) => void;
  const putPending = new Promise((resolve) => { releasePut = resolve; });
  let releaseGet!: (value: unknown) => void;
  const getPending = new Promise((resolve) => { releaseGet = resolve; });
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': {
      api: { put: () => putPending, post: async () => ({}), get: () => getPending },
      getToken: () => 'tok',
    },
    './workspace.svelte': { ws: { currentId: 'w1' } }, '../labels': { NO_WORKSPACE: 'Select a workspace first' },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  c.currentId = 'scene-a';
  const doc = (source: string, format = 'mermaid') => ({ type: 'otto-canvas', version: 1, format, source });
  const save = c.persistDoc('scene-a', doc('B'));
  const reload = c.reloadAfterStop('scene-a');
  assert.equal(c.ingestPush(1, 'scene-a', doc('A')), false);
  releasePut({});
  await save;
  releaseGet({ id: 'scene-a', doc_json: JSON.stringify(doc('B', 'd2')), session_id: null });
  assert.equal(await reload, true);
  assert.equal(c.source, 'B');
  assert.equal(c.format, 'd2');
  assert.equal(c.ingestPush(2, 'scene-a', doc('A')), false);
  assert.equal(c.source, 'B');
});

test('a stop note keeps the board even when this client did not save during the turn', async () => {
  const { c, doc, settle } = guardedStore();
  const turn = c.assist('redraw');
  assert.equal(c.assistKeptSaved('Stopped — your saved changes were preserved.'), true);
  c.finishAssistTurn();
  assert.equal(c.ingestDoc(doc('current'), 'scene-a'), true);
  await settle();
  await turn;
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

test('scene A save protection during Ask Otto does not block scene B after open', async () => {
  let settleAssist!: () => void;
  const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': {
      api: {
        put: async () => ({}),
        post: () => new Promise((resolve) => { settleAssist = () => resolve({}); }),
        get: async (url: string) => ({
          id: url.includes('/b') ? 'b' : 'a',
          title: 'Scene',
          doc_json: JSON.stringify(doc(url.includes('/b') ? 'B1' : 'A1')),
          updated_at: '2026-10-18T00:00:00Z',
          session_id: null,
        }),
      },
      getToken: () => 'tok',
    },
    './workspace.svelte': { ws: { currentId: 'w1' } }, '../labels': { NO_WORKSPACE: 'Select a workspace first' },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  await c.open('a');
  const turn = c.assist('draw A');
  await c.persistDoc('a', doc('A-saved'));
  c.finishAssistTurn('a');
  settleAssist();
  await turn;

  await c.open('b');
  assert.equal(c.ingestDoc(doc('B-live'), 'b'), true);
  assert.equal(c.source, 'B-live');
  assert.equal(c.ingestPush(1, 'b', doc('B-push')), true);
  assert.equal(c.source, 'B-push');
});

test('late scene-A save and late scene-A finishAssistTurn do not contaminate scene B', async () => {
  const puts: Array<() => void> = [];
  const posts: Array<() => void> = [];
  const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': {
      api: {
        put: () => new Promise((resolve) => { puts.push(() => resolve({})); }),
        post: () => new Promise((resolve) => { posts.push(() => resolve({})); }),
        get: async (url: string) => ({
          id: url.includes('/b') ? 'b' : 'a',
          title: 'Scene',
          doc_json: JSON.stringify(doc(url.includes('/b') ? 'B-init' : 'A-init')),
          updated_at: '2026-10-18T00:00:00Z',
          session_id: null,
        }),
      },
      getToken: () => 'tok',
    },
    './workspace.svelte': { ws: { currentId: 'w1' } }, '../labels': { NO_WORKSPACE: 'Select a workspace first' },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  await c.open('a');
  const turnA = c.assist('draw A');
  const saveA = c.persistDoc('a', doc('A-late-save'));

  // Switch to scene B while scene A's save and assist turn are still in flight.
  await c.open('b');
  const turnB = c.assist('draw B');

  // Late PUT completion for scene A must not mark scene B as having saved during turn B.
  puts.shift()!();
  await saveA;
  assert.equal(c.assistKeptSaved('', 'b'), false);
  assert.equal(c.ingestPush(1, 'b', doc('B-preview-1')), true);
  assert.equal(c.source, 'B-preview-1');

  // Late finishAssistTurn from scene A must not end or pin scene B's active turn.
  posts.shift()!();
  await turnA;
  assert.equal(c.assistKeptSaved('', 'a'), false);
  c.finishAssistTurn('a');

  // Scene B now saves during its own active turn; B's guard must still be active.
  const saveB = c.persistDoc('b', doc('B-saved'));
  await Promise.resolve();
  puts.shift()!();
  await saveB;
  assert.equal(c.assistKeptSaved('', 'b'), true);
  assert.equal(c.ingestPush(2, 'b', doc('B-stale-preview')), false);
  assert.equal(c.source, 'B-saved');

  c.finishAssistTurn('b');
  assert.equal(c.ingestPush(3, 'b', doc('B-stale-after')), false);
  assert.equal(c.ingestPush(4, 'b', doc('B-saved')), true);
  posts.shift()!();
  await turnB;
});

test('root sequence: assisted save in A, finish, open B, ingest B-live', async () => {
  const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': {
      api: {
        put: async () => ({}),
        post: async () => ({}),
        get: async () => ({
          id: 'b',
          title: 'B',
          doc_json: JSON.stringify(doc('B')),
          updated_at: '2026-10-18T00:00:00Z',
          session_id: null,
        }),
      },
      getToken: () => 'fixture-token',
    },
    './workspace.svelte': { ws: { currentId: 'fixture-workspace' } },
    '../labels': { NO_WORKSPACE: 'workspace' },
    '../providers': { defaultAgentProvider: () => 'codex' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  c.currentId = 'a';
  await c.assist('change');
  await c.persistDoc('a', doc('A-saved'));
  c.finishAssistTurn();
  await c.open('b');
  const before = c.source;
  const accepted = c.ingestDoc(doc('B-live'), 'b');
  const output = { scene: c.currentId, before, accepted, after: c.source, expectedAccepted: true };
  assert.deepEqual(output, { scene: 'b', before: 'B', accepted: true, after: 'B-live', expectedAccepted: true });
});

test('late scene-A save and no-arg finish do not arm scene B, including after auth and close', async () => {
  const puts: Array<() => void> = [];
  let token = 'tok-a';
  const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': {
      api: {
        put: () => new Promise((resolve) => { puts.push(() => resolve({})); }),
        post: async () => ({}),
        get: async (url: string) => ({
          id: url.includes('/b') ? 'b' : 'a',
          title: 'Scene',
          doc_json: JSON.stringify(doc(url.includes('/b') ? 'B' : 'A')),
          updated_at: '2026-10-18T00:00:00Z',
          session_id: null,
        }),
      },
      getToken: () => token,
    },
    './workspace.svelte': { ws: { currentId: 'w1' } },
    '../labels': { NO_WORKSPACE: 'workspace' },
    '../providers': { defaultAgentProvider: () => 'codex' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  await c.open('a');
  const turn = c.assist('draw A');
  const saveA = c.persistDoc('a', doc('A-late'));
  // Finish while the PUT is still in flight, then leave the scene.
  c.finishAssistTurn();
  await c.open('b');
  c.finishAssistTurn();
  c.finishAssistTurn('a');
  puts.shift()!();
  await saveA;
  assert.equal(c.assistKeptSaved('', 'b'), false);
  const before = c.source;
  const accepted = c.ingestDoc(doc('B-live'), 'b');
  assert.deepEqual(
    { scene: c.currentId, before, accepted, after: c.source, expectedAccepted: true },
    { scene: 'b', before: 'B', accepted: true, after: 'B-live', expectedAccepted: true },
  );
  // Scene B's own turn still rejects a stale preview after B saves.
  const turnB = c.assist('draw B');
  c.finishAssistTurn('a');
  const saveB = c.persistDoc('b', doc('B-saved'));
  await Promise.resolve();
  puts.shift()!();
  await saveB;
  assert.equal(c.ingestDoc(doc('B-stale'), 'b'), false);
  assert.equal(c.source, 'B-saved');
  await turn;
  await turnB;

  token = 'tok-b';
  c.resetAuthContext();
  await c.open('b');
  assert.equal(c.ingestDoc(doc('B-after-auth'), 'b'), true);
  assert.equal(c.source, 'B-after-auth');
  c.closeScene();
  await c.open('a');
  assert.equal(c.ingestDoc(doc('A-after-close'), 'a'), true);
  assert.equal(c.source, 'A-after-close');
});

test('a save queued during Ask Otto still pins when it lands after finish, and the next scene stays open', async () => {
  const puts: Array<() => void> = [];
  const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': {
      api: {
        put: () => new Promise((resolve) => { puts.push(() => resolve({})); }),
        post: async () => ({}),
        get: async (url: string) => ({
          id: url.includes('/b') ? 'b' : 'a',
          title: 'Scene',
          doc_json: JSON.stringify(doc(url.includes('/b') ? 'B' : 'A')),
          updated_at: '2026-10-18T00:00:00Z',
          session_id: null,
        }),
      },
      getToken: () => 'tok',
    },
    './workspace.svelte': { ws: { currentId: 'w1' } },
    '../labels': { NO_WORKSPACE: 'workspace' },
    '../providers': { defaultAgentProvider: () => 'codex' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  const c = mod.canvas;
  await c.open('a');
  const turn = c.assist('draw A');
  const save = c.persistDoc('a', doc('A-saved'));
  await Promise.resolve();
  await Promise.resolve();
  c.finishAssistTurn('a');
  puts.shift()!();
  await save;
  assert.equal(c.ingestDoc(doc('A-stale'), 'a'), false);
  assert.equal(c.ingestDoc(doc('A-saved'), 'a'), true);
  await c.open('b');
  c.finishAssistTurn('a');
  assert.equal(c.ingestDoc(doc('B-live'), 'b'), true);
  assert.equal(c.source, 'B-live');
  await turn;
});

test('CanvasPanel lazily imports canvas store inside openInCanvas, sets pendingOpenId before routing, and toasts on failure', async () => {
  const { readFileSync } = await import('node:fs');
  const { compile } = await import('svelte/compiler');
  const panelUrl = new URL('../src/modules/panels/CanvasPanel.svelte', import.meta.url);
  const panelSource = readFileSync(panelUrl, 'utf8');

  // Static top-level import of canvas.svelte must be gone so workspace shell stays under budget.
  assert.doesNotMatch(panelSource, /^\s*import\s+\{\s*canvas\s*\}\s+from\s+['"]\.\.\/\.\.\/lib\/stores\/canvas\.svelte['"]/m);
  assert.doesNotMatch(panelSource, /^\s*import\s+\{\s*renderMermaid\s*\}\s+from\s+['"][./]+(?:canvas|lib\/render)\/mermaid['"]/m);
  assert.doesNotMatch(panelSource, /^\s*import\s+\{\s*renderD2\s*\}\s+from\s+['"]\.\.\/canvas\/d2['"]/m);
  assert.match(panelSource, /import\('\.\.\/\.\.\/lib\/render\/mermaid'\)/);
  assert.match(panelSource, /import\('\.\.\/canvas\/d2'\)/);
  const compiled = compile(panelSource, { filename: panelUrl.pathname, generate: 'client' });
  assert.ok(compiled.js.code.length > 0);

  const fnStart = panelSource.indexOf('async function openInCanvas');
  assert.notEqual(fnStart, -1, 'openInCanvas is async');
  const fnSlice = panelSource.slice(fnStart, fnStart + 320);
  const importIdx = fnSlice.indexOf("await import('../../lib/stores/canvas.svelte')");
  const pendingIdx = fnSlice.indexOf('canvas.pendingOpenId = sceneId');
  const routeIdx = fnSlice.indexOf("router.go('canvas')");
  const toastIdx = fnSlice.indexOf("toastError('Couldn’t open Canvas', e)");
  assert.ok(importIdx > 0 && pendingIdx > importIdx && routeIdx > pendingIdx && toastIdx > routeIdx);
});
