// Canvas saves are guarded by the `updated_at` this tab last saw: a scene
// changed elsewhere (the `canvas_update_scene` MCP tool, another tab or
// person) answers 409 instead of being overwritten, and the person's board is
// kept until they pick "Reload theirs" or "Keep mine as a copy".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

type Call = { method: string; path: string; body?: Record<string, unknown> };

function store(put: (body: Record<string, unknown>) => unknown) {
  const calls: Call[] = [];
  let postFails = false;
  let stored = { id: 'scene-a', title: 'Board', doc_json: '{"format":"excalidraw","source":"server"}', updated_at: 'T1', session_id: null };
  const api = {
    get: async (path: string) => {
      calls.push({ method: 'GET', path });
      return path.includes('scene-copy') ? { ...stored, id: 'scene-copy', updated_at: 'C1' } : stored;
    },
    put: async (path: string, body: Record<string, unknown>) => { calls.push({ method: 'PUT', path, body }); return put(body); },
    post: async (path: string, body: Record<string, unknown>) => {
      calls.push({ method: 'POST', path, body });
      if (postFails) throw new Error('offline');
      return { id: 'scene-copy', title: body.title, doc_json: JSON.stringify(body.doc), updated_at: 'C1', session_id: null };
    },
  };
  const mod = loadSource(new URL('../src/lib/stores/canvas.svelte.ts', import.meta.url), {
    '../api/client': { api, getToken: () => 'tok' },
    './workspace.svelte': { ws: { currentId: 'w1' } }, '../labels': { NO_WORKSPACE: 'Select a workspace first' },
    '../providers': { defaultAgentProvider: () => 'claude' },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../canvas/scene': { assistToNodes: () => [], emptyScene: () => ({}), parseScene: (x: unknown) => x },
  });
  return { c: mod.canvas, calls, setStored: (row: typeof stored) => { stored = row; }, failPost: () => { postFails = true; } };
}

const puts = (calls: Call[]) => calls.filter((c) => c.method === 'PUT').map((c) => JSON.parse(JSON.stringify(c.body)));

test('a save sends the stamp it last saw and adopts the one the server answers', async () => {
  let n = 1;
  const { c, calls } = store(() => ({ updated_at: `T${++n}` }));
  await c.open('scene-a');
  await c.persistDoc('scene-a', { source: 'one' });
  await c.persistDoc('scene-a', { source: 'two' });
  const sent = puts(calls);
  assert.equal(sent[0].expect_updated_at, 'T1');
  assert.equal(sent[1].expect_updated_at, 'T2');
});

test('an agent push forgets the stamp: the next save is unguarded and re-stamps', async () => {
  const { c, calls } = store(() => ({ updated_at: 'T9' }));
  await c.open('scene-a');
  c.ingestPush(1, 'scene-a', { format: 'excalidraw', source: 'agent' });
  await c.persistDoc('scene-a', { source: 'mine' });
  await c.persistDoc('scene-a', { source: 'mine2' });
  const sent = puts(calls);
  assert.equal('expect_updated_at' in sent[0], false);
  assert.equal(sent[1].expect_updated_at, 'T9');
});

test('a 409 keeps the draft, raises the conflict and no retry banner', async () => {
  const { c } = store(() => { throw Object.assign(new Error('changed since the edit began'), { status: 409 }); });
  await c.open('scene-a');
  const mine = { source: 'mine' };
  await assert.rejects(c.persistDoc('scene-a', mine));
  assert.equal(c.conflict?.id, 'scene-a');
  assert.equal(c.conflict?.doc, mine);
  assert.deepEqual(JSON.parse(JSON.stringify(c.docSaveErrors)), {});
  assert.equal(c.dirty, true, 'the unsaved board is still marked unsaved');
});

test('keep mine as a copy saves the board as a new scene before showing theirs', async () => {
  const { c, calls, setStored } = store(() => { throw Object.assign(new Error('conflict'), { status: 409 }); });
  c.scenes = [{ id: 'scene-a', workspace_id: 'w7', title: 'Board', updated_at: 'T1' }];
  await c.open('scene-a');
  const mine = { format: 'excalidraw', source: 'mine' };
  await assert.rejects(c.persistDoc('scene-a', mine));
  setStored({ id: 'scene-a', title: 'Board', doc_json: '{"format":"excalidraw","source":"theirs"}', updated_at: 'T5', session_id: null });
  const copy = await c.keepMineAsCopy();
  const post = calls.find((x) => x.method === 'POST')!;
  assert.equal(post.path, '/workspaces/w7/canvas/scenes');
  assert.equal(JSON.parse(JSON.stringify(post.body)).doc.source, 'mine');
  assert.equal(JSON.parse(JSON.stringify(post.body)).title, 'Board (my copy)');
  assert.equal(copy?.id, 'scene-copy');
  assert.equal(c.conflict, null);
  assert.equal(c.currentId, 'scene-copy');
});

test('reload theirs drops the draft and shows the stored board', async () => {
  const { c } = store(() => { throw Object.assign(new Error('conflict'), { status: 409 }); });
  await c.open('scene-a');
  await assert.rejects(c.persistDoc('scene-a', { source: 'mine' }));
  await c.reloadTheirs();
  assert.equal(c.conflict, null);
  assert.equal(c.dirty, false);
  assert.equal(c.source, 'server');
});

test('a failed copy keeps the conflict (and so the draft) for another try', async () => {
  const { c, failPost } = store(() => { throw Object.assign(new Error('conflict'), { status: 409 }); });
  await c.open('scene-a');
  const mine = { source: 'mine' };
  await assert.rejects(c.persistDoc('scene-a', mine));
  failPost();
  await assert.rejects(c.keepMineAsCopy());
  assert.equal(c.conflict?.doc, mine);
  assert.equal(c.currentId, 'scene-a');
});
