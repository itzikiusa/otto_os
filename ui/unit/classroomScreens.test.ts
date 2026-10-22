/// <reference lib="dom" />
// ui-performance-5-01: the classroom's live-screen poller must not restart on
// every session status event. `buildSchool` re-allocates every room and kid
// per rebuild (one per status event), so the box keys the poller on the open
// room's id and re-pushes screens only when `roomScreensKey` changes; a
// status-driven early refresh is throttled to one `now()` per POLL_MS.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildSchool } from '../src/modules/home/school/model.ts';
import { POLL_MS, roomScreensKey, throttledNow } from '../src/modules/home/school/screens.ts';
import type { Session } from '../src/lib/api/types';

const T0 = Date.now();
const sess = (id: string, p: Partial<Session> = {}): Session => ({
  id, workspace_id: 'w1', kind: 'agent', provider: 'claude', title: `Session ${id}`, status: 'working', cwd: '/tmp',
  created_at: new Date(T0 - 3_600_000).toISOString(), last_active_at: new Date(T0 - 1000).toISOString(),
  archived: false, meta: {}, ...p,
} as Session);
const WS = [{ id: 'w1', name: 'Otto', my_role: 'admin' }];
const school = (status: Record<string, string>) =>
  buildSchool({ workspaces: WS, currentId: 'w1', sessions: [sess('a'), sess('b')], statusOf: (id) => status[id] });

test('a rebuild per status event allocates a new room, but the screens key only moves when a monitor changes', () => {
  const before = school({ a: 'working', b: 'working' });
  // Same visible state: e.g. the event was about a session in another room.
  const after = school({ a: 'working', b: 'working', other: 'idle' });
  assert.notEqual(before.rooms[0], after.rooms[0], 'buildSchool re-allocates: keying on the room object re-ran the poller');
  assert.equal(before.rooms[0].id, after.rooms[0].id);
  assert.equal(roomScreensKey(before.rooms[0]), roomScreensKey(after.rooms[0]));
  assert.equal(roomScreensKey(null), '');
  const waiting = school({ a: 'working', b: 'waiting_input' });
  const bPose = (s: ReturnType<typeof school>) => s.kids.find((k) => k.id === 'b')!.pose;
  if (bPose(waiting) !== bPose(before)) assert.notEqual(roomScreensKey(waiting.rooms[0]), roomScreensKey(before.rooms[0]));
});

test('a status event does not recreate the poller: the poll effect keys on the open room id', () => {
  const src = readFileSync(join(import.meta.dirname, '..', 'src/modules/home/boxes/ClassroomsBox.svelte'), 'utf8');
  const start = src.indexOf('const p = pollScreens(');
  assert.ok(start > 0);
  const effect = src.slice(src.lastIndexOf('$effect(', start), start);
  assert.match(effect, /openRoomId/);
  assert.doesNotMatch(effect, /openRoomModel/, 'the room object changes on every status event');
  // The screen-push effect depends on the key string, not the room object.
  assert.match(src, /const openRoomScreens = \$derived\(roomScreensKey\(openRoomModel\)\)/);
  assert.match(src, /void openRoomScreens;[\s\S]{0,200}untrack\(/);
});

test('status-driven refreshes call now() at most once per POLL_MS', () => {
  let t = 0, calls = 0;
  const nudge = throttledNow({ now: () => { calls++; } }, POLL_MS, () => t);
  for (let i = 0; i < 5; i++) { nudge(); t += 100; }
  assert.equal(calls, 1);
  t = POLL_MS;
  nudge();
  assert.equal(calls, 2);
  nudge();
  assert.equal(calls, 2);
});

test('the Classrooms box loads through freshness() like the other Home boxes', () => {
  const src = readFileSync(join(import.meta.dirname, '..', 'src/modules/home/boxes/ClassroomsBox.svelte'), 'utf8');
  assert.match(src, /const fresh = freshness\(load\)/);
  assert.match(src, /livePoll\(fresh\.run, 30_000, [^\n]*immediate: fresh\.start\(/);
  assert.doesNotMatch(src, /immediate: true/);
  // The grid copy of a zoomed box releases its scene (one WebGL context).
  assert.match(src, /mode !== '3d' \|\| zoomedAway\) return;/);
});
