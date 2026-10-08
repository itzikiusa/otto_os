/** Capability-isolated game relay. See docs/contracts/game-rooms.md. */
export type GameRoomKind = 'shooter' | 'kart';
export type GameRoomMap = 'station' | 'foundry' | 'dunes' | 'coast' | 'forest' | 'neon';
export type GameRoomRole = 'host' | 'guest';
export interface GameRoomConfig { game: GameRoomKind; map: GameRoomMap }
export interface GameRoomCredential {
  room_id: string; member_id: string; role: GameRoomRole; token: string;
  config: GameRoomConfig; invite?: string; invite_url?: string;
}
export interface GameRoomMember {
  id: string; name: string; role: GameRoomRole; connected: boolean; ready: boolean;
}
export interface GameRoomState {
  id: string; config: GameRoomConfig; round: number; generation: number;
  phase: 'lobby' | 'playing' | 'finished' | 'closed'; paused: boolean;
  members: GameRoomMember[];
}
export type GameRoomCommand =
  | { type: 'ready'; ready: boolean }
  | { type: 'start'; round: number; generation: number }
  | { type: 'snapshot' | 'input'; round: number; generation: number; seq: number; data: unknown }
  | { type: 'finish'; round: number; generation: number; result: unknown }
  | { type: 'rematch' | 'leave' | 'ping' };
export type GameRoomEvent =
  | { type: 'state'; room: GameRoomState }
  | { type: 'snapshot' | 'input'; round: number; generation: number; seq: number; data: unknown }
  | { type: 'finished'; round: number; generation: number; result: unknown }
  | { type: 'error'; message: string }
  | { type: 'closed'; reason: string }
  | { type: 'pong' };
