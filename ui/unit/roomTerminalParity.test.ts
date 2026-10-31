// Session-room terminal parity: the Navigator keeps Agents' long session list
// from burying its section siblings (Rooms, History…), the room relay's input
// cap is honoured by chunking, and the relay's `ended` frame is decoded so the
// terminal stops retrying a stream that can never come back.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { agentsLast } from '../src/lib/sidebar.ts';
import { inputFrames, ROOM_INPUT_CHUNK, endedLine } from '../src/lib/components/termRoom.ts';
import { readFileSync } from 'node:fs';
import { base64ToBytes } from '../src/lib/b64.ts';
import { parseRoomInvite } from '../src/modules/rooms/room-state.ts';

const mod = (id: string) => ({ id, icon: 'home' as const, label: id, group: 'work' as const });

test('agentsLast moves Agents after its section siblings and leaves other sections alone', () => {
  const work = ['home', 'agents', 'rooms'].map(mod);
  assert.deepEqual(agentsLast(work).map((m) => m.id), ['home', 'rooms', 'agents']);
  const noAgents = ['git', 'vault'].map(mod);
  assert.equal(agentsLast(noAgents), noAgents);
  const already = ['rooms', 'agents'].map(mod);
  assert.equal(agentsLast(already), already);
});

test('room input over the relay cap is split into ordered chunks that reassemble exactly', () => {
  const text = 'é'.repeat(ROOM_INPUT_CHUNK); // 2 bytes each → two+ chunks
  const frames = inputFrames(text, true, true);
  assert.ok(frames.length > 1);
  const bytes = frames.flatMap((f) => [...base64ToBytes(f.data)]);
  assert.ok(frames.every((f) => base64ToBytes(f.data).length <= ROOM_INPUT_CHUNK && f.user));
  assert.equal(new TextDecoder().decode(new Uint8Array(bytes)), text);
  // The owner's socket (no cap) and small room input stay one frame.
  assert.equal(inputFrames(text, true, false).length, 1);
  assert.equal(inputFrames('ls\r', false, true).length, 1);
});

test('the relay ended frame is decoded', () => {
  // termProtocol.ts uses extensionless imports, so pin its decoder by source.
  const proto = readFileSync(new URL('../src/lib/components/termProtocol.ts', import.meta.url), 'utf8');
  assert.match(proto, /case 'ended':\s*\n\s*return \{ type: 'ended', reason:/);
  assert.match(endedLine('Terminal sharing ended'), /\[otto\] Terminal sharing ended/);
});

test('LAN HTTPS invites (what a network-listener host now issues) parse', () => {
  const inv = parseRoomInvite('https://192.168.60.96:7701/#/room/room_1/0123456789abcdef0123');
  assert.equal(inv.origin, 'https://192.168.60.96:7701');
  assert.equal(inv.roomId, 'room_1');
});

test('invites pinning the host certificate parse; malformed pins do not', () => {
  const pin = 'ab'.repeat(32);
  const inv = parseRoomInvite(`https://192.168.60.96:7701/#/room/room_1/0123456789abcdef0123/${pin}`);
  assert.equal(inv.roomId, 'room_1');
  assert.equal(inv.invite, '0123456789abcdef0123');
  assert.throws(() => parseRoomInvite('https://192.168.60.96:7701/#/room/room_1/0123456789abcdef0123/xyz'));
});
