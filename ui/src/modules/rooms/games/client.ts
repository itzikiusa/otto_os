/** Game-room transport: invitation parsing and the capability-scoped WebSocket client. */
import type {GameRoomCommand, GameRoomCredential, GameRoomEvent} from '../../../lib/api/game-room-types';

export function parseGameInvite(value: string): {origin: string; roomId: string; invite: string} {
  const url = new URL(value.trim());
  const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if ((url.protocol !== 'https:' && !(local && url.protocol === 'http:')) || url.username || url.password || url.pathname !== '/' || url.search) throw new Error('Use the complete HTTPS game invitation. HTTP is supported on this computer only.');
  const match = /^#\/game-room\/([\w-]{1,128})\?invite=([\w-]{16,512})$/.exec(url.hash);
  if (!match) throw new Error('Paste the complete game invitation, including its invitation code.');
  return {origin: url.origin, roomId: match[1], invite: match[2]};
}
export function gameSocketUrl(origin: string, id: string): string {
  const url = new URL(`/ws/game-rooms/${encodeURIComponent(id)}`, origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'; return url.href;
}
export type GameConnection = 'connecting' | 'connected' | 'disconnected' | 'closed';
/** No owner API imports, local storage, or native privileges in guest transport. */
export class GameClient {
  private socket: WebSocket | null = null;
  private ping: ReturnType<typeof setTimeout> | undefined;
  private deadline: ReturnType<typeof setTimeout> | undefined;
  private retry: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;
  private attempts = 0;
  private origin: string;
  private credential: GameRoomCredential;
  private event: (event: GameRoomEvent) => void;
  private status: (state: GameConnection) => void;
  constructor(origin: string, credential: GameRoomCredential, event: (event: GameRoomEvent) => void, status: (state: GameConnection) => void) { this.origin = origin; this.credential = credential; this.event = event; this.status = status; }
  connect(): void {
    if (this.disposed) return;
    this.detach(); clearTimeout(this.retry); this.status('connecting');
    const socket = this.socket = new WebSocket(gameSocketUrl(this.origin, this.credential.room_id), ['otto-game', this.credential.token]);
    const arm = () => { clearTimeout(this.deadline); this.deadline = setTimeout(() => { this.status('disconnected'); socket.close(); }, 12000); };
    arm();
    socket.onopen = () => { arm(); const ping = () => { if (this.disposed || this.socket !== socket) return; this.send({type:'ping'}); this.ping = setTimeout(ping, 5000); }; this.ping = setTimeout(ping, 5000); };
    socket.onmessage = message => {
      if (typeof message.data !== 'string' || message.data.length > 24 * 1024) return;
      let event: GameRoomEvent;
      try { event = JSON.parse(message.data) as GameRoomEvent; } catch { return; }
      if (!event || typeof event !== 'object' || !['state','snapshot','input','finished','error','closed','pong'].includes(event.type)) return;
      arm();
      if (event.type === 'state') { this.attempts = 0; this.status('connected'); }
      if (event.type === 'closed') { this.disposed = true; this.detach(); this.status('closed'); }
      this.event(event);
    };
    socket.onerror = () => this.status('disconnected');
    socket.onclose = () => {
      clearTimeout(this.ping); clearTimeout(this.deadline);
      if (this.disposed) return; this.status('disconnected');
      if (this.attempts < 5) this.retry = setTimeout(() => this.connect(), Math.min(5000, 1000 * 2 ** this.attempts++));
    };
  }
  send(command: GameRoomCommand): boolean {
    if (this.disposed || this.socket?.readyState !== WebSocket.OPEN || this.socket.bufferedAmount > 64 * 1024) return false;
    this.socket.send(JSON.stringify(command)); return true;
  }
  private detach(): void { clearTimeout(this.ping); clearTimeout(this.deadline); if (this.socket) { this.socket.onclose = null; this.socket.onmessage = null; this.socket.onerror = null; this.socket.onopen = null; this.socket.close(); this.socket = null; } }
  dispose(): void { if (this.disposed) return; this.send({type:'leave'}); this.disposed = true; clearTimeout(this.retry); this.detach(); }
}
