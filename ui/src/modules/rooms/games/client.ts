/** Game-room transport: the capability-scoped WebSocket client. */
import type {GameRoomCommand, GameRoomCredential, GameRoomEvent} from '../../../lib/api/game-room-types';
import { RelaySocket } from '../../../lib/relaySocket.ts';

export function gameSocketUrl(origin: string, id: string): string {
  const url = new URL(`/ws/game-rooms/${encodeURIComponent(id)}`, origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'; return url.href;
}
/** `gave-up`: the automatic reconnects ran out; the page offers a Reconnect button. */
export type GameConnection = 'connecting' | 'connected' | 'disconnected' | 'gave-up' | 'closed';
const EVENT_TYPES = new Set(['state', 'snapshot', 'input', 'finished', 'error', 'closed', 'pong']);
/** The game-room transport: a `RelaySocket` on the `otto-game` subprotocol. */
export class GameClient {
  private relay: RelaySocket<GameRoomEvent>;
  constructor(origin: string, credential: GameRoomCredential, event: (event: GameRoomEvent) => void, status: (state: GameConnection) => void) {
    this.relay = new RelaySocket<GameRoomEvent>({
      url: gameSocketUrl(origin, credential.room_id), protocol: 'otto-game', token: credential.token,
      maxBytes: 24 * 1024, silenceMs: 12000, heartbeatMs: 5000, heartbeat: JSON.stringify({type: 'ping'} satisfies GameRoomCommand),
      maxAttempts: 5, backoff: attempt => Math.min(5000, 1000 * 2 ** attempt), maxBuffered: 64 * 1024,
      parse: data => {
        let value: GameRoomEvent;
        try { value = JSON.parse(data) as GameRoomEvent; } catch { return null; }
        return value && typeof value === 'object' && EVENT_TYPES.has(value.type) ? value : null;
      },
      connectedBy: e => e.type === 'state',
      endedBy: e => e.type === 'closed',
      onEvent: event,
      onStatus: s => status(s === 'ended' ? 'closed' : s),
    });
  }
  connect(): void { this.relay.connect(); }
  /** The Reconnect button (and wake/online): a fresh retry budget. */
  reconnect(): void { this.relay.reconnect(); }
  send(command: GameRoomCommand): boolean { return this.relay.send(JSON.stringify(command)); }
  dispose(): void { if (this.relay.ended) return; this.send({type: 'leave'}); this.relay.dispose(); }
}
