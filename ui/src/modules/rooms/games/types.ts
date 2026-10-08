/** Simulation coordinates: metres, Y up, yaw 0 faces +Z, positive pitch looks up. */
export type GameKind = 'shooter' | 'kart';
export type Difficulty = 'easy' | 'normal' | 'hard';
export interface GameConfig { kind: GameKind; map: string; difficulty: Difficulty; vsComputer: boolean; seed?: number }
export interface Vec3 { x: number; y: number; z: number }
export interface CoverBox extends Vec3 { width: number; height: number; depth: number }
export interface ArenaMap { id: string; name: string; halfSize: number; cover: CoverBox[]; spawns: Vec3[] }
export interface TrackMap { id: string; name: string; width: number; route: Vec3[]; pickups: number[] }
/** Shooter axes are camera-relative; kart axes are steering and throttle. Angles absolute. */
export interface GameInput {
  moveX: number; moveZ: number; yaw: number; pitch: number;
  fire: boolean; reload: boolean; jump: boolean; sprint: boolean;
  drift: boolean; item: boolean; reset: boolean;
}
export interface GamePlayer extends Vec3 {
  id: number; yaw: number; pitch: number; hp: number; ammo: number; score: number;
  speed: number; steering: number; velocityY: number; grounded: boolean; moving: boolean;
  cooldown: number; reloadTime: number; respawnTime: number; invulnerable: number;
  lap: number; checkpoint: number; boost: number; driftCharge: number; drifting: boolean;
  item: 'boost' | 'pulse' | null; itemCooldown: number; offTrack: boolean; offTrackTime: number;
  finishTime: number | null; resetCooldown: number;
  botThink: number; botInput: GameInput; botTarget: number; botStrafe: number;
}
export interface GameEvent {
  id: number; type: 'shot' | 'hit' | 'kill' | 'respawn' | 'boost' | 'pickup' | 'lap' | 'finish';
  player: number; x: number; y: number; z: number; target?: number; end?: Vec3;
}
/** Every field is JSON-safe; snapshots can be passed directly to the renderer. */
export interface GameState {
  config: GameConfig; phase: 'countdown' | 'playing' | 'finished';
  countdown: number; elapsed: number; remaining: number; winner: number | null;
  players: [GamePlayer, GamePlayer]; events: GameEvent[]; eventSequence: number;
  rng: number; accumulator: number; tick: number; pickupTimers: number[];
}
