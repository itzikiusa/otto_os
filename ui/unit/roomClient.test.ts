import { test } from 'node:test';
import assert from 'node:assert/strict';
import { RoomClient } from '../src/modules/rooms/room-client.ts';
class Socket {
  static OPEN = 1;
  static instances: Socket[] = [];
  url: string; protocols: string[]; sent: string[] = []; closed = false; readyState = 1;
  onopen: (() => void) | null = null;
  onmessage: ((event: {data: string}) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(url: string, protocols: string[]) { this.url = url; this.protocols = protocols; Socket.instances.push(this); }
  send(value: string) { this.sent.push(value); }
  close() { this.closed = true; this.onclose?.(); }
}
test('membership waits for snapshot and fails closed after ten seconds of silence', context => {
  const original = globalThis.WebSocket;
  globalThis.WebSocket = Socket as unknown as typeof WebSocket;
  context.after(() => { globalThis.WebSocket = original; });
  context.mock.timers.enable({apis: ['setInterval', 'setTimeout']});
  let now = 0; context.mock.method(performance, 'now', () => now);
  const states: string[] = [];
  const client = new RoomClient('https://host.test', {room_id: 'room', member_id: 'guest', token: 'private'}, () => {}, state => states.push(state));
  client.connect();
  const socket = Socket.instances.at(-1)!;
  assert.deepEqual(socket.protocols, ['otto-room', 'private']);
  assert.equal(socket.url, 'wss://host.test/ws/rooms/room');
  socket.onopen!();
  assert.equal(states.at(-1), 'connecting');
  socket.onmessage!({data: JSON.stringify({type: 'snapshot', room: {room_id: 'room', member_id: 'guest', admission: 'pending'}})});
  assert.equal(states.at(-1), 'connected');
  now = 5000; context.mock.timers.tick(5000);
  assert.equal(socket.sent.at(-1), '{"type":"heartbeat"}');
  now = 10000; context.mock.timers.tick(5000);
  assert.equal(states.at(-1), 'disconnected');
  assert.equal(socket.closed, true);
  client.dispose();
  const count = Socket.instances.length; context.mock.timers.tick(30000);
  assert.equal(Socket.instances.length, count);
});


test('silence deadline resets per server frame rather than rounding to the heartbeat tick', context => {
  const original = globalThis.WebSocket;
  globalThis.WebSocket = Socket as unknown as typeof WebSocket;
  context.after(() => { globalThis.WebSocket = original; });
  context.mock.timers.enable({apis: ['setInterval', 'setTimeout']});
  const client = new RoomClient('https://host.test', {room_id: 'room', member_id: 'guest', token: 'private'}, () => {}, () => {});
  client.connect(); const socket = Socket.instances.at(-1)!; socket.onopen!();
  context.mock.timers.tick(1);
  socket.onmessage!({data: '{"type":"heartbeat"}'});
  context.mock.timers.tick(9999); assert.equal(socket.closed, false);
  context.mock.timers.tick(1); assert.equal(socket.closed, true);
  client.dispose();
});
