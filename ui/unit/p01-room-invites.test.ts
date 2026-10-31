// architecture-1-13: room invite capture lives in lib/rooms/invites.ts,
// so the router no longer imports modules/rooms; the rooms module reads the
// same window-memory store.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { captureRoomInvite, roomInvite } from '../src/lib/rooms/invites.ts';
import { forgetRoom, rememberRoom, roomInvite as moduleRoomInvite } from '../src/modules/rooms/room-access.ts';

test('room invites are dropped once the room is remembered or forgotten', () => {
  captureRoomInvite('r1', 'inv-1');
  assert.equal(moduleRoomInvite('r1'), 'inv-1');
  rememberRoom('https://host', { room_id: 'r1' } as Parameters<typeof rememberRoom>[1]);
  assert.equal(roomInvite('r1'), undefined);
  captureRoomInvite('r2', 'inv-2');
  forgetRoom('r2');
  assert.equal(roomInvite('r2'), undefined);
});

test('lib/router imports no modules/ code', () => {
  const src = readFileSync(new URL('../src/lib/router.svelte.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /from '\.\.\/modules\//);
});
