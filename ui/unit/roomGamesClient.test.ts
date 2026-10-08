import {test, type TestContext} from 'node:test';
import assert from 'node:assert/strict';
import {parseGameInvite, gameSocketUrl, decodeInput, decodeSnapshot, GameClient} from '../src/modules/rooms/games/client.ts';
import {createGame} from '../src/modules/rooms/games/simulation.ts';
import type {GameRoomCredential} from '../src/lib/api/game-room-types';
test('game invitations keep credentials in fragment and require secure remote origins', () => {
  const invite = parseGameInvite('https://host.test/#/game-room/abc?invite=abcdefghijklmnopqrstuv');
  assert.equal(invite.roomId, 'abc'); assert.equal(invite.invite, 'abcdefghijklmnopqrstuv');
  assert.throws(() => parseGameInvite('http://host.test/#/game-room/a?invite=abcdefghijklmnopqrstuv'));
  assert.throws(() => parseGameInvite('https://user:pass@host.test/#/game-room/a?invite=abcdefghijklmnopqrstuv'));
  assert.equal(gameSocketUrl('https://host.test', 'abc'), 'wss://host.test/ws/game-rooms/abc');
});
test('untrusted input rejects missing, nonfinite and out-of-range axes', () => {
  assert.equal(decodeInput({moveX: 1000}), null);
  assert.equal(decodeInput(null), null);
});

class Socket {
  static OPEN = 1;
  static instances: Socket[] = [];
  readyState = 0;
  bufferedAmount = 0;
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: {data: string}) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  readonly url: string;
  readonly protocols: string[];
  constructor(url: string, protocols: string[]) { this.url=url; this.protocols=protocols; Socket.instances.push(this); }
  open(): void { this.readyState = Socket.OPEN; this.onopen?.(); }
  send(value: string): void { assert.equal(this.readyState, Socket.OPEN); this.sent.push(value); }
  close(): void { if (this.readyState === 3) return; this.readyState = 3; this.onclose?.(); }
}
const credential: GameRoomCredential = {room_id:'room',member_id:'guest',role:'guest',token:'private',config:{game:'shooter',map:'station'}};
function transport(context: TestContext) {
  const original = globalThis.WebSocket;
  Socket.instances = [];
  globalThis.WebSocket = Socket as unknown as typeof WebSocket;
  context.after(() => { globalThis.WebSocket = original; });
  context.mock.timers.enable({apis:['setTimeout']});
  const states: string[] = [];
  const client = new GameClient('https://host.test',credential,()=>{},state=>states.push(state));
  context.after(() => client.dispose());
  return {client,states};
}

test('game reconnects stop after five retries even when sockets open without receiving room state', context => {
  const {client,states} = transport(context);
  client.connect();
  assert.equal(Socket.instances[0].url,'wss://host.test/ws/game-rooms/room');
  assert.deepEqual(Socket.instances[0].protocols,['otto-game','private']);
  for (const [index,delay] of [1000,2000,4000,5000,5000].entries()) {
    const socket = Socket.instances.at(-1)!;
    socket.open(); socket.close();
    context.mock.timers.tick(delay-1);
    assert.equal(Socket.instances.length,index+1,'retry must respect its backoff');
    context.mock.timers.tick(1);
    assert.equal(Socket.instances.length,index+2);
  }
  Socket.instances.at(-1)!.open(); Socket.instances.at(-1)!.close();
  context.mock.timers.tick(120000);
  assert.equal(Socket.instances.length,6,'initial connection plus five retries only');
  assert.equal(states.at(-1),'disconnected');
});

test('disposing an open game socket cancels heartbeat and silence timers and detaches callbacks', context => {
  const {client,states} = transport(context);
  const scheduled = context.mock.method(globalThis,'setTimeout');
  const cleared = context.mock.method(globalThis,'clearTimeout');
  client.connect();
  const socket = Socket.instances[0]; socket.open();
  context.mock.timers.tick(5000);
  assert.deepEqual(socket.sent,['{"type":"ping"}']);
  const heartbeat = scheduled.mock.calls.filter(call=>call.arguments[1]===5000).at(-1)!.result;
  const deadline = scheduled.mock.calls.filter(call=>call.arguments[1]===12000).at(-1)!.result;
  const beforeDispose = cleared.mock.calls.length;
  client.dispose();
  const cancelled = cleared.mock.calls.slice(beforeDispose).map(call=>call.arguments[0]);
  assert.ok(cancelled.includes(heartbeat),'pending heartbeat timer must be cleared');
  assert.ok(cancelled.includes(deadline),'silence timer must be cleared');
  assert.equal(socket.readyState,3);
  assert.deepEqual([socket.onopen,socket.onmessage,socket.onclose,socket.onerror],[null,null,null,null]);
  assert.deepEqual(socket.sent,['{"type":"ping"}','{"type":"leave"}']);
  const beforeStates = [...states];
  context.mock.timers.tick(120000); client.connect(); client.dispose();
  assert.equal(Socket.instances.length,1);
  assert.deepEqual(states,beforeStates);
  assert.equal(socket.sent.length,2,'disposed clients cannot send more heartbeats or leaves');
});

test('disposing while disconnected cancels the scheduled retry rather than leaving a dormant timer', context => {
  const {client,states} = transport(context);
  const scheduled = context.mock.method(globalThis,'setTimeout');
  const cleared = context.mock.method(globalThis,'clearTimeout');
  client.connect(); Socket.instances[0].close();
  const retry = scheduled.mock.calls.filter(call=>call.arguments[1]===1000).at(-1)!.result;
  const beforeDispose = cleared.mock.calls.length;
  client.dispose();
  assert.ok(cleared.mock.calls.slice(beforeDispose).some(call=>call.arguments[0]===retry),'retry timer must be cleared');
  const beforeStates = [...states];
  context.mock.timers.tick(120000);
  assert.equal(Socket.instances.length,1);
  assert.deepEqual(states,beforeStates);
});

test('snapshot decoder accepts the wire shape and rejects old-game or malformed renderer state', () => {
  const current = createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});
  const wire: unknown = JSON.parse(JSON.stringify(current));
  assert.deepEqual(decodeSnapshot(wire,current.config),current);
  const {tick: _tick,...legacy} = current;
  const cases: [string,unknown][] = [
    ['previous map',{...current,config:{...current.config,map:'foundry'}}],
    ['previous game',createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false})],
    ['old shape missing tick',legacy],
    ['missing opponent',{...current,players:[current.players[0]]}],
    ['nonfinite position',{...current,players:[{...current.players[0],x:NaN},current.players[1]]}],
    ['missing nested input',{...current,players:[{...current.players[0],botInput:null},current.players[1]]}],
    ['invalid event endpoint',{...current,events:[{id:1,type:'shot',player:0,x:0,y:0,z:0,end:{x:0,y:Infinity,z:0}}]}],
    ['oversized event list',{...current,events:Array.from({length:129},(_,id)=>({id,type:'hit',player:0,x:0,y:0,z:0}))}],
  ];
  for(const [label,value] of cases)assert.equal(decodeSnapshot(value,current.config),null,label);
});
