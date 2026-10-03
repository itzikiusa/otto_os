// Agent rooms live feed (perf R2/R3): room events carry the whole message, so
// an open Rooms view appends with no GET; with the view closed (or for a room
// never opened) only the activity line moves; a gap re-reads the tail.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

type Msg = { id: string; room_id: string; author_kind: 'agent' | 'user'; author_id: string; text: string; created_at: string };

function setup() {
  const calls: string[] = [];
  let tail: Msg[] = [];
  const api = {
    messagesBefore: async (roomId: string) => { calls.push(`before:${roomId}`); return tail; },
    messages: async (roomId: string, after?: string) => { calls.push(`after:${roomId}:${after}`); return []; },
    postMessage: async (roomId: string, text: string): Promise<Msg> => ({
      id: 'p1', room_id: roomId, author_kind: 'user', author_id: 'u', text, created_at: 't9',
    }),
  };
  const mod = loadSource(new URL('../src/lib/stores/personalAgents.svelte.ts', import.meta.url), {
    '../api/personalAgents': { personalAgentsApi: api },
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../lazyModule': { announceModule: () => {} },
  });
  const store = mod.personalAgents;
  store.rooms = [{ room: { id: 'r1' }, members: [], message_count: 1, last_message_at: null }];
  return { store, calls, setTail: (m: Msg[]) => { tail = m; } };
}

const msg = (id: string, room = 'r1'): Msg => ({ id, room_id: room, author_kind: 'agent', author_id: 'a', text: id, created_at: `t-${id}` });
const ev = (m: Msg) => ({ type: 'agent_room_message', workspace_id: 'w', room_id: m.room_id, message_id: m.id, author_kind: m.author_kind, author_id: m.author_id, text: m.text, created_at: m.created_at });

test('closed Rooms view: an event bumps activity and fetches nothing', () => {
  const { store, calls } = setup();
  store.applyRoomEvent(ev(msg('m2')));
  assert.equal(store.rooms[0].message_count, 2);
  assert.equal(store.rooms[0].last_message_at, 't-m2');
  assert.deepEqual(calls, []);
});

test('open view appends the event message with no GET, deduped by id', async () => {
  const { store, calls, setTail } = setup();
  const release = store.watchRooms();
  setTail([msg('m1')]);
  store.setActiveRoom('r1');
  await store.loadMessages('r1');
  calls.length = 0;
  store.applyRoomEvent(ev(msg('m2')));
  store.applyRoomEvent(ev(msg('m2')));
  assert.deepEqual(store.messagesByRoom.r1.map((m: Msg) => m.id), ['m1', 'm2']);
  // A room that was never opened is not loaded by its events.
  store.applyRoomEvent(ev(msg('x1', 'r2')));
  assert.equal('r2' in store.messagesByRoom, false);
  // postMessage appends the returned row; its WS echo dedupes.
  await store.postMessage('r1', 'hi');
  store.applyRoomEvent({ ...ev(msg('p1')), author_kind: 'user' });
  assert.deepEqual(store.messagesByRoom.r1.map((m: Msg) => m.id), ['m1', 'm2', 'p1']);
  assert.deepEqual(calls, []);
  release();
});

test('unmount evicts all but the selected room; a resync re-reads the tail', async () => {
  const { store, calls, setTail } = setup();
  const release = store.watchRooms();
  setTail([msg('m1')]);
  await store.loadMessages('r2');
  await store.loadMessages('r1');
  store.setActiveRoom('r1');
  release();
  assert.deepEqual(Object.keys(store.messagesByRoom), ['r1']);
  // Remount: the kept feed missed events while closed → tail, not after-cursor.
  const again = store.watchRooms();
  calls.length = 0;
  setTail([msg('m1'), msg('m2'), msg('m3')]);
  await store.loadMessages('r1');
  assert.deepEqual(calls, ['before:r1']);
  assert.equal(store.messagesByRoom.r1.length, 3);
  // A WS gap: the shown room re-reads its tail now.
  calls.length = 0;
  store.resyncRooms();
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(calls, ['before:r1']);
  again();
});
