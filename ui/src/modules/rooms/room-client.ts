import type { RoomAction, RoomCredential, RoomEvent } from '../../lib/api/room-types';
import { roomSocketAddress } from './room-state.ts';
import { RelaySocket } from '../../lib/relaySocket.ts';
export type RoomConnection = 'connecting' | 'connected' | 'disconnected' | 'ended';
export const JOIN_UNREACHABLE = 'Couldn’t reach the host. Check the link or ask the host to keep Otto open, then try again.';
/** Intentionally independent of api/client: a remote guest must never read owner auth. */
export async function joinRoom(origin: string, roomId: string, invite: string, name: string): Promise<RoomCredential> {
  let response: Response;
  try {
    response = await fetch(new URL('/api/v1/room-join', origin), {
      method: 'POST', credentials: 'omit', referrerPolicy: 'no-referrer',
      headers: {'Content-Type': 'application/json'}, body: JSON.stringify({room_id: roomId, invite, name}),
    });
  } catch {
    // fetch rejects (TypeError "Load failed") only when the host is unreachable;
    // the guest must never see that raw engine text.
    throw new Error(JOIN_UNREACHABLE);
  }
  if (!response.ok) {
    const problem = await response.json().catch(() => ({})) as {message?: string};
    throw new Error(problem.message ?? 'Couldn’t join this room. Ask the host for a fresh invitation.');
  }
  return await response.json() as RoomCredential;
}
/** The session-room transport: a `RelaySocket` on the `otto-room` subprotocol. */
export class RoomClient {
  private relay: RelaySocket<RoomEvent>;
  readonly origin: string;
  readonly credential: RoomCredential;
  constructor(origin: string, credential: RoomCredential, event: (event: RoomEvent) => void, status: (status: RoomConnection) => void) {
    this.origin = origin; this.credential = credential;
    this.relay = new RelaySocket<RoomEvent>({
      url: roomSocketAddress(origin, credential.room_id), protocol: 'otto-room', token: credential.token,
      maxBytes: 1024 * 1024, silenceMs: 10000, heartbeatMs: 5000, heartbeat: JSON.stringify({type: 'heartbeat'} satisfies RoomAction),
      // Explicit retry remains available; automatic retries are bounded by server credential expiry.
      maxAttempts: 6, backoff: attempt => Math.min(30000, 1000 * 2 ** attempt),
      parse: data => { try { return JSON.parse(data) as RoomEvent; } catch { return null; } },
      connectedBy: event => event.type === 'snapshot',
      endedBy: event => event.type === 'ended',
      onEvent: event,
      // The room page keeps its own explicit retry, so running out of attempts reads as disconnected.
      onStatus: s => status(s === 'gave-up' ? 'disconnected' : s),
    });
  }
  connect(): void { this.relay.connect(); }
  send(action: RoomAction): boolean { return this.relay.send(JSON.stringify(action)); }
  terminal(): WebSocket { return new WebSocket(roomSocketAddress(this.origin, this.credential.room_id, true), ['otto-room', this.credential.token]); }
  dispose(): void { this.relay.dispose(); }
}
