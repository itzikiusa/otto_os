import { test } from 'node:test';
import assert from 'node:assert/strict';
import { hostRequests, nextExpiry } from '../src/modules/rooms/room-state.ts';
import { joinRoom, JOIN_UNREACHABLE } from '../src/modules/rooms/room-client.ts';
import type { RoomMember, RoomSnapshot } from '../src/lib/api/room-types.ts';

function member(id: string, extra: Partial<RoomMember> = {}): RoomMember {
  return {
    id, name: id, role: 'viewer', admission: 'admitted', connected: true, generation: 1,
    control_requested: false, audio_joined: false, muted: true, room_muted: false,
    presenter_requested: false, presenter_allowed: false, ...extra,
  };
}

test('annotation expiry arms one timer for the earliest live mark, none when nothing is left', () => {
  assert.equal(nextExpiry([], 100), null);
  assert.equal(nextExpiry([50, 90], 100), null, 'already-expired marks never re-arm a timer');
  assert.equal(nextExpiry([400, 150, 90, Number.NaN, 300], 100), 150);
});

test('the host gets a summary of knocks and requests; guests get nothing', () => {
  const room: RoomSnapshot = {
    room_id: 'r', member_id: 'host', host_member_id: 'host', admission: 'admitted',
    members: [
      member('host', { role: 'host', control_requested: true }),
      member('Maya', { admission: 'pending' }),
      member('Noa', { role: 'editor', control_requested: true }),
      member('Lee', { presenter_requested: true }),
      member('Ari', { presenter_requested: true, presenter_allowed: true }),
    ],
  };
  const host = hostRequests(room);
  assert.deepEqual([host.waiting, host.control, host.present, host.total], [1, 1, 1, 3]);
  assert.equal(host.summary, '1 waiting · 1 control request · 1 presentation request');
  assert.deepEqual(host.items.map(i => i.text), ['Maya is waiting for admission.', 'Noa asks for terminal control.', 'Lee asks to present.']);
  // A guest (even an admitted editor) never sees the host's decision queue.
  const guest = hostRequests({ ...room, member_id: 'Noa' });
  assert.equal(guest.total, 0);
  assert.equal(guest.summary, '');
  assert.equal(hostRequests(null).total, 0);
});

test('a join that cannot reach the host says so instead of the raw "Load failed"', async (context) => {
  context.mock.method(globalThis, 'fetch', async () => { throw new TypeError('Load failed'); });
  await assert.rejects(joinRoom('https://host.test', 'room', 'invite_1234567890', 'Maya'), { message: JOIN_UNREACHABLE });
});

test('an HTTP refusal still carries the host’s own message', async (context) => {
  context.mock.method(globalThis, 'fetch', async () => new Response(JSON.stringify({ message: 'This invitation expired.' }), { status: 410 }));
  await assert.rejects(joinRoom('https://host.test', 'room', 'invite_1234567890', 'Maya'), { message: 'This invitation expired.' });
});
