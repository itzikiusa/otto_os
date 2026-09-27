import { terminalReply } from '../src/lib/components/terminalInput.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseRoomInvite, roomSocketAddress, reconcilePin, normalizedPoint } from '../src/modules/rooms/room-state.ts';

test('invites require an explicit secure host and exactly one fragment capability', () => {
  assert.deepEqual(parseRoomInvite('https://host.test/#/room/room-1/secret_1234567890'), { origin: 'https://host.test', roomId: 'room-1', invite: 'secret_1234567890', url: 'https://host.test/#/room/room-1/secret_1234567890' });
  assert.equal(parseRoomInvite('http://127.0.0.1:7700/#/room/r/token_1234567890').origin, 'http://127.0.0.1:7700');
  for (const url of ['http://host.test/#/room/r/token_1234567890', 'https://user:pass@host.test/#/room/r/token_1234567890', 'https://host.test/?token=owner#/room/r/token_1234567890', 'https://host.test/#/room/r/token_1234567890/extra', 'javascript:alert(1)']) assert.throws(() => parseRoomInvite(url));
});
test('room sockets carry no query credential and use the room endpoint', () => {
  assert.equal(roomSocketAddress('https://host.test', 'r', true), 'wss://host.test/ws/rooms/r/terminal');
});
test('pin is local and a stopped source falls back to the terminal', () => {
  assert.equal(reconcilePin('a', ['a', 'b']), 'a');
  assert.equal(reconcilePin('a', ['b']), null);
  assert.equal(reconcilePin(null, ['b']), null);
});
test('annotation positions account for letterboxing and reject the margins', () => {
  assert.deepEqual(normalizedPoint(150, 100, {left: 0, top: 0, width: 300, height: 200}, 1600, 900), {x: .5, y: .5});
  assert.equal(normalizedPoint(150, 1, {left: 0, top: 0, width: 300, height: 200}, 1600, 900), null);
});
test('only recognized automatic terminal reports are passive input', () => {
  assert.equal(terminalReply('\x1b[12;24R'), true);
  assert.equal(terminalReply('\x1b[?1;2c'), true);
  assert.equal(terminalReply('\x1b]10;rgb:ffff/ffff/ffff\x1b\\'), true);
  for (const text of ['ls\r', '\x1b[A', '\x1b[200~paste\x1b[201~', '\r']) assert.equal(terminalReply(text), false);
});

test('late annotation events cannot restore cleared or revoked marks', async () => {
  const {applyRoomEvent} = await import('../src/modules/rooms/room-state.ts');
  const room: import('../src/lib/api/room-types').RoomSnapshot = {room_id: 'r', member_id: 'm', admission: 'admitted', presentations: [{id: 's', member_id: 'm', generation: 3, clear_epoch: 2, width: 100, height: 100, title: 'Screen'}], annotation_grants: [{source_id: 's', member_id: 'm', allowed: true, requested: false, blocked: false, epoch: 4}]};
  const event: import('../src/lib/api/room-types').RoomEvent = {type: 'annotation', annotation: {id: 'a', source_id: 's', source_generation: 3, clear_epoch: 2, grant_epoch: 4, member_id: 'm', tool: 'pen', points: [{x: .1, y: .2}], expires_at: '2099-01-01'}};
  assert.equal(applyRoomEvent(room, event).annotations?.length, 1);
  assert.equal(applyRoomEvent({...room, annotation_grants: []}, event).annotations, undefined);
  const cleared = applyRoomEvent(room, {type: 'clear_annotations', source_id: 's', clear_epoch: 3});
  assert.equal(applyRoomEvent(cleared, event), cleared);
  assert.equal(applyRoomEvent(room, {...event, annotation: {...event.annotation, grant_epoch: 3}}), room);
});
