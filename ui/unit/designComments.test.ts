import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as pins from '../src/modules/design-hall/comments/pins.ts';
import { deferred, loadSource } from './sourceHarness.ts';

// Design Hall review comments: the pin maths (fractions of the stage's
// content box), threading / ordering / which pins draw, and the shared
// store's load guard, writes and live-event follow.

type C = Record<string, any>;
let t = 0;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const comment = (id: string, over: C = {}): any => ({
  id, artifact_id: 'A', workspace_id: 'w', version_id: 'v2', version_seq: 2, parent_id: null,
  author_kind: 'user', author_id: 'u1', author_name: 'Ana', session_id: null, body: id, pin: null,
  resolved: false, resolved_by: null, resolved_by_name: null, resolved_at: null, edited_at: null,
  created_at: `2026-11-13T10:00:${String(t++).padStart(2, '0')}Z`, updated_at: '2026-11-13T10:00:00Z', ...over,
});

test('pinAt: a pointer on the content box → clamped fractions; an empty box → null', () => {
  const box = { left: 100, top: 50, width: 400, height: 200 };
  assert.deepEqual(pins.pinAt(300, 100, box), { x: 0.5, y: 0.25 });
  assert.deepEqual(pins.pinAt(50, 400, box), { x: 0, y: 1 }, 'outside the box clamps onto its edge');
  assert.deepEqual(pins.pinAt(100 + 400 / 3, 50, box), { x: 0.3333, y: 0 });
  assert.equal(pins.pinAt(10, 10, { left: 0, top: 0, width: 0, height: 100 }), null);
});

test('a pin keeps its spot when the stage resizes or the device frame changes', () => {
  const pin = pins.pinAt(250, 150, { left: 0, top: 0, width: 1000, height: 600 })!;
  // Same design, re-laid-out into a narrower pane: the same relative spot.
  assert.deepEqual(pins.pinPoint(pin, { width: 500, height: 300 }), { x: 125, y: 75 });
  assert.deepEqual(pins.pinPoint(pin, { width: 390, height: 844 }), { x: 97.5, y: 211 });
  assert.deepEqual(pins.pinPoint({ x: 2, y: -1 }, { width: 100, height: 100 }), { x: 100, y: 0 });
});

test('nudge moves a draft pin with the keyboard and never off the stage', () => {
  assert.deepEqual(pins.nudge({ x: 0.5, y: 0.5 }, 1, -1), { x: 0.51, y: 0.49 });
  assert.deepEqual(pins.nudge({ x: 0.99, y: 0.01, frame: 'f' }, 1, -1, 0.05), { x: 1, y: 0, frame: 'f' });
  assert.equal(pins.pinWords({ x: 0.424, y: 0.18 }), '42% across, 18% down');
});

test('threadsOf: replies under their root, numbered by creation, orphans dropped', () => {
  const a = comment('a');
  const b = comment('b', { resolved: true });
  const r1 = comment('r1', { parent_id: 'a' });
  const orphan = comment('o', { parent_id: 'gone' });
  const threads = pins.threadsOf([r1, b, orphan, a]);
  assert.deepEqual(threads.map((x) => [x.root.id, x.n, x.replies.map((r) => r.id)]), [['a', 1, ['r1']], ['b', 2, []]]);
  const { open, resolved } = pins.splitThreads(threads);
  assert.deepEqual([open.map((x) => x.root.id), resolved.map((x) => x.root.id)], [['a'], ['b']]);
  assert.equal(pins.openCount(threads), 1);
  assert.deepEqual(pins.threadComments(threads[0]).map((c) => c.id), ['a', 'r1']);
});

test('pinsFor: open pinned threads draw; another version is muted; resolved and unpinned do not', () => {
  const threads = pins.threadsOf([
    comment('here', { pin: { x: 0.1, y: 0.1 } }),
    comment('older', { pin: { x: 0.2, y: 0.2 }, version_id: 'v1', version_seq: 1 }),
    comment('done', { pin: { x: 0.3, y: 0.3 }, resolved: true }),
    comment('whole'),
  ]);
  assert.deepEqual(pins.pinsFor(threads, 'v2').map((d) => [d.thread.root.id, d.muted]), [['here', false], ['older', true]]);
  assert.deepEqual(pins.pinsFor(threads, 'v1').map((d) => [d.thread.root.id, d.muted]), [['here', true], ['older', false]]);
});

test('versionChip names the version a comment was made on, unless it is the one shown', () => {
  assert.equal(pins.versionChip({ version_id: 'v1', version_seq: 3 }, 'v9'), 'made on v3');
  assert.equal(pins.versionChip({ version_id: 'v9', version_seq: 9 }, 'v9'), null);
  assert.equal(pins.versionChip({ version_id: null, version_seq: null }, 'v9'), null);
});

test('isTypingTarget keeps the C shortcut away from fields and editors', () => {
  assert.equal(pins.isTypingTarget({ tagName: 'TEXTAREA' }), true);
  assert.equal(pins.isTypingTarget({ tagName: 'DIV', isContentEditable: true }), true);
  assert.equal(pins.isTypingTarget({ tagName: 'DIV', closest: (s: string) => (s.includes('.cm-editor') ? {} : null) }), true);
  assert.equal(pins.isTypingTarget({ tagName: 'BUTTON', closest: () => null }), false);
  assert.equal(pins.isTypingTarget(null), false);
});

/** Rows from the store's vm realm, as plain values. */
const ids = (xs: C[]): string[] => JSON.parse(JSON.stringify(xs.map((c) => c.id)));

function loadStore(api: Record<string, unknown>) {
  return loadSource(new URL('../src/modules/design-hall/comments/commentsStore.svelte.ts', import.meta.url), {
    '../../../lib/api/design': api,
    './pins': pins,
  });
}

test('store: a stale list never overwrites a newer one, and a write survives an in-flight list', async () => {
  const first = deferred<C[]>();
  const second = deferred<C[]>();
  const lists = [first, second];
  const { CommentsStore } = loadStore({
    listComments: () => lists.shift()!.promise,
    createComment: async (_: string, body: C) => comment('new', { body: body.body, pin: body.pin ?? null }),
  });
  const s = new CommentsStore('A');
  const p1 = s.load();
  const p2 = s.load();
  second.resolve([comment('fresh')]);
  await p2;
  first.resolve([comment('stale')]);
  await p1;
  assert.deepEqual(ids(s.items), ['fresh']);
  assert.equal(s.loaded, true);

  const third = deferred<C[]>();
  lists.push(third);
  const p3 = s.load();
  const made = await s.create('Bigger CTA', { pin: { x: 0.4, y: 0.2 }, versionId: 'v2' });
  third.resolve([comment('fresh')]);
  await p3;
  assert.equal(made.pin.x, 0.4);
  assert.deepEqual(ids(s.items), ['fresh', 'new'], 'the older list did not drop the new row');
});

test('store: create sends a reply without version or pin; remove takes the replies', async () => {
  const sent: C[] = [];
  const { CommentsStore } = loadStore({
    listComments: async () => [comment('a'), comment('r', { parent_id: 'a' }), comment('b')],
    createComment: async (_: string, body: C) => { sent.push(JSON.parse(JSON.stringify(body))); return comment('r2', { parent_id: 'a' }); },
    deleteComment: async () => undefined,
  });
  const s = new CommentsStore('A');
  await s.load();
  await s.create('Agreed', { parentId: 'a', versionId: 'v1', pin: { x: 0.1, y: 0.1 } });
  assert.deepEqual(sent, [{ body: 'Agreed', parent_id: 'a' }]);
  s.selectedId = 'a';
  await s.remove('a');
  assert.deepEqual(ids(s.items), ['b']);
  assert.equal(s.selectedId, null);
});

test('store: a failed reload keeps the last good rows and reports the error', async () => {
  let fail = false;
  const { CommentsStore } = loadStore({
    listComments: async () => {
      if (fail) throw new Error('offline');
      return [comment('a')];
    },
  });
  const s = new CommentsStore('A');
  await s.load();
  fail = true;
  await s.load();
  assert.deepEqual(ids(s.items), ['a']);
  assert.ok(s.error);
  assert.equal(s.loading, false);
});

test('store.follow: loads once, then reloads only for this design’s comment events or a reconnect', async () => {
  let calls = 0;
  const { CommentsStore } = loadStore({ listComments: async () => { calls++; return []; } });
  const s = new CommentsStore('A');
  const log: C[] = [];
  const bus = { seq: 0, resyncTick: 0, since: (n: number) => log.slice(n) };
  s.follow(bus);
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls, 1, 'the first follow loads');
  const push = (ev: C) => { log.push(ev); bus.seq = log.length; s.follow(bus); };
  push({ type: 'design_artifact_updated', artifact_id: 'A', change: 'content' });
  push({ type: 'design_artifact_updated', artifact_id: 'B', change: 'comments' });
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(calls, 1, 'other changes and other designs do not reload');
  push({ type: 'design_artifact_updated', artifact_id: 'A', change: 'comments' });
  push({ type: 'design_artifact_updated', artifact_id: 'A', change: 'comments' });
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(calls, 2, 'a burst coalesces into one reload');
  bus.resyncTick = 1;
  s.follow(bus);
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(calls, 3, 'a reconnect reloads');
  s.dispose();
});

test('commentsFor: one shared store per design, a few recent ones cached', () => {
  const { commentsFor } = loadStore({});
  const a = commentsFor('A');
  assert.equal(commentsFor('A'), a);
  for (const id of ['B', 'C', 'D', 'E', 'F', 'G']) commentsFor(id);
  assert.notEqual(commentsFor('A'), a, 'the oldest store was evicted');
});
