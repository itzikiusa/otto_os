import type {GameRoomCommand, GameRoomCredential, GameRoomEvent} from '../../../lib/api/game-room-types';
import type {GameInput, GameConfig, GameState} from './types';
import {trackFor} from './maps.ts';

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
const numericInput = ['moveX', 'moveZ', 'yaw', 'pitch'] as const;
const booleanInput = ['fire', 'reload', 'jump', 'sprint', 'drift', 'item', 'reset'] as const;
function record(value: unknown): value is Record<string, unknown> { return !!value && typeof value === 'object' && !Array.isArray(value); }
const characters=['fox','panda','rabbit','robot'];
function finite(value: unknown): value is number { return typeof value === 'number' && Number.isFinite(value) && Math.abs(value) < 1e9; }
export function decodeInput(value: unknown): GameInput | null {
  if (!record(value) || (value.weapon!==undefined&&![0,1,2,3].includes(value.weapon as number)) || (value.character!==undefined&&!characters.includes(value.character as string)) || numericInput.some(k => !finite(value[k])) || booleanInput.some(k => typeof value[k] !== 'boolean') || Math.abs(value.moveX as number) > 1 || Math.abs(value.moveZ as number) > 1 || Math.abs(value.pitch as number) > 1.6) return null;
  return value as unknown as GameInput;
}
export function decodeSnapshot(value: unknown, config: GameConfig): GameState | null {
  if (!record(value) || !record(value.config) || value.config.kind !== config.kind || value.config.map !== config.map || !['easy','normal','hard'].includes(String(value.config.difficulty)) || typeof value.config.vsComputer !== 'boolean') return null;
  if (!['countdown','playing','finished'].includes(String(value.phase)) || !Array.isArray(value.players) || value.players.length !== 2 || !Array.isArray(value.events) || value.events.length > 128 || !Array.isArray(value.pickupTimers) || value.pickupTimers.length > 64 || value.pickupTimers.some(n => !finite(n))) return null;
  if (typeof value.rng!=='number'||!Number.isInteger(value.rng)||value.rng<0||value.rng>0xffffffff)return null;
  if (!Array.isArray(value.projectiles)||value.projectiles.length>8)return null;
  for(const p of value.projectiles)if(!record(p)||!['id','x','y','z','life'].every(k=>finite(p[k]))||![0,1].includes(p.owner as number)||![0,1].includes(p.target as number))return null;
  if (!['countdown','elapsed','remaining','eventSequence','accumulator','tick'].every(k => finite(value[k])) || ![null,0,1].includes(value.winner as null | number)) return null;
  const numeric = ['shield','damageTime','dashCooldown','airTime','launchCooldown','x','y','z','yaw','pitch','hp','ammo','score','speed','steering','velocityY','cooldown','reloadTime','respawnTime','invulnerable','lap','checkpoint','boost','driftCharge','itemCooldown','offTrackTime','resetCooldown','botThink','botTarget','botStrafe'];
  const checkpointCount=config.kind==='kart'?trackFor(config.map).route.length:0;
  for (let i = 0; i < 2; i++) {
    const p = value.players[i];
    if (!record(p) || !characters.includes(p.character as string) || !['rifle','scatter','rail'].includes(p.weapon as string) || p.id !== i || numeric.some(k => !finite(p[k])) || ['grounded','moving','drifting','offTrack','underwater','trick'].some(k => typeof p[k] !== 'boolean') || !decodeInput(p.botInput) || ![null,'boost','pulse','shield','seeker'].includes(p.item as null | string) || (p.finishTime !== null && !finite(p.finishTime))) return null;
    if(config.kind==='kart'&&(!Number.isInteger(p.checkpoint)||(p.checkpoint as number)<0||(p.checkpoint as number)>=checkpointCount))return null;
  }
  for (const e of value.events) {
    if (!record(e) || !['shot','hit','kill','respawn','boost','pickup','lap','finish','launch','land','trick','shield','dash'].includes(String(e.type)) || !['id','player','x','y','z'].every(k => finite(e[k])) || ![0,1].includes(e.player as number) || (e.end !== undefined && (!record(e.end) || !['x','y','z'].every(k => finite((e.end as Record<string,unknown>)[k]))))) return null;
  }
  return value as unknown as GameState;
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
