import type { RoomAction, RoomCredential, RoomEvent } from '../../lib/api/room-types';
import type { Transcript } from '../../lib/api/types';
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
/** A request to the room host authenticated ONLY by the room capability
 *  (header, never the URL) — owner auth is never read or sent. */
export async function roomFetch(origin: string, credential: RoomCredential, path: string, init: RequestInit = {}): Promise<Response> {
  let response: Response;
  try {
    response = await fetch(new URL(`/api/v1${path}`, origin), {
      ...init, credentials: 'omit', referrerPolicy: 'no-referrer',
      headers: {...(init.headers as Record<string, string> | undefined), 'x-otto-room-token': credential.token},
    });
  } catch {
    throw new Error(JOIN_UNREACHABLE);
  }
  if (!response.ok) {
    const problem = await response.json().catch(() => ({})) as {message?: string};
    throw new Error(problem.message ?? `The host refused this request (${response.status}).`);
  }
  return response;
}
/** Store a pasted image on the host and type its path (driver only). */
export async function pasteRoomImage(origin: string, credential: RoomCredential, grantEpoch: number, png: Blob): Promise<void> {
  await roomFetch(origin, credential, `/room-paste/${encodeURIComponent(credential.room_id)}`, {
    method: 'POST', body: png, headers: {'Content-Type': 'image/png', 'x-otto-grant-epoch': String(grantEpoch)},
  });
}
/** The shared session's conversation page (same shape as the owner's `GET …/transcript`). */
export async function fetchRoomTranscript(origin: string, credential: RoomCredential, limit = 80): Promise<Transcript> {
  const r = await roomFetch(origin, credential, `/room-transcript/${encodeURIComponent(credential.room_id)}?limit=${limit}`);
  return await r.json() as Transcript;
}
/** A transcript image as a revocable blob URL. */
export async function roomImageUrl(origin: string, credential: RoomCredential, imageId: string): Promise<string> {
  const r = await roomFetch(origin, credential, `/room-transcript/${encodeURIComponent(credential.room_id)}/images/${encodeURIComponent(imageId)}`);
  return URL.createObjectURL(await r.blob());
}
/** Submit a prompt (or, with `submit: false`, raw keys such as Esc) as the driver. */
export async function roomPrompt(origin: string, credential: RoomCredential, grantEpoch: number, text: string, submit: boolean): Promise<void> {
  await roomFetch(origin, credential, `/room-prompt/${encodeURIComponent(credential.room_id)}`, {
    method: 'POST', body: JSON.stringify({text, submit}),
    headers: {'Content-Type': 'application/json', 'x-otto-grant-epoch': String(grantEpoch)},
  });
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
