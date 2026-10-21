/** Capability-isolated game relay. See docs/contracts/game-rooms.md. */
export type GameRoomKind = 'shooter' | 'kart';
export type GameRoomMap = 'isle' | 'mesa' | 'meadow' | 'downtown' | 'canyon' | 'frost' | 'lagoon' | 'volcano' | 'skyway';
export type GameRoomRole = 'host' | 'guest';
export interface GameRoomConfig { game: GameRoomKind; map: GameRoomMap }
export interface GameRoomCredential {
  room_id: string; member_id: string; role: GameRoomRole; token: string;
  config: GameRoomConfig; invite?: string; invite_url?: string;
}
export interface GameRoomMember {
  id: string; name: string; role: GameRoomRole;
  /** Index in the member list; stable for the room's lifetime (host is 0). */
  slot: number;
  connected: boolean; ready: boolean;
  /** Guest left or outlived the reconnect grace; never reconnects. */
  left: boolean;
  /** Guest asked the host for a rematch. */
  rematch: boolean;
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
  | { type: 'snapshot'; round: number; generation: number; seq: number; data: unknown }
  | { type: 'input'; round: number; generation: number; seq: number; slot: number; data: unknown }
  | { type: 'finished'; round: number; generation: number; result: unknown }
  | { type: 'error'; message: string }
  | { type: 'closed'; reason: string }
  | { type: 'pong' };
