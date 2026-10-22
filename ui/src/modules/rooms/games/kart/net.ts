/**
 * Compact relay payloads for Kart Grand Prix. The host sends snapshots (~2.5 KB
 * for 8 racers) at 20 Hz; guests send inputs (~60 B) at 30 Hz. Everything is
 * validated before it reaches the renderer or the simulation.
 */
import { finite, nums, r2, r3 } from '../common/math.ts';
import { ITEM_KINDS, type ItemKind } from './items.ts';
import { RACER_IDS, type RacerId } from './roster.ts';
import { TRACK_IDS, trackFor } from './tracks.ts';
import { itemBoxes, MAX_COINS, MAX_HAZARDS, MAX_RACERS, MAX_SHELLS, SHELL, SHELL_KINDS, trackCoins, type KartEvent, type KartEventType, type KartInput, type KartState, type Racer } from './sim.ts';

const PHASES = ['intro', 'countdown', 'race', 'done'] as const;
const EVENTS: KartEventType[] = ['box', 'item', 'boost', 'turbo', 'spin', 'explode', 'bump', 'lap', 'final', 'finish', 'bolt', 'drop', 'throw', 'star', 'respawn', 'trick', 'land', 'go', 'count', 'splash', 'glide', 'crush', 'zap', 'draft', 'burnout'];
const SURFACES = ['road', 'grass', 'ice'] as const;
const DIFFICULTIES = ['easy', 'normal', 'hard'] as const;

/**
 * v4: racer rows end with the coin purse, then the golden-mushroom window,
 * post-rescue invulnerability, slipstream charge and burnout timer; shell rows
 * end with the shell kind (green, red, blue, bomb). `cn` lists the coins
 * currently collected (indices into the track's coin spots). Drawbridges,
 * rollers and crushers need nothing: guests pose them from the race clock `t`.
 */
/** Bump when the snapshot layout changes: mixed builds then refuse each other's
 *  snapshots instead of mis-decoding them. */
export const KART_SNAPSHOT_VERSION = 4;
export interface KartSnapshot {
  v: typeof KART_SNAPSHOT_VERSION; tr: string; d: number; p: number; c: number; cd: number; t: number;
  /** Per racer: see RACER_FIELDS. */
  r: number[][];
  /** Static per racer: [racer index, human, slot]. */
  q: number[][];
  lt: number[][];
  b: number[]; cn: number[]; h: number[][]; s: number[][]; e: number[][];
}
const flags = (r: Racer) => (r.grounded ? 1 : 0) | (r.human ? 2 : 0) | (r.trickReady ? 4 : 0) | (r.glide ? 8 : 0) | (r.under ? 16 : 0);
/** Values per racer row, per shell row. */
export const RACER_FIELDS = 36;
export const SHELL_FIELDS = 8;

export function encodeSnapshot(state: KartState, events: KartEvent[]): KartSnapshot {
  return {
    v: KART_SNAPSHOT_VERSION, tr: state.track, d: DIFFICULTIES.indexOf(state.difficulty), p: PHASES.indexOf(state.phase),
    c: r2(state.clock), cd: r2(state.countdown), t: r3(state.raceTime),
    r: state.racers.map(r => [
      r2(r.x), r2(r.y), r2(r.z), r3(r.yaw), r2(r.vx), r2(r.vz), r2(r.vy), r2(r.speed), r2(r.steer), flags(r),
      r.drift, r.driftLevel, r2(r.boost), r2(r.boostPower), r2(r.spin), r2(r.star), r2(r.slow),
      r.item ? ITEM_KINDS.indexOf(r.item) : -1, r.itemCount, r2(r.roll), r2(r.travel), r.lap, r.place,
      r.finish === null ? -1 : r3(r.finish), r2(r.respawn), r2(r.wrong), r2(r.trick), SURFACES.indexOf(r.surface), r.hint, r2(r.lateral), r.seg, r.coins,
      r2(r.golden), r2(r.ghost), r2(r.draft), r2(r.stall),
    ]),
    q: state.racers.map(r => [RACER_IDS.indexOf(r.racer), r.human ? 1 : 0, r.slot]),
    lt: state.racers.map(r => r.lapTimes.map(r3)),
    b: state.boxes.map(b => r2(b.timer)),
    cn: takenCoins(state),
    h: state.hazards.map(h => [h.id, r2(h.x), r2(h.y), r2(h.z), h.owner]),
    s: state.shells.map(s => [s.id, r2(s.x), r2(s.y), r2(s.z), r3(s.yaw), s.target, s.owner, SHELL_KINDS.indexOf(s.kind)]),
    e: events.slice(-24).map(e => [e.id, EVENTS.indexOf(e.type), e.racer, r2(e.x), r2(e.y), r2(e.z), e.value ?? 0]),
  };
}

/** Indices of the coins that are collected right now. */
function takenCoins(state: KartState): number[] {
  const out: number[] = [];
  for (let i = 0; i < state.coins.length; i++) if (state.coins[i].timer > 0) out.push(i);
  return out;
}


/** Rebuilds a render/HUD state from a snapshot; null when anything is malformed. */
export function decodeSnapshot(data: unknown, names: (slot: number, racer: RacerId) => string): KartState | null {
  if (!data || typeof data !== 'object') return null;
  const s = data as Partial<KartSnapshot>;
  if (s.v !== KART_SNAPSHOT_VERSION || !TRACK_IDS.includes(s.tr as never) || !finite(s.p) || !PHASES[s.p] || !finite(s.d) || !DIFFICULTIES[s.d]) return null;
  if (![s.c, s.cd, s.t].every(finite)) return null;
  if (!Array.isArray(s.r) || !Array.isArray(s.q) || !Array.isArray(s.lt) || s.r.length !== s.q.length || s.r.length === 0 || s.r.length > MAX_RACERS || s.lt.length !== s.r.length) return null;
  if (!Array.isArray(s.b) || !s.b.every(finite) || !Array.isArray(s.h) || s.h.length > MAX_HAZARDS || !Array.isArray(s.s) || s.s.length > MAX_SHELLS || !Array.isArray(s.e) || s.e.length > 48) return null;
  const track = trackFor(s.tr!);
  const boxes = itemBoxes(track);
  if (s.b.length !== boxes.length) return null;
  const coins = trackCoins(track);
  if (!Array.isArray(s.cn) || s.cn.length > coins.length || !s.cn.every(i => Number.isInteger(i) && i >= 0 && i < coins.length)) return null;
  for (const i of s.cn) coins[i].timer = 1;
  const racers: Racer[] = [];
  for (let i = 0; i < s.r.length; i++) {
    const row = s.r[i], q = s.q[i], lt = s.lt[i];
    if (!nums(row, RACER_FIELDS) || !nums(q, 3) || !RACER_IDS[q[0]] || !Array.isArray(lt) || lt.length > 3 || !lt.every(finite)) return null;
    const [x, y, z, yaw, vx, vz, vy, speed, steer, f, drift, driftLevel, boost, boostPower, spin, star, slow, item, itemCount, roll, travel, lap, place, finish, respawn, wrong, trick, surface, hint, lateral, seg, coinCount, golden, ghost, draft, stall] = row;
    const path = Number.isInteger(seg) ? track.paths[seg] : undefined;
    if (!path || item < -1 || item >= ITEM_KINDS.length || !SURFACES[surface] || place < 1 || place > s.r.length || hint < -1 || hint >= path.count || !Number.isInteger(coinCount) || coinCount < 0 || coinCount > MAX_COINS || q[2] < -1 || q[2] > 15 || (q[1] === 1) !== (q[2] >= 0)) return null;
    const racer = RACER_IDS[q[0]];
    racers.push({
      id: i, racer, human: q[1] === 1, slot: q[2], name: names(q[2], racer),
      x, y, z, yaw, vx, vz, vy, speed, steer, grounded: (f & 1) === 1, air: 0, trick, trickReady: (f & 4) === 4, trickDone: false,
      drift: Math.sign(drift), driftCharge: 0, driftLevel: Math.max(0, Math.min(3, Math.round(driftLevel))), boost, boostPower, spin, star, slow,
      golden: Math.max(0, golden), ghost: Math.max(0, ghost), draft: Math.max(0, draft), stall: Math.max(0, stall),
      item: item < 0 ? null : (ITEM_KINDS[item] as ItemKind), itemCount, roll, prevItem: false, prevDrift: false,
      travel, lastS: 0, hint: Math.round(hint), lateral, surface: SURFACES[surface], lap, lapStart: 0, lapTimes: lt as number[], finish: finish < 0 ? null : finish,
      seg, segS: 0, glide: (f & 8) === 8, under: (f & 16) === 16, safeSeg: seg, safeS: 0,
      place, respawn, wrong, startCharge: -1, bumpCd: 0, cpuLane: 0, cpuStuck: 0, cpuMark: travel, cpuItemDelay: 0, cpuMistake: 0, cpuRoute: 0, coins: coinCount,
    });
  }
  const events: KartEvent[] = [];
  for (const e of s.e) {
    if (!nums(e, 7) || !EVENTS[e[1]] || e[2] < 0 || e[2] >= racers.length) return null;
    events.push({ id: e[0], type: EVENTS[e[1]], racer: e[2], x: e[3], y: e[4], z: e[5], value: e[6] });
  }
  for (const h of s.h) if (!nums(h, 5)) return null;
  for (const sh of s.s) if (!nums(sh, SHELL_FIELDS) || !SHELL_KINDS[sh[7]]) return null;
  boxes.forEach((b, i) => { b.timer = s.b![i]; });
  return {
    track: track.def.id, difficulty: DIFFICULTIES[s.d!], laps: 3, phase: PHASES[s.p!], clock: s.c!, countdown: s.cd!, raceTime: s.t!,
    endTimer: 0, firstFinish: null, racers, boxes, coins,
    hazards: s.h.map(h => ({ id: h[0], x: h[1], y: h[2], z: h[3], owner: h[4], age: 9 })),
    shells: s.s.map(sh => ({ id: sh[0], kind: SHELL_KINDS[sh[7]], x: sh[1], y: sh[2], z: sh[3], yaw: sh[4], target: sh[5], owner: sh[6], life: SHELL[SHELL_KINDS[sh[7]]].life, hint: -1, seg: 0, vy: 0, speed: 0 })),
    events, eventSeq: events.length ? events[events.length - 1].id : 0, nextId: 0, rng: 0, accumulator: 0,
  };
}

export interface KartWireInput { k: number[] }
export function encodeInput(input: KartInput, racer: RacerId): KartWireInput {
  return { k: [r2(input.throttle), r2(input.steer), (input.drift ? 1 : 0) | (input.item ? 2 : 0) | (input.respawn ? 4 : 0), RACER_IDS.indexOf(racer)] };
}
export function decodeInput(data: unknown): { input: KartInput; racer: RacerId } | null {
  if (!data || typeof data !== 'object') return null;
  const k = (data as Partial<KartWireInput>).k;
  if (!nums(k, 4) || Math.abs(k[0]) > 1 || Math.abs(k[1]) > 1 || k[2] < 0 || k[2] > 7 || !RACER_IDS[k[3]]) return null;
  return { input: { throttle: k[0], steer: k[1], drift: (k[2] & 1) === 1, item: (k[2] & 2) === 2, respawn: (k[2] & 4) === 4 }, racer: RACER_IDS[k[3]] };
}
