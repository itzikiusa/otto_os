import type { RoomAction, RoomCredential, RoomEvent } from '../../lib/api/room-types';
import { roomSocketAddress } from './room-state.ts';
export type RoomConnection = 'connecting' | 'connected' | 'disconnected' | 'ended';
/** Intentionally independent of api/client: a remote guest must never read owner auth. */
export async function joinRoom(origin: string, roomId: string, invite: string, name: string): Promise<RoomCredential> {
  const response = await fetch(new URL('/api/v1/room-join', origin), {
    method: 'POST', credentials: 'omit', referrerPolicy: 'no-referrer',
    headers: {'Content-Type': 'application/json'}, body: JSON.stringify({room_id: roomId, invite, name}),
  });
  if (!response.ok) {
    const problem = await response.json().catch(() => ({})) as {message?: string};
    throw new Error(problem.message ?? 'Could not join this room. Ask the host for a fresh invitation.');
  }
  return await response.json() as RoomCredential;
}
export class RoomClient {
  private socket: WebSocket | null = null;
  private heartbeat: ReturnType<typeof setInterval> | undefined;
  private reconnect: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;
  private attempts = 0;
  private silence: ReturnType<typeof setTimeout> | undefined;
  readonly origin: string;
  readonly credential: RoomCredential;
  private event: (event: RoomEvent) => void;
  private status: (status: RoomConnection) => void;
  constructor(origin: string, credential: RoomCredential, event: (event: RoomEvent) => void, status: (status: RoomConnection) => void) {
    this.origin = origin; this.credential = credential; this.event = event; this.status = status;
  }
  connect(): void {
    if (this.disposed) return;
    clearTimeout(this.reconnect);
    this.detach();
    this.status('connecting');
    const socket = new WebSocket(roomSocketAddress(this.origin, this.credential.room_id), ['otto-room', this.credential.token]);
    this.socket = socket;
    // A failed upgrade may never reach onopen; bound that wait as well.
    this.armSilence(socket);
    socket.onopen = () => {
      this.armSilence(socket);
      this.heartbeat = setInterval(() => this.send({type: 'heartbeat'}), 5000);
    };
    socket.onmessage = (message) => {
      if (typeof message.data !== 'string' || message.data.length > 1024 * 1024) return;
      this.armSilence(socket);
      let event: RoomEvent;
      try { event = JSON.parse(message.data) as RoomEvent; } catch { return; }
      if (event.type === 'snapshot') { this.attempts = 0; this.status('connected'); }
      if (event.type === 'ended') { this.disposed = true; this.detach(); this.status('ended'); }
      this.event(event);
    };
    socket.onerror = () => this.status('disconnected');
    socket.onclose = () => {
      clearInterval(this.heartbeat);
      clearTimeout(this.silence);
      if (this.disposed) return;
      this.status('disconnected');
      // Explicit retry remains available; automatic retries are bounded by server credential expiry.
      if (this.attempts < 6) this.reconnect = setTimeout(() => this.connect(), Math.min(30000, 1000 * 2 ** this.attempts++));
    };
  }
  send(action: RoomAction): boolean {
    if (this.socket?.readyState !== WebSocket.OPEN || this.disposed) return false;
    this.socket.send(JSON.stringify(action));
    return true;
  }
  terminal(): WebSocket { return new WebSocket(roomSocketAddress(this.origin, this.credential.room_id, true), ['otto-room', this.credential.token]); }
  private armSilence(socket: WebSocket): void {
    clearTimeout(this.silence);
    this.silence = setTimeout(() => {
      if (socket !== this.socket || this.disposed) return;
      this.status('disconnected');
      socket.close();
    }, 10000);
  }
  private detach(): void {
    clearInterval(this.heartbeat);
    clearTimeout(this.silence);
    if (this.socket) { this.socket.onclose = null; this.socket.onmessage = null; this.socket.onerror = null; this.socket.onopen = null; this.socket.close(); this.socket = null; }
  }
  dispose(): void { this.disposed = true; clearTimeout(this.reconnect); this.detach(); }
}
