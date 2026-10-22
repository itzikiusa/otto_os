/**
 * Kart Grand Prix simulation (host-authoritative, fixed 60 Hz, no three.js).
 * Up to 8 racers: humans plus CPU drivers. Drifting with three mini-turbo
 * tiers (blue, orange, purple sparks: longer and stronger each), a rocket
 * start (or a burnout if the gas goes down too early), slipstreaming, item
 * boxes with position-weighted odds, ten items (mushrooms, triple and golden
 * mushrooms, banana, green / red / blue shells, fuse bomb, star, lightning),
 * jumps with trick boosts, launch ramps and the glider, dash panels, ice,
 * off-road patches (any boost blasts through), banking, walls and rails,
 * branching routes, drawbridges, half-pipe trick lips, coins (up to ten raise
 * top speed; hits and falls cost some), underwater driving, quick fair
 * rescues (back on the road with a moment of invulnerability), rolling and
 * crushing obstacles, laps with progress tracking and live ranking.
 *
 * Nothing a racer fires or drops can hurt its owner: lightning strikes every
 * opponent and never the user, shells and bombs spare their thrower (a green
 * shell only after a short grace, once it has bounced back), and a banana is
 * harmless to whoever dropped it for its first moments.
 */
import { approach, clamp, Rng, wrapAngle } from '../common/math.ts';
import {
  advance, circ, coinSpots, CRUSHER_HALF, DRAW_ANGLE, drawLift, F_ICE, F_OFFROAD, F_RAILS, F_UNDER, inLaunchZone, inOffroad, locate, nearest, newNear, obstacleAt,
  pointAt, progressOf, ROLLER_RADIUS, sampleAt, trackFor, type Near, type Spot, type Track, type TrackId,
} from './tracks.ts';
import { racerDef, type RacerId } from './roster.ts';
import { cpuInput } from './cpu.ts';
import { rollItem, type ItemKind } from './items.ts';

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
  cpuLane: number; cpuThink: number; cpuItemDelay: number; cpuMistake: number;
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

const _near: Near = newNear();
const _pt = [0, 0, 0, 0];
const _ob = [0, 0, 0, 0];
const _spot: Spot = { seg: 0, s: 0 };
const DIFF_INDEX: Record<Difficulty, number> = { easy: 0, normal: 1, hard: 2 };
/** Rolls a CPU's routes for one lap: each branch is taken with its per-difficulty chance. */
export function pickRoute(track: Track, difficulty: Difficulty, rng: Rng): number {
  let route = 0;
  for (let b = 1; b < track.paths.length; b++) if (rng.chance(track.paths[b].cpu[DIFF_INDEX[difficulty]])) route |= 1 << (b - 1);
  return route;
}

export function createRace(trackId: TrackId, seats: KartSeat[], difficulty: Difficulty, seed: number): KartState {
  const track = trackFor(trackId);
  const rng = new Rng(seed);
  // Humans start mid-grid (from 4th back), CPUs fill the slots around them,
  // so there are rivals both ahead and behind from the start.
  const cpus = seats.filter(s => !s.human), humans = seats.filter(s => s.human);
  const first = Math.min(3, Math.max(0, Math.min(seats.length, MAX_RACERS) - humans.length));
  const order = [...cpus.slice(0, first), ...humans, ...cpus.slice(first)].slice(0, MAX_RACERS);
  const racers = order.map((seat, i): Racer => {
    const row = Math.floor(i / 2), side = i % 2 ? -1 : 1;
    const s = -9 - row * 7.5 - (i % 2) * 3.2;
    pointAt(track, s, side * track.halfWidth * 0.42, _pt);
    const lap = ((s % track.length) + track.length) % track.length;
    return {
      id: i, racer: seat.racer, human: seat.human, slot: seat.slot, name: seat.name,
      x: _pt[0], y: _pt[1], z: _pt[2], yaw: _pt[3], vx: 0, vz: 0, vy: 0,
      speed: 0, steer: 0, grounded: true, air: 0, trick: 0, trickReady: false, trickDone: false,
      drift: 0, driftCharge: 0, driftLevel: 0, boost: 0, boostPower: 0, spin: 0, star: 0, slow: 0,
      golden: 0, ghost: 0, draft: 0, stall: 0,
      item: null, itemCount: 0, roll: 0, prevItem: false, prevDrift: false,
      travel: s, lastS: lap, hint: -1, lateral: 0, surface: 'road',
      seg: 0, segS: lap, glide: false, under: false, safeSeg: 0, safeS: lap,
      lap: 0, lapStart: 0, lapTimes: [], finish: null, place: i + 1, respawn: 0, wrong: 0, startCharge: -1, bumpCd: 0,
      cpuLane: rng.range(-0.45, 0.45), cpuThink: 0, cpuItemDelay: rng.range(0.5, 2.5), cpuMistake: 0,
      cpuRoute: seat.human ? 0 : pickRoute(track, difficulty, rng), coins: 0,
    };
  });
  const boxes = itemBoxes(track);
  const state: KartState = {
    track: track.def.id, difficulty, laps: LAPS, phase: 'intro', clock: 0, countdown: COUNTDOWN_SECONDS, raceTime: 0,
    endTimer: 0, firstFinish: null, racers, boxes, coins: trackCoins(track), hazards: [], shells: [], events: [], eventSeq: 0, nextId: 1,
    rng: rng.state, accumulator: 0,
  };
  rank(state);
  return state;
}

/** Rows of item boxes across the road (five on full-width roads, three on narrow branches). */
export function itemBoxes(track: Track): ItemBox[] {
  const boxes: ItemBox[] = [];
  for (const path of track.paths) {
    const per = path.halfWidth < 6 ? 3 : 5;
    for (const at of path.features.itemRows) for (let k = 0; k < per; k++) {
      pointAt(path, at * path.length, (k - (per - 1) / 2) * path.halfWidth * (per === 5 ? 0.34 : 0.5), _pt);
      boxes.push({ x: _pt[0], y: _pt[1] + 1.1, z: _pt[2], timer: 0 });
    }
  }
  return boxes;
}

/** Every coin of the track, all in place. */
export function trackCoins(track: Track): Coin[] {
  const spots = coinSpots(track), coins: Coin[] = [];
  for (let i = 0; i < spots.length; i += 3) coins.push({ x: spots[i], y: spots[i + 1], z: spots[i + 2], timer: 0 });
  return coins;
}

function emit(state: KartState, type: KartEventType, r: { id: number; x: number; y: number; z: number }, value?: number): void {
  state.events.push({ id: ++state.eventSeq, type, racer: r.id, x: r.x, y: r.y, z: r.z, value });
  if (state.events.length > 48) state.events.shift();
}

/** Ranking: finishers by time, then by distance travelled. */
export function rank(state: KartState): void {
  const sorted = [...state.racers].sort((a, b) => {
    if (a.finish !== null || b.finish !== null) return (a.finish ?? Infinity) - (b.finish ?? Infinity);
    return b.travel - a.travel;
  });
  sorted.forEach((r, i) => { r.place = i + 1; });
}

const stats = (r: Racer) => {
  const d = racerDef(r.racer);
  return { top: 29 + (d.speed - 3) * 0.9, accel: 15 + (d.accel - 3) * 2.2, turn: 1.9 + (d.handling - 3) * 0.14, weight: 0.8 + d.weight * 0.1 };
};
const CPU_SPEED: Record<Difficulty, number> = { easy: 0.86, normal: 0.94, hard: 1.0 };
/** Countdown value at which CPUs floor the throttle. */
const CPU_START: Record<Difficulty, number> = { easy: 0.12, normal: 0.6, hard: 1.2 };

/**
 * Starts (or extends) a boost: `seconds` at `power` × the racer's top speed,
 * with a shove of `kick` of the way there at once so it is felt straight away.
 * The strongest boost running wins; a later one can only lengthen it.
 */
export function giveBoost(r: Racer, seconds: number, power: number, kick = 0.3): void {
  const top = stats(r).top * power;
  r.boostPower = r.boost > 0 ? Math.max(r.boostPower, power) : power;
  r.boost = Math.max(r.boost, seconds);
  if (r.speed < top) r.speed += (top - r.speed) * kick;
}

/** Advances the race by real time `dt` using fixed steps (at most 6 per call). */
export function stepRace(state: KartState, inputs: (KartInput | undefined)[], dt: number): void {
  if (!Number.isFinite(dt) || dt <= 0) return;
  state.accumulator = Math.min(state.accumulator + Math.min(dt, 0.1), STEP * 6);
  while (state.accumulator >= STEP - 1e-9) {
    state.accumulator -= STEP;
    tick(state, inputs);
  }
}

export function tick(state: KartState, inputs: (KartInput | undefined)[]): void {
  const track = trackFor(state.track);
  state.clock += STEP;
  if (state.phase === 'intro') {
    if (state.clock >= INTRO_SECONDS) { state.phase = 'countdown'; emit(state, 'count', state.racers[0], 3); }
    return;
  }
  if (state.phase === 'countdown') {
    const before = Math.ceil(state.countdown);
    state.countdown = Math.max(0, state.countdown - STEP);
    const after = Math.ceil(state.countdown);
    for (const r of state.racers) {
      const input = r.human ? inputs[r.id] : undefined;
      // CPUs floor it on cue: hard ones as "2" fades, normal ones a beat later, easy ones at GO.
      const gas = r.human ? (input?.throttle ?? 0) > 0.5 : state.countdown < CPU_START[state.difficulty];
      // Rocket start: press the throttle as "2" fades and hold through GO.
      if (gas && r.startCharge < 0) r.startCharge = state.countdown;
      if (!gas) r.startCharge = -1;
    }
    if (after !== before && after > 0) emit(state, 'count', state.racers[0], after);
    if (state.countdown <= 0) {
      state.phase = 'race';
      emit(state, 'go', state.racers[0]);
      for (const r of state.racers) {
        const c = r.startCharge;
        if (c >= START_PERFECT[0] && c <= START_PERFECT[1]) { giveBoost(r, 1.4, 1.5, 0.55); emit(state, 'boost', r, 2); }
        else if (c > START_GOOD && c < START_PERFECT[0]) { giveBoost(r, 0.8, 1.38, 0.35); emit(state, 'boost', r); }
        else if (c > START_BURNOUT) { r.stall = BURNOUT_SECONDS; emit(state, 'burnout', r); }
        r.lapStart = 0;
      }
    }
    return;
  }
  if (state.phase === 'done') return;
  state.raceTime += STEP;
  const rng = new Rng(state.rng);
  const leader = state.racers.find(r => r.place === 1)!;
  const humanBest = Math.max(...state.racers.filter(r => r.human).map(r => r.travel), -Infinity);
  for (const r of state.racers) {
    let input: KartInput;
    if (r.finish !== null) input = cpuInput(state, track, r, rng, true);
    else if (r.human) input = sanitize(inputs[r.id]);
    else input = cpuInput(state, track, r, rng, false);
    driveRacer(state, track, r, input, rng, leader, humanBest);
  }
  collideRacers(state);
  updateDrafts(state);
  collideObstacles(state, track);
  updateBoxes(state);
  updateCoins(state);
  updateHazards(state);
  updateShells(state, track);
  rank(state);
  finishCheck(state);
  state.rng = rng.state;
}

export function sanitize(input: KartInput | undefined): KartInput {
  if (!input) return idleInput();
  const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? clamp(v, -1, 1) : 0);
  return { throttle: n(input.throttle), steer: n(input.steer), drift: input.drift === true, item: input.item === true, respawn: input.respawn === true };
}

function driveRacer(state: KartState, track: Track, r: Racer, input: KartInput, rng: Rng, leader: Racer, humanBest: number): void {
  const st = stats(r);
  if (r.respawn > 0) {
    r.respawn -= STEP;
    // Back on the road: gas down as the wheels touch (or held through the wait) fires a respawn boost.
    if (r.respawn <= 0) rescue(state, track, r, input.throttle > 0.5);
    return;
  }
  if (r.roll > 0) {
    r.roll -= STEP;
    if (r.roll <= 0) {
      r.item = rollItem(rng, r.place, state.racers.length);
      r.itemCount = r.item === 'triple' ? 3 : 1;
      emit(state, 'item', r);
    }
  }
  // Timers.
  r.boost = Math.max(0, r.boost - STEP);
  r.star = Math.max(0, r.star - STEP);
  r.slow = Math.max(0, r.slow - STEP);
  r.bumpCd = Math.max(0, r.bumpCd - STEP);
  r.ghost = Math.max(0, r.ghost - STEP);
  r.stall = Math.max(0, r.stall - STEP);
  if (r.golden > 0) {
    r.golden = Math.max(0, r.golden - STEP);
    // The golden mushroom's window closed: it is used up.
    if (r.golden <= 0 && r.item === 'golden') { r.item = null; r.itemCount = 0; }
  }
  const spinning = r.spin > 0;
  r.spin = Math.max(0, r.spin - STEP);

  // Surface & bounds from the centre line of the path the racer is on
  // (switching between the main loop and a branch inside fork/merge zones).
  const near = locate(track, r.x, r.y, r.z, r.seg, r.hint, _near);
  r.seg = near.seg; r.hint = near.index; r.segS = near.s;
  r.lateral = near.lateral;
  const path = track.paths[r.seg];
  const flags = path.flags[near.index];
  const off = Math.abs(near.lateral) - path.halfWidth;
  const onIce = (flags & F_ICE) !== 0 && off < 0;
  const muddy = (flags & F_OFFROAD) !== 0 && off < 0 && inOffroad(path, near.s, near.lateral);
  r.surface = off > 0.4 || muddy ? 'grass' : onIce ? 'ice' : 'road';
  const liquid = track.liquid;
  const wasUnder = r.under;
  r.under = liquid !== null && !track.lava && r.y + 0.3 < liquid && (flags & F_UNDER) !== 0;
  if (r.under && !wasUnder) emit(state, 'splash', r, 1);

  // Target speed.
  let top = st.top;
  if (!r.human) {
    top *= CPU_SPEED[state.difficulty];
    // Gentle rubber band keeps CPUs near the humans without teleporting.
    const gap = humanBest - r.travel;
    if (Number.isFinite(gap)) top *= clamp(1 + gap * (state.difficulty === 'hard' ? 0.0011 : 0.0008), 0.9, 1.08);
  } else if (r !== leader) {
    top *= 1 + clamp((leader.travel - r.travel) * 0.00015, 0, 0.04);
  }
  // Off-road; the deep mud of a boost-only shortcut is far worse without a mushroom or star.
  if (r.surface === 'grass' && r.boost <= 0 && r.star <= 0) top *= muddy && path.boost ? 0.32 : 0.55;
  if (r.slow > 0) top *= 0.7;
  if (r.star > 0) top *= 1.15;
  if (r.under) top *= 0.93;
  top *= 1 + Math.min(r.coins, MAX_COINS) * COIN_SPEED;
  if (r.boost > 0) top = Math.max(top, st.top * r.boostPower);
  if (r.drift !== 0) top *= 0.97;

  // A burnout spins the wheels in place for a moment.
  const throttle = spinning || r.stall > 0 ? 0 : input.throttle;
  if (r.glide) r.speed = Math.max(r.speed, st.top * 0.95);
  else if (r.boost > 0) r.speed = approach(r.speed, top, 6, STEP);
  // Past top speed (a boost just ended) the extra bleeds off gently: boosts carry.
  else if (throttle > 0.05) r.speed += (r.speed < top ? st.accel * throttle * (r.under ? 0.8 : 1) * (1 - Math.max(0, r.speed) / (top * 1.25)) : -(r.speed - top) * 1.8) * STEP;
  else if (throttle < -0.05) r.speed -= (r.speed > 0 ? 34 : 10) * -throttle * STEP;
  else r.speed = approach(r.speed, 0, r.under ? 1.2 : 0.7, STEP);
  r.speed = clamp(r.speed, -9, 48);
  if (spinning) r.speed = approach(r.speed, 0, 2.2, STEP);

  // Steering: smoothed input; full lock reduces with speed.
  r.steer = approach(r.steer, spinning ? 0 : input.steer, 9, STEP);
  const speedRatio = clamp(Math.abs(r.speed) / st.top, 0, 1.4);
  const grip = clamp(Math.abs(r.speed) / 6, 0, 1);
  const driftPressed = input.drift && !r.prevDrift;
  r.prevDrift = input.drift;

  if (r.grounded) {
    // Drift: hop + commit to a direction while steering at speed.
    if (driftPressed && r.drift === 0 && !spinning) {
      r.vy = 3.2; r.grounded = false; r.air = 0;
      r.trickReady = false;
      if (Math.abs(input.steer) > 0.25 && r.speed > st.top * 0.45) r.drift = Math.sign(input.steer) as -1 | 1;
    }
    if (r.drift !== 0 && (!input.drift || r.speed < st.top * 0.3 || spinning)) releaseDrift(state, r);
  } else if (driftPressed && r.trickReady && r.air > 0.12) {
    r.trick = 0.45; r.trickReady = false; r.trickDone = true; emit(state, 'trick', r);
  } else if (r.drift === 0 && input.drift && Math.abs(input.steer) > 0.25 && r.speed > st.top * 0.45 && r.air < 0.4 && !r.trickReady && !r.glide) {
    r.drift = Math.sign(input.steer);
  }

  let yawRate: number;
  if (r.drift !== 0) {
    // Steering into the drift tightens it and charges faster.
    const into = r.steer * r.drift;
    yawRate = -r.drift * (1.25 + into * 0.6) * st.turn / 1.9;
    if (r.grounded) r.driftCharge += STEP * (1 + Math.max(0, into) * 0.7);
    const level = DRIFT_LEVELS.filter(t => r.driftCharge >= t).length;
    if (level > r.driftLevel) r.driftLevel = level;
  } else {
    yawRate = -r.steer * st.turn * grip * (1.45 - 0.45 * clamp(speedRatio, 0, 1)) * Math.sign(r.speed || 1);
  }
  if (r.surface === 'ice') yawRate *= 0.8;
  // Banked turns hold the line a little better; the glider steers gently.
  if (path.bank[near.index] !== 0 && r.grounded) yawRate *= 1.12;
  if (r.glide) yawRate *= 0.7;
  r.yaw = wrapAngle(r.yaw + yawRate * STEP);

  // Velocity follows heading according to grip (low on ice, partial when drifting).
  const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw);
  const gripRate = r.glide ? 2.2 : !r.grounded ? 0.4 : r.surface === 'ice' ? 1.6 : r.drift !== 0 ? 5.5 : 11;
  r.vx = approach(r.vx, fx * r.speed, gripRate, STEP);
  r.vz = approach(r.vz, fz * r.speed, gripRate, STEP);
  const prevS = near.s;
  r.x += r.vx * STEP;
  r.z += r.vz * STEP;

  // Walls (city / canyon), bridge rails and tunnels bounce racers back onto the road.
  if (path.edge === 'wall' || (flags & F_RAILS) !== 0) {
    const limit = path.halfWidth + 1.3;
    const lat = nearest(path, r.x, r.z, r.hint, _near, 4).lateral;
    if (Math.abs(lat) > limit) {
      const push = Math.abs(lat) - limit, sign = Math.sign(lat);
      // Left-hand normal is (tz, -tx).
      r.x -= _near.tz * push * sign; r.z += _near.tx * push * sign;
      const vn = r.vx * _near.tz - r.vz * _near.tx;
      if (vn * sign > 0) { r.vx -= _near.tz * vn * 1.4; r.vz += _near.tx * vn * 1.4; }
      if (Math.abs(r.speed) > 8 && r.bumpCd <= 0) { r.speed *= 0.86; r.bumpCd = 0.4; emit(state, 'bump', r); }
      r.yaw = wrapAngle(r.yaw + sign * 0.02);
    }
  } else if (off > path.outOfBounds && r.grounded && path.drop[near.index] < 0.5) {
    r.respawn = RESCUE_SECONDS; r.speed = 0; r.vx = r.vz = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0;
    emit(state, 'respawn', r);
    return;
  }
  if (input.respawn && r.grounded && Math.abs(r.speed) < 3 && r.spin <= 0) {
    r.respawn = 0.6; emit(state, 'respawn', r); return;
  }

  // Height: ground following, ramps, launches, drawbridges, half-pipe lips, flight, the glider.
  nearest(path, r.x, r.z, r.hint, _near, 4);
  const ground = track.ground(r.x, r.z, _near, state.raceTime);
  const onRoad = Math.abs(_near.lateral) < path.halfWidth;
  for (const at of path.features.ramps) {
    const rs = at * path.length;
    const crossed = prevS < rs && _near.s >= rs && _near.s - prevS < 20;
    if (crossed && r.grounded && onRoad && r.speed > 10) {
      r.vy = 7.5 + Math.min(r.speed, 40) * 0.22; r.grounded = false; r.air = 0; r.trickReady = true;
      emit(state, 'boost', r, 0);
    }
  }
  for (const l of path.features.launches) {
    const ls = l.at * path.length;
    const crossed = prevS < ls && _near.s >= ls && _near.s - prevS < 20;
    // Mid drift-hop counts: anyone on the ramp surface at the lip takes off.
    if (crossed && (r.grounded || r.y < ground + 1.6) && onRoad && r.speed > 6) {
      // Launch ramps double as dash ramps so a racer at speed always clears the gap.
      r.speed = Math.max(r.speed, st.top * (l.glide ? 1 : 1.05));
      // The ramp throws the kart straight down the road at that speed.
      r.vx = _near.tx * r.speed; r.vz = _near.tz * r.speed;
      r.vy = l.power; r.grounded = false; r.air = 0; r.trickReady = true;
      r.drift = 0; r.driftCharge = 0; r.driftLevel = 0;
      if (l.glide) { r.glide = true; emit(state, 'glide', r); } else emit(state, 'boost', r, 0);
    }
  }
  // A raised drawbridge throws a racer cresting its first leaf over the opening.
  for (const d of path.features.drawbridges) {
    const lift = drawLift(d, state.raceTime);
    if (lift < 0.3) continue;
    const a = d.from * path.length, half = (d.to - d.from) * path.length / 2, th = DRAW_ANGLE * lift, tip = a + half * Math.cos(th);
    const crossed = prevS < tip && _near.s >= tip && _near.s - prevS < 20;
    if (crossed && (r.grounded || r.y < ground + 1.6) && r.speed > 4) {
      r.vy = r.speed * Math.sin(th) + 1.5; r.grounded = false; r.air = 0; r.trickReady = true;
      r.drift = 0; r.driftCharge = 0; r.driftLevel = 0;
      emit(state, 'boost', r, 0);
    }
  }
  // Riding high on a half-pipe wall off its end lip: a trick jump.
  for (const [, to] of path.features.halfpipes) {
    const ps = to * path.length;
    const crossed = prevS < ps && _near.s >= ps && _near.s - prevS < 20;
    if (crossed && r.grounded && r.y > ground + 1.2 && r.speed > 8) {
      r.vy = 6 + r.speed * 0.12; r.grounded = false; r.air = 0; r.trickReady = true;
      emit(state, 'boost', r, 0);
    }
  }
  for (const pad of path.features.boostPads) {
    const ps = pad.at * path.length;
    if (r.grounded && Math.abs(_near.s - ps) < 2.2 && Math.abs(_near.lateral - pad.lane * path.halfWidth) < 2.4 && r.boost < PAD_SECONDS * 0.5) {
      giveBoost(r, PAD_SECONDS, PAD_POWER, 0.4); emit(state, 'boost', r);
    }
  }
  if (r.grounded) {
    if (ground < r.y - 1.2) { r.grounded = false; r.air = 0; r.vy = 0; } // drove off a crest, a ledge or into a chasm
    else r.y = ground;
  }
  if (!r.grounded) {
    r.air += STEP;
    r.vy -= (r.glide ? GLIDE_GRAVITY : r.under ? WATER_GRAVITY : GRAVITY) * STEP;
    if (r.glide) r.vy = Math.max(r.vy, -GLIDE_SINK);
    r.y += r.vy * STEP;
    if (r.trick > 0) r.trick = Math.max(0, r.trick - STEP);
    if (r.y <= ground && r.vy <= 0) {
      r.y = ground; r.grounded = true; r.vy = 0;
      if (r.air > 0.5) emit(state, 'land', r);
      if (r.trickDone) { giveBoost(r, TRICK_SECONDS, TRICK_POWER, 0.3); emit(state, 'boost', r); }
      r.trick = 0; r.trickReady = false; r.trickDone = false; r.glide = false;
    }
  }
  // Fell in: lava always, water unless the road runs under the surface here; or down a chasm.
  const diveable = !track.lava && (path.flags[_near.index] & F_UNDER) !== 0 && Math.abs(_near.lateral) - path.halfWidth < Math.max(2, path.outOfBounds);
  if (liquid !== null && r.y + 0.3 < liquid && !diveable) {
    r.respawn = RESCUE_SECONDS; r.speed = 0; r.vx = r.vz = 0; r.vy = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0; r.glide = false;
    emit(state, 'splash', r, track.lava ? 2 : 0);
    return;
  }
  if (!r.grounded && r.y < _near.y - FALL_DEPTH) {
    r.respawn = RESCUE_SECONDS; r.speed = 0; r.vx = r.vz = 0; r.vy = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0; r.glide = false;
    emit(state, 'respawn', r);
    return;
  }
  if (r.grounded && Math.abs(_near.lateral) < path.halfWidth && !inLaunchZone(path, _near.s)) { r.safeSeg = r.seg; r.safeS = _near.s; }

  // Progress / laps (main-loop distance, mapped from branches).
  const s = progressOf(track, r.seg, _near.s);
  r.segS = _near.s;
  let ds = s - r.lastS;
  if (ds > track.length / 2) ds -= track.length;
  if (ds < -track.length / 2) ds += track.length;
  if (Math.abs(ds) < 25) r.travel += ds;
  r.lastS = s;
  r.wrong = ds < -0.02 && r.speed > 3 ? r.wrong + STEP : Math.max(0, r.wrong - STEP * 2);
  if (r.finish === null) {
    while (r.travel >= (r.lap + 1) * track.length && r.lap < state.laps) {
      r.lap++;
      r.lapTimes.push(state.raceTime - r.lapStart);
      r.lapStart = state.raceTime;
      if (!r.human) r.cpuRoute = pickRoute(track, state.difficulty, rng);
      if (r.lap >= state.laps) {
        r.finish = state.raceTime;
        state.firstFinish ??= state.raceTime;
        emit(state, 'finish', r, r.finish);
      } else emit(state, r.lap === state.laps - 1 ? 'final' : 'lap', r, r.lap);
    }
  }

  // Items (holding the brake throws a green shell or a bomb backwards).
  const itemPressed = input.item && !r.prevItem;
  r.prevItem = input.item;
  if (itemPressed && r.item && r.roll <= 0 && !spinning) useItem(state, track, r, input.throttle < -0.5);
}

function releaseDrift(state: KartState, r: Racer): void {
  if (r.driftLevel > 0) {
    const tier = r.driftLevel - 1;
    giveBoost(r, TURBO_SECONDS[tier], TURBO_POWER[tier], 0.2 + tier * 0.1);
    emit(state, 'turbo', r, r.driftLevel);
  }
  r.drift = 0; r.driftCharge = 0; r.driftLevel = 0;
}

function rescue(state: KartState, track: Track, r: Racer, revving: boolean): void {
  // Back to the centre just behind the last spot the racer was on the road,
  // stepping back further if that lands in a chasm or a launch run-up.
  let seg = r.safeSeg, s = r.safeS - 3;
  let path = track.paths[seg] ?? track;
  for (let k = 0; k < 80; k++) {
    if (seg !== 0 && s < 0) { s += path.from; seg = 0; path = track; }
    if (!inLaunchZone(path, s)) break;
    s -= 4;
  }
  if (seg === 0) s = ((s % track.length) + track.length) % track.length;
  pointAt(path, s, 0, _pt);
  r.x = _pt[0]; r.y = _pt[1]; r.z = _pt[2]; r.yaw = _pt[3];
  // Pointed down the road and briefly untouchable (items, bananas, obstacles).
  r.vx = r.vz = r.vy = 0; r.speed = 0;
  r.grounded = true; r.spin = 0; r.glide = false; r.under = false; r.stall = 0; r.ghost = GHOST_SECONDS;
  r.coins = Math.max(0, r.coins - COIN_LOSS);
  r.seg = seg; r.segS = s; r.hint = sampleAt(path, s);
  r.y = track.ground(r.x, r.z, nearest(path, r.x, r.z, r.hint, _near, 4), state.raceTime);
  // Progress follows the racer back: no lap credit for distance flown or fallen.
  const prog = progressOf(track, seg, s);
  r.travel += circ(prog - r.lastS, track.length);
  r.lastS = prog;
  emit(state, 'land', r);
  if (revving) { giveBoost(r, RESPAWN_BOOST, 1.3, 0.5); emit(state, 'boost', r); }
}

export function spinOut(state: KartState, r: Racer, seconds = 1.0): boolean {
  if (r.star > 0 || r.ghost > 0 || r.respawn > 0 || r.finish !== null && r.speed < 1) return false;
  r.spin = Math.max(r.spin, seconds);
  r.boost = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0; r.draft = 0;
  r.coins = Math.max(0, r.coins - COIN_LOSS);
  emit(state, 'spin', r);
  return true;
}

/** The best-placed unfinished racer other than `r` (a blue shell's target), or undefined. */
function leaderBesides(state: KartState, r: Racer): Racer | undefined {
  let best: Racer | undefined;
  for (const o of state.racers) if (o !== r && o.finish === null && (!best || o.place < best.place)) best = o;
  return best;
}

function launch(state: KartState, r: Racer, kind: ShellKind, back: boolean, target: number): void {
  const yaw = back ? wrapAngle(r.yaw + Math.PI) : r.yaw, fx = Math.sin(yaw), fz = Math.cos(yaw);
  const spec = SHELL[kind];
  // Thrown ahead it leaves at the shell's speed (or the kart's, if faster); a bomb is lobbed.
  const speed = kind === 'bomb' ? spec.speed + (back ? -12 : Math.max(0, r.speed) * 0.4) : Math.max(spec.speed, back ? 0 : r.speed + 8);
  state.shells.push({
    id: state.nextId++, kind, x: r.x + fx * 2.8, y: r.y + 0.6, z: r.z + fz * 2.8, yaw, target, owner: r.id,
    life: spec.life, hint: r.hint, seg: r.seg, vy: kind === 'bomb' ? (back ? 4 : 8) : 0, speed,
  });
  if (state.shells.length > MAX_SHELLS) state.shells.shift();
  emit(state, 'throw', r, SHELL_KINDS.indexOf(kind));
}

function useItem(state: KartState, track: Track, r: Racer, back: boolean): void {
  const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw);
  switch (r.item) {
    case 'mushroom':
    case 'triple':
      giveBoost(r, MUSHROOM_SECONDS, MUSHROOM_POWER, 0.45); emit(state, 'boost', r, 1);
      break;
    case 'golden':
      // The first press opens the window; every press inside it is another mushroom. Not used up per press.
      if (r.golden <= 0) r.golden = GOLDEN_SECONDS;
      giveBoost(r, MUSHROOM_SECONDS, MUSHROOM_POWER, 0.45); emit(state, 'boost', r, 3);
      return;
    case 'banana': {
      // Dropped just behind the kart.
      state.hazards.push({ id: state.nextId++, x: r.x - fx * 2.6, y: r.y, z: r.z - fz * 2.6, owner: r.id, age: 0 });
      if (state.hazards.length > MAX_HAZARDS) state.hazards.shift();
      emit(state, 'drop', r);
      break;
    }
    case 'green': launch(state, r, 'green', back, -1); break;
    case 'red': {
      const ahead = state.racers.find(o => o.place === r.place - 1 && o.finish === null);
      launch(state, r, 'red', false, ahead ? ahead.id : -1);
      break;
    }
    case 'blue': launch(state, r, 'blue', false, leaderBesides(state, r)?.id ?? -1); break;
    case 'bomb': launch(state, r, 'bomb', back, -1); break;
    case 'star':
      r.star = 7; r.spin = 0; emit(state, 'star', r);
      break;
    case 'bolt': {
      // Lightning strikes every opponent, never the user (who keeps every bit of speed): a short spin, then
      // shrunk and slowed — longest for the leaders — and whatever they were holding is lost. A star or a
      // post-rescue moment shrugs it off.
      const n = state.racers.length;
      let hits = 0;
      for (const o of state.racers) {
        if (o === r || o.finish !== null || o.respawn > 0 || o.star > 0 || o.ghost > 0) continue;
        spinOut(state, o, 0.7);
        o.slow = Math.max(o.slow, 2 + 2.5 * (1 - (o.place - 1) / Math.max(1, n - 1)));
        if (o.roll <= 0) { o.item = null; o.itemCount = 0; o.golden = 0; }
        emit(state, 'zap', o);
        hits++;
      }
      emit(state, 'bolt', r, hits);
      break;
    }
  }
  r.itemCount--;
  if (r.itemCount <= 0) { r.item = null; r.itemCount = 0; }
}

/**
 * Slipstream: a racer tucked in behind another at speed (3–20 m back, nearly
 * in line, heading the same way) charges a draft; held long enough, it fires a
 * boost past them.
 */
function updateDrafts(state: KartState): void {
  for (const r of state.racers) {
    if (r.respawn > 0 || !r.grounded || r.speed < 18 || r.boost > 0 || r.finish !== null) { r.draft = Math.max(0, r.draft - STEP * 2); continue; }
    const fx = Math.sin(r.yaw), fz = Math.cos(r.yaw);
    let tucked = false;
    for (const o of state.racers) {
      if (o === r || o.respawn > 0 || o.speed < 15) continue;
      const dx = o.x - r.x, dz = o.z - r.z;
      const ahead = dx * fx + dz * fz, side = Math.abs(dx * fz - dz * fx);
      if (ahead > 3 && ahead < 20 && side < 2.4 && Math.abs(o.y - r.y) < 2 && Math.abs(wrapAngle(o.yaw - r.yaw)) < 0.35) { tucked = true; break; }
    }
    if (!tucked) { r.draft = Math.max(0, r.draft - STEP * 2); continue; }
    r.draft += STEP;
    if (r.draft >= DRAFT_SECONDS) { r.draft = 0; giveBoost(r, DRAFT_BOOST, DRAFT_POWER, 0.3); emit(state, 'draft', r); }
  }
}

function collideRacers(state: KartState): void {
  const rs = state.racers;
  for (let i = 0; i < rs.length; i++) for (let j = i + 1; j < rs.length; j++) {
    const a = rs[i], b = rs[j];
    if (a.respawn > 0 || b.respawn > 0) continue;
    const dx = b.x - a.x, dz = b.z - a.z, d2 = dx * dx + dz * dz;
    if (d2 > 2.3 * 2.3 || d2 < 1e-6 || Math.abs(a.y - b.y) > 1.6) continue;
    const d = Math.sqrt(d2), nx = dx / d, nz = dz / d, overlap = 2.3 - d;
    const wa = stats(a).weight + (a.star > 0 ? 3 : 0), wb = stats(b).weight + (b.star > 0 ? 3 : 0);
    const ta = wb / (wa + wb), tb = wa / (wa + wb);
    a.x -= nx * overlap * ta; a.z -= nz * overlap * ta;
    b.x += nx * overlap * tb; b.z += nz * overlap * tb;
    const rel = (b.vx - a.vx) * nx + (b.vz - a.vz) * nz;
    if (rel < 0) {
      const impulse = -rel * 1.3;
      a.vx -= nx * impulse * ta; a.vz -= nz * impulse * ta;
      b.vx += nx * impulse * tb; b.vz += nz * impulse * tb;
      if (-rel > 3) emit(state, 'bump', a);
    }
    if (a.star > 0 && b.star <= 0) spinOut(state, b);
    if (b.star > 0 && a.star <= 0) spinOut(state, a);
  }
}

/** Rolling boulders and crushers (poses are a pure function of race time). */
function collideObstacles(state: KartState, track: Track): void {
  for (const o of track.obstacles) {
    obstacleAt(track, o, state.raceTime, _ob);
    for (const r of state.racers) {
      if (r.respawn > 0 || r.star > 0 || r.ghost > 0 || r.spin > 0.3) continue;
      const dx = r.x - _ob[0], dz = r.z - _ob[2];
      if (o.kind === 'roller') {
        if (dx * dx + dz * dz > (ROLLER_RADIUS + 1.1) ** 2 || Math.abs(r.y + 0.6 - _ob[1]) > ROLLER_RADIUS + 1) continue;
        const d = Math.hypot(dx, dz) || 1;
        r.x = _ob[0] + dx / d * (ROLLER_RADIUS + 1.15); r.z = _ob[2] + dz / d * (ROLLER_RADIUS + 1.15);
        r.speed *= 0.5;
        if (spinOut(state, r, 0.9)) emit(state, 'bump', r);
      } else {
        // A crusher only hurts while it is (nearly) down.
        if (_ob[3] > 1.6 || dx * dx + dz * dz > (CRUSHER_HALF + 0.9) ** 2 || Math.abs(r.y - _ob[1]) > 3) continue;
        r.speed = Math.min(r.speed, 4);
        if (spinOut(state, r, 1.4)) emit(state, 'crush', r);
      }
    }
  }
}

function updateBoxes(state: KartState): void {
  for (const box of state.boxes) {
    if (box.timer > 0) { box.timer = Math.max(0, box.timer - STEP); continue; }
    for (const r of state.racers) {
      if (r.respawn > 0) continue;
      const dx = r.x - box.x, dz = r.z - box.z;
      if (dx * dx + dz * dz < 2.4 * 2.4 && Math.abs(r.y + 0.6 - box.y) < 2.2) {
        box.timer = 2;
        emit(state, 'box', r);
        if (!r.item && r.roll <= 0 && r.finish === null) r.roll = 1.1;
        break;
      }
    }
  }
}

/** Coins: driving through one adds it to the racer's purse (capped); it reappears later. No event per coin (the HUD counts). */
function updateCoins(state: KartState): void {
  for (const c of state.coins) {
    if (c.timer > 0) { c.timer = Math.max(0, c.timer - STEP); continue; }
    for (const r of state.racers) {
      if (r.respawn > 0 || r.finish !== null) continue;
      const dx = r.x - c.x, dz = r.z - c.z;
      if (dx * dx + dz * dz < 1.9 * 1.9 && Math.abs(r.y + 0.6 - c.y) < 2.2) {
        c.timer = COIN_RESPAWN;
        r.coins = Math.min(MAX_COINS, r.coins + 1);
        break;
      }
    }
  }
}

function updateHazards(state: KartState): void {
  for (let h = state.hazards.length - 1; h >= 0; h--) {
    const hz = state.hazards[h];
    hz.age += STEP;
    for (const r of state.racers) {
      // Its owner drives clear of a banana it just dropped.
      if (r.id === hz.owner && hz.age < BANANA_GRACE) continue;
      const dx = r.x - hz.x, dz = r.z - hz.z;
      if (dx * dx + dz * dz < 1.7 * 1.7 && Math.abs(r.y - hz.y) < 1.5 && r.respawn <= 0) {
        spinOut(state, r, 1.0);
        state.hazards.splice(h, 1);
        break;
      }
    }
  }
}

/** An explosion at a shell: everyone but its owner inside `radius` spins out. */
function blast(state: KartState, sh: Shell, radius: number): void {
  emit(state, 'explode', { id: sh.owner, x: sh.x, y: sh.y, z: sh.z }, SHELL_KINDS.indexOf(sh.kind));
  for (const o of state.racers) if (o.id !== sh.owner && Math.hypot(o.x - sh.x, o.z - sh.z) < radius && Math.abs(o.y - sh.y) < 4) spinOut(state, o, 1.3);
}

/**
 * Shells and bombs. Green: straight on, bouncing off walls and rails. Red:
 * down the road, homing in on its target once close. Blue: high over the
 * pack along the leader's route, then down onto them in a blast. Bomb: lobbed,
 * rolls to a stop, goes off on contact or when its fuse runs out. None of them
 * touches its owner (a green shell only after GREEN_GRACE).
 */
function updateShells(state: KartState, track: Track): void {
  for (let i = state.shells.length - 1; i >= 0; i--) {
    const sh = state.shells[i];
    sh.life -= STEP;
    const near = locate(track, sh.x, sh.y, sh.z, sh.seg, sh.hint, _near);
    sh.hint = near.index; sh.seg = near.seg;
    const path = track.paths[near.seg];
    // The blue shell re-targets whoever leads (never its owner); a red one keeps its pick.
    if (sh.kind === 'blue') { const owner = state.racers[sh.owner]; sh.target = owner ? leaderBesides(state, owner)?.id ?? -1 : -1; }
    const target = sh.target >= 0 ? state.racers[sh.target] : undefined;
    const ground = Math.max(track.ground(sh.x, sh.z, near, state.raceTime), near.y - 0.5);
    // Spent green and red shells just vanish; a blue one or a bomb goes off where it is.
    let gone = sh.life <= 0 && (sh.kind === 'green' || sh.kind === 'red');
    if (sh.kind === 'green' || sh.kind === 'bomb') {
      if (sh.kind === 'bomb') {
        // Lobbed: arcs, lands, rolls to a stop.
        sh.vy -= GRAVITY * STEP; sh.y += sh.vy * STEP;
        if (sh.y <= ground + 0.5) { sh.y = ground + 0.5; sh.vy = 0; sh.speed = approach(sh.speed, 0, 4, STEP); }
      } else sh.y = approach(sh.y, ground + 0.7, 10, STEP);
      sh.x += Math.sin(sh.yaw) * sh.speed * STEP;
      sh.z += Math.cos(sh.yaw) * sh.speed * STEP;
      const edge = Math.abs(near.lateral) - path.halfWidth;
      if (path.edge === 'wall' || (path.flags[near.index] & F_RAILS) !== 0) {
        if (edge > 0.9 && near.lateral * (Math.sin(sh.yaw) * near.tz - Math.cos(sh.yaw) * near.tx) > 0) {
          // Bounce: mirror the heading about the road's direction.
          sh.yaw = wrapAngle(2 * Math.atan2(near.tx, near.tz) - sh.yaw);
        }
      } else if (edge > path.outOfBounds) gone = true; // off into the scenery
    } else {
      let tx: number, tz: number;
      const close = sh.kind === 'blue' ? 40 : 45;
      if (target && target.respawn <= 0 && Math.hypot(target.x - sh.x, target.z - sh.z) < close) { tx = target.x; tz = target.z; }
      else {
        // Follow the road (onto the target's route when it is on a branch).
        advance(track, near.seg, near.s, sh.kind === 'blue' ? 24 : 16, target && target.seg > 0 ? 1 << (target.seg - 1) : 0, _spot);
        pointAt(track.paths[_spot.seg], _spot.s, 0, _pt); tx = _pt[0]; tz = _pt[2];
      }
      const want = Math.atan2(tx - sh.x, tz - sh.z), turn = sh.kind === 'blue' ? 7 : 4;
      sh.yaw = wrapAngle(sh.yaw + clamp(wrapAngle(want - sh.yaw), -turn * STEP, turn * STEP));
      sh.x += Math.sin(sh.yaw) * sh.speed * STEP;
      sh.z += Math.cos(sh.yaw) * sh.speed * STEP;
      // Red shells skim the road (over gaps and water at road height); the blue one flies high until it dives.
      const dive = sh.kind === 'blue' && target ? clamp((Math.hypot(target.x - sh.x, target.z - sh.z) - 3) / 14, 0, 1) : 1;
      sh.y = approach(sh.y, ground + 0.7 + (sh.kind === 'blue' ? 6 * dive : 0), 10, STEP);
    }
    if (!gone && sh.kind === 'blue') {
      // Only its target stops it: it passes over everyone else.
      if (target && target.respawn <= 0 && Math.hypot(target.x - sh.x, target.z - sh.z) < 3) { blast(state, sh, SHELL[sh.kind].blast); gone = true; }
      else if (sh.life <= 0) { blast(state, sh, SHELL[sh.kind].blast); gone = true; }
    } else if (!gone) {
      for (const r of state.racers) {
        if (r.respawn > 0 || r.id === sh.owner && (sh.kind !== 'green' || SHELL.green.life - sh.life < GREEN_GRACE)) continue;
        const dx = r.x - sh.x, dz = r.z - sh.z;
        if (dx * dx + dz * dz < 2.2 * 2.2 && Math.abs(r.y + 0.6 - sh.y) < 2.2) {
          if (sh.kind === 'bomb') blast(state, sh, SHELL.bomb.blast);
          else { emit(state, 'explode', { id: r.id, x: sh.x, y: sh.y, z: sh.z }, SHELL_KINDS.indexOf(sh.kind)); spinOut(state, r, 1.0); }
          gone = true;
          break;
        }
      }
      // A bomb's fuse runs out where it lies.
      if (!gone && sh.kind === 'bomb' && sh.life <= 0) { blast(state, sh, SHELL.bomb.blast); gone = true; }
    }
    if (!gone && sh.kind !== 'blue') for (let h = state.hazards.length - 1; h >= 0; h--) {
      const hz = state.hazards[h];
      if (Math.hypot(hz.x - sh.x, hz.z - sh.z) < 1.6) {
        state.hazards.splice(h, 1); gone = true;
        if (sh.kind === 'bomb') blast(state, sh, SHELL.bomb.blast);
        else emit(state, 'explode', { id: sh.owner, x: sh.x, y: sh.y, z: sh.z }, SHELL_KINDS.indexOf(sh.kind));
        break;
      }
    }
    if (gone) state.shells.splice(i, 1);
  }
}

function finishCheck(state: KartState): void {
  const track = trackFor(state.track);
  const humans = state.racers.filter(r => r.human);
  const allHumansDone = humans.length > 0 && humans.every(r => r.finish !== null);
  const timeout = state.firstFinish !== null && state.raceTime - state.firstFinish > 45;
  if (!allHumansDone && !timeout && !state.racers.every(r => r.finish !== null)) return;
  state.endTimer += STEP;
  if (state.endTimer < 3.5 && !state.racers.every(r => r.finish !== null)) return;
  // Project unfinished racers from their average pace so everybody gets a time.
  for (const r of state.racers) if (r.finish === null) {
    const pace = Math.max(10, r.travel / Math.max(1, state.raceTime));
    r.finish = state.raceTime + Math.max(0, state.laps * track.length - r.travel) / pace;
  }
  rank(state);
  state.phase = 'done';
}

/** Final standings for the results screen and the relay `finish` payload. */
export function standings(state: KartState): { id: number; racer: RacerId; name: string; human: boolean; slot: number; place: number; time: number; best: number }[] {
  return [...state.racers].sort((a, b) => a.place - b.place).map(r => ({
    id: r.id, racer: r.racer, name: r.name, human: r.human, slot: r.slot, place: r.place,
    time: r.finish ?? 0, best: r.lapTimes.length ? Math.min(...r.lapTimes) : 0,
  }));
}
