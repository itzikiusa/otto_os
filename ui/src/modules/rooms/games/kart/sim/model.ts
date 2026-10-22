// Kart race model: the tuning constants, the racer / shell / event shapes
// and the replicated KartState (re-exported by ../sim.ts).
import type { TrackId } from '../tracks.ts';
import type { RacerId } from '../roster.ts';
import type { ItemKind } from '../items.ts';

export type Difficulty = 'easy' | 'normal' | 'hard';
export const STEP = 1 / 60;
export const LAPS = 3;
export const MAX_RACERS = 8;
export const MAX_HAZARDS = 20;
export const MAX_SHELLS = 8;
export const INTRO_SECONDS = 2.5;
export const COUNTDOWN_SECONDS = 3;
/** Drift charge (seconds) needed for each mini-turbo tier: blue, orange, purple sparks. */
export const DRIFT_LEVELS = [0.7, 1.45, 2.4];
/** Mini-turbo per tier: seconds and top-speed multiplier (each tier longer and stronger). */
export const TURBO_SECONDS = [0.7, 1.2, 1.9];
export const TURBO_POWER = [1.3, 1.38, 1.46];
/** Turbo mushroom (one each of a triple; every press inside a golden mushroom's window): a long, strong burst. */
export const MUSHROOM_SECONDS = 1.5;
export const MUSHROOM_POWER = 1.55;
/** Golden mushroom: unlimited mushrooms for this long after the first press. */
export const GOLDEN_SECONDS = 7.5;
/** Dash panels and trick landings. */
export const PAD_SECONDS = 1.1, PAD_POWER = 1.45;
export const TRICK_SECONDS = 1.0, TRICK_POWER = 1.38;
/**
 * Rocket start, by the countdown value when the gas went down (and stayed
 * down through GO): a perfect window around "2" fading, a good one later, and
 * a burnout (a stalled moment) when floored before then.
 */
export const START_PERFECT: [number, number] = [0.75, 1.45];
export const START_GOOD = 0.2;
export const START_BURNOUT = 1.9;
export const BURNOUT_SECONDS = 0.8;
/** Slipstream: this long in another racer's wake (3–20 m behind, in line) fires a draft boost. */
export const DRAFT_SECONDS = 1.1;
export const DRAFT_BOOST = 1.0, DRAFT_POWER = 1.32;
/** After a rescue: a moment of invulnerability and a rolling restart. */
export const GHOST_SECONDS = 2;
export const RESCUE_SECONDS = 1.0;
/** A CPU that gains under STUCK_METRES of lap distance in STUCK_SECONDS calls the rescue (never sits stuck). */
export const STUCK_SECONDS = 4, STUCK_METRES = 8;
/** Gas down as a rescued kart lands: a short boost away. */
export const RESPAWN_BOOST = 0.7;
/** Shells and bombs: lifetime, speed, the owner's grace and the blast radius. */
export type ShellKind = 'green' | 'red' | 'blue' | 'bomb';
export const SHELL_KINDS: ShellKind[] = ['green', 'red', 'blue', 'bomb'];
export const SHELL: Record<ShellKind, { life: number; speed: number; blast: number }> = {
  green: { life: 5, speed: 52, blast: 0 },
  red: { life: 7, speed: 50, blast: 0 },
  blue: { life: 14, speed: 68, blast: 7 },
  bomb: { life: 2.6, speed: 26, blast: 7.5 },
};
/** A green shell can come back for its thrower only after this long (never a red, a blue or a bomb). */
export const GREEN_GRACE = 1.0;
/** A dropped banana never trips its owner during this long. */
export const BANANA_GRACE = 1.5;
export const GRAVITY = 24;
/** Glider: gentle gravity and a capped sink rate. */
export const GLIDE_GRAVITY = 9;
export const GLIDE_SINK = 6.5;
/** Underwater the karts float: lower gravity and a little drag. */
export const WATER_GRAVITY = 12;
/** A racer this far below the road (no ground under it) has fallen and is rescued. */
export const FALL_DEPTH = 12;
/** Coins: each one up to MAX_COINS raises top speed by COIN_SPEED; a spin-out or a fall costs COIN_LOSS. */
export const MAX_COINS = 10;
export const COIN_SPEED = 0.006;
export const COIN_LOSS = 3;
/** Seconds before a collected coin reappears. */
export const COIN_RESPAWN = 12;

export interface KartInput { throttle: number; steer: number; drift: boolean; item: boolean; respawn: boolean }
export const idleInput = (): KartInput => ({ throttle: 0, steer: 0, drift: false, item: false, respawn: false });

export interface Racer {
  id: number; racer: RacerId; human: boolean; slot: number; name: string;
  x: number; y: number; z: number; yaw: number; vx: number; vz: number; vy: number;
  speed: number; steer: number; grounded: boolean; air: number; trick: number; trickReady: boolean; trickDone: boolean;
  drift: number; driftCharge: number; driftLevel: number;
  boost: number; boostPower: number; spin: number; star: number; slow: number;
  /** Golden mushroom window left; `ghost` post-rescue invulnerability; `draft` slipstream charge; `stall` burnout. */
  golden: number; ghost: number; draft: number; stall: number;
  item: ItemKind | null; itemCount: number; roll: number; prevItem: boolean; prevDrift: boolean;
  /** `lastS` is lap progress (main-loop distance); `seg`/`segS` the path and local distance on it. */
  travel: number; lastS: number; hint: number; lateral: number; surface: 'road' | 'grass' | 'ice';
  seg: number; segS: number; glide: boolean; under: boolean;
  /** Last on-road spot (rescues return here, never into a chasm). */
  safeSeg: number; safeS: number;
  lap: number; lapStart: number; lapTimes: number[]; finish: number | null; place: number;
  respawn: number; wrong: number; startCharge: number; bumpCd: number;
  /** CPU brain; the stuck watchdog counts `cpuStuck` seconds since `travel` last got STUCK_METRES past `cpuMark`. */
  cpuLane: number; cpuStuck: number; cpuMark: number; cpuItemDelay: number; cpuMistake: number;
  /** CPU route: bit k-1 set = take branch k this lap. */
  cpuRoute: number;
  coins: number;
}
export interface ItemBox { x: number; y: number; z: number; timer: number }
/** A coin on the road; `timer` > 0 while it is collected and waiting to reappear. */
export interface Coin { x: number; y: number; z: number; timer: number }
/** A banana on the road; `age` seconds since it was dropped (host only). */
export interface Hazard { id: number; x: number; y: number; z: number; owner: number; age: number }
/** A shell or bomb in flight: `vy` for a bomb's arc, `speed` its current ground speed (host only). */
export interface Shell { id: number; kind: ShellKind; x: number; y: number; z: number; yaw: number; target: number; owner: number; life: number; hint: number; seg: number; vy: number; speed: number }
/**
 * `respawn` value: 2 when a stuck CPU called the rescue (falls and requests carry none);
 * `boost` value: 0 ramp / pad, 1 mushroom, 2 perfect start, 3 golden; `turbo`
 * value: mini-turbo tier; `bolt`: the user's strike (value = racers hit), then
 * a `zap` per opponent struck; `draft`: slipstream boost; `burnout`: floored
 * too early at the start; `explode` value: shell kind index.
 */
export type KartEventType = 'box' | 'item' | 'boost' | 'turbo' | 'spin' | 'explode' | 'bump' | 'lap' | 'final' | 'finish' | 'bolt' | 'drop' | 'throw' | 'star' | 'respawn' | 'trick' | 'land' | 'go' | 'count' | 'splash' | 'glide' | 'crush' | 'zap' | 'draft' | 'burnout';
export interface KartEvent { id: number; type: KartEventType; racer: number; x: number; y: number; z: number; value?: number }
export interface KartSeat { racer: RacerId; human: boolean; slot: number; name: string }
export interface KartState {
  track: TrackId; difficulty: Difficulty; laps: number;
  phase: 'intro' | 'countdown' | 'race' | 'done';
  clock: number; countdown: number; raceTime: number; endTimer: number; firstFinish: number | null;
  racers: Racer[]; boxes: ItemBox[]; coins: Coin[]; hazards: Hazard[]; shells: Shell[];
  events: KartEvent[]; eventSeq: number; nextId: number; rng: number; accumulator: number;
}
