/// <reference lib="dom" />
// arch-5-01 (UI half) + error-handling-5-09: both relays (RoomClient,
// GameClient) wrap one RelaySocket. When its automatic reconnects run out it
// reports `gave-up` (the game page then offers Reconnect) instead of a
// forever "Reconnecting…", and an explicit reconnect, the network coming back
// or the page becoming visible again start a fresh attempt budget.
import { test, type TestContext } from 'node:test';
import assert from 'node:assert/strict';
import { RelaySocket, type RelayStatus } from '../src/lib/relaySocket.ts';
import { GameClient, type GameConnection } from '../src/modules/rooms/games/client.ts';

class Socket {
  static OPEN = 1;
  static instances: Socket[] = [];
  url: string; protocols: string[]; sent: string[] = []; closed = false; readyState = 1; bufferedAmount = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(url: string, protocols: string[]) { this.url = url; this.protocols = protocols; Socket.instances.push(this); }
  send(value: string) { this.sent.push(value); }
  close() { this.closed = true; this.onclose?.(); }
}
function install(context: TestContext) {
  const original = globalThis.WebSocket;
  globalThis.WebSocket = Socket as unknown as typeof WebSocket;
  context.after(() => { globalThis.WebSocket = original; });
  context.mock.timers.enable({ apis: ['setInterval', 'setTimeout'] });
}
const relay = (states: RelayStatus[], maxAttempts = 2) => new RelaySocket<{ type: string }>({
  url: 'wss://host.test/ws/x', protocol: 'otto-x', token: 't'.repeat(43), maxBytes: 1024, silenceMs: 10_000,
  heartbeatMs: 5000, heartbeat: '{"type":"ping"}', maxAttempts, backoff: () => 1000,
  parse: (d) => { try { return JSON.parse(d); } catch { return null; } },
  connectedBy: (e) => e.type === 'state', endedBy: (e) => e.type === 'closed', onEvent: () => {}, onStatus: (s) => states.push(s),
});

test('the token rides only in the subprotocol pair', (context) => {
  install(context);
  const r = relay([]);
  r.connect();
  const socket = Socket.instances.at(-1)!;
  assert.deepEqual(socket.protocols, ['otto-x', 't'.repeat(43)]);
  assert.equal(socket.url, 'wss://host.test/ws/x');
  r.dispose();
});

test('after the reconnect budget the status is gave-up, and reconnect() starts a fresh budget', (context) => {
  install(context);
  const states: RelayStatus[] = [];
  const r = relay(states, 2);
  context.after(() => r.dispose());
  r.connect();
  Socket.instances.at(-1)!.close();
  assert.equal(states.at(-1), 'disconnected');
  context.mock.timers.tick(1000);
  Socket.instances.at(-1)!.close();
  context.mock.timers.tick(1000);
  const before = Socket.instances.length;
  Socket.instances.at(-1)!.close();
  assert.equal(states.at(-1), 'gave-up');
  context.mock.timers.tick(60_000);
  assert.equal(Socket.instances.length, before, 'no more automatic attempts after giving up');
  r.reconnect();
  assert.equal(Socket.instances.length, before + 1);
  assert.equal(states.at(-1), 'connecting');
  Socket.instances.at(-1)!.onmessage!({ data: '{"type":"state"}' });
  assert.equal(states.at(-1), 'connected');
});

test('an ended relay never reconnects, and invalid frames do not count as liveness', (context) => {
  install(context);
  const states: RelayStatus[] = [];
  const r = relay(states);
  r.connect();
  const socket = Socket.instances.at(-1)!;
  socket.onopen!();
  socket.onmessage!({ data: 'not json' });
  socket.onmessage!({ data: '{"type":"closed"}' });
  assert.equal(states.at(-1), 'ended');
  const count = Socket.instances.length;
  r.reconnect();
  context.mock.timers.tick(10_000);
  assert.equal(Socket.instances.length, count);
});

test('GameClient exposes gave-up and a Reconnect that resets the attempts', (context) => {
  install(context);
  const states: GameConnection[] = [];
  const client = new GameClient('https://host.test', { room_id: 'g', member_id: 'm', role: 'guest', token: 'k'.repeat(43), config: { game: 'kart', map: 'meadow' } }, () => {}, (s) => states.push(s));
  context.after(() => client.dispose());
  client.connect();
  assert.deepEqual(Socket.instances.at(-1)!.protocols, ['otto-game', 'k'.repeat(43)]);
  for (let i = 0; i < 5; i++) { Socket.instances.at(-1)!.close(); context.mock.timers.tick(5000); }
  Socket.instances.at(-1)!.close();
  assert.equal(states.at(-1), 'gave-up');
  const count = Socket.instances.length;
  client.reconnect();
  assert.equal(Socket.instances.length, count + 1);
  Socket.instances.at(-1)!.onmessage!({ data: JSON.stringify({ type: 'closed', reason: 'Host left' }) });
  assert.equal(states.at(-1), 'closed');
});
