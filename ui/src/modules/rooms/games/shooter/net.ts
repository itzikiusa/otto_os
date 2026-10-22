/**
 * Compact relay payloads for Storm Royale (all five modes). Snapshots carry
 * every fighter (team, revive progress, gunplay stance, grenades,
 * killstreaks, class, points, respawn), the storm, rockets, grenades in
 * flight, clouds / fires / airstrike markers, drones, UAVs, the infected,
 * the mode's scores / capture points / Zombies round, launch pads, supply
 * drops, felled trees and recent events; the two bulky lists are optional and
 * sent only when they changed (or periodically), never both at once, so a
 * snapshot stays under the relay's 16 KiB cap: loot (six numbers per item,
 * flat, ids as deltas) and structures (two numbers per piece: packed slot +
 * hp). Lists are flat and quantised (integer decimetres and centiradians) to
 * stay small at 60+ infected; a full floor of loot rides with at most
 * Apocalypse Royale's horde (Zombies caps its floor loot instead). Inputs are a ten-number array. Everything is validated
 * before use.
 */
import { finite, r2, r3 } from '../common/math.ts';
import { FIGHTER_IDS, SHOOTER_MAP_IDS, SHOOTER_MODE_IDS, type FighterId, type ShooterMapId, type ShooterMode } from './catalog.ts';
import { AMMO_KINDS, ATTACHMENTS, CLASSES, isConsumable, isWeapon, ITEM_IDS, ITEM_SLOTS, MATERIALS, MAX_MATS, NADE_KINDS, STREAKS, type Material, type NadeKind, type Rarity } from './weapons.ts';
import { buildGeometry, MAX_BUILDS, packBuild, unpackBuild, type Build } from './builds.ts';
import {
  DOWN_HP, emptyBot, MAX_DROPS, MAX_FIGHTERS, MAX_LOOT, MAX_NODES, MAX_PADS, MAX_ROCKETS, PICKAXE_KEY, PICKAXE_SLOT, REVIVE_TIME, SLOTS,
  type Fighter, type FighterMode, type Held, type Loot, type LootKind, type ShooterEvent, type ShooterEventType, type ShooterInput, type ShooterState, type TeamSize,
} from './sim.ts';
import { emptyMode, type CapturePoint, type ModeState } from './modes/index.ts';
import { MAX_AREAS, MAX_DRONES, MAX_FLYING } from './modes/gear.ts';
import { APOC_CAP, MAX_INFECTED, ZOMBIE_KINDS, ZOMBIE_LOOT_CAP, type Zombie, type ZombieKind } from './zombies.ts';
import { mapFor } from './map.ts';

export const SNAPSHOT_VERSION = 4;
const PHASES = ['bus', 'play', 'done'] as const;
const MODES: FighterMode[] = ['bus', 'sky', 'glide', 'ground', 'dead', 'down'];
/** Append-only: the index is the wire value. */
const LOOT_KINDS: LootKind[] = ['weapon', 'ammo', 'heal', 'shield', 'item', 'mats', 'attach', 'nade'];
/** Append-only: the index is the wire value. */
const EVENTS: ShooterEventType[] = [
  'shot', 'hit', 'kill', 'pickup', 'chest', 'explode', 'reload', 'land', 'jump', 'storm', 'win', 'glide', 'build', 'break', 'harvest', 'use', 'swing', 'edit', 'knock', 'revive', 'supply', 'launch', 'fell',
  'nade', 'smoke', 'fire', 'flash', 'streak', 'capture', 'round', 'zhit', 'zkill', 'buy', 'extract', 'spawn', 'mantle', 'slide', 'repair',
];
/** Events whose second point (x2, y2, z2) rides along: tracers. */
const WITH_END = new Set<ShooterEventType>(['shot']);
const SQUADS: TeamSize[] = [1, 2, 4];
const DIFFS = ['easy', 'normal', 'hard'] as const;
const r1 = (v: number) => Math.round(v * 10) / 10;
/** Decimetres / centiradians as integers (no decimal point on the wire) for the long lists. */
const dm = (v: number) => Math.round(v * 10);
const cr = (v: number) => Math.round(v * 100);
const SLOT_AT = 19;
const AMMO_AT = SLOT_AT + SLOTS * 3;
const TAIL_AT = AMMO_AT + AMMO_KINDS.length;
/** Gunplay, gear and mode fields after the original tail. */
export const GEAR_AT = TAIL_AT + 11;
export const FIGHTER_FIELDS = GEAR_AT + 15;
/** Flat record sizes: loot, infected, grenades, areas, drones, capture points, mode state. */
const LOOT_N = 6, ZB_N = 7, ND_N = 5, AR_N = 7, DN_N = 5, CP_N = 7, MS_N = 17;

export interface ShooterSnapshot {
  v: 4; m: string; d: number; p: number; c: number; et: number; w: number; bus: number[];
  /** Team size, winning team, game mode. */
  sq: number; wt: number; mo: number;
  /** Fighter rows; per-fighter fighter id, human flag, member slot and team. */
  f: number[][]; q: number[][]; n: string[]; st: number[]; ch: string; r: number[][];
  /** Events: id delta, type, player, target, value, flag, x, y, z (+ x2, y2, z2 for tracers). */
  e: number[][];
  /** Launch pads (x, y, z per pad), supply drops (id, x, y, z, landing height, landed) and felled trunk indices. */
  pd: number[]; dr: number[][]; fl: number[];
  /** Mode state, capture points (flat), the infected (flat), grenades, areas, drones (flat) and UAV time per team. */
  ms: number[]; cp: number[]; zb: number[]; nd: number[]; ar: number[]; dn: number[]; uv: number[];
  /** Loot (flat: id delta, code, amount, x, y, z). */
  lr: number; l?: number[];
  /** Structures: packed slot, then hp (negative while the piece is still going up). */
  br: number; b?: number[];
}

/** Loot kind, item, rarity and ammo/material in one small integer. */
const lootCode = (l: Loot) => ((LOOT_KINDS.indexOf(l.kind) * ITEM_SLOTS + (l.weapon ? ITEM_IDS.indexOf(l.weapon) : -1) + 1) * 5 + l.rarity) * 6 + (l.ammo ? AMMO_KINDS.indexOf(l.ammo) : l.mat ?? -1) + 1;
/** Held item rarity and attachment bits in one number. */
const heldCode = (w: Held) => w.rarity + (w.att | 0) * 5;

/** A flat list of records, each led by its id as a delta from the one before (ids only grow). */
function flatIds<T extends { id: number }>(list: readonly T[], row: (t: T) => number[]): number[] {
  const out: number[] = [];
  let prev = 0;
  for (const t of list) { out.push(t.id - prev, ...row(t)); prev = t.id; }
  return out;
}

export function encodeSnapshot(state: ShooterState, events: ShooterEvent[], withLoot = true, withBuilds = false): ShooterSnapshot {
  const evs = events.slice(-24);
  let prevEvent = 0;
  const obj = state.obj;
  const snap: ShooterSnapshot = {
    v: 4, m: state.map, d: DIFFS.indexOf(state.difficulty), p: PHASES.indexOf(state.phase), c: r2(state.clock), et: r2(state.endTimer), w: state.winner,
    sq: state.squad, wt: state.winTeam, mo: SHOOTER_MODE_IDS.indexOf(state.mode),
    bus: [r1(state.bus.ax), r1(state.bus.az), r1(state.bus.bx), r1(state.bus.bz), r2(state.bus.t), r2(state.bus.duration)],
    f: state.players.map(p => [
      r2(p.x), r2(p.y), r2(p.z), r3(p.yaw), r3(p.pitch), r2(p.vx), r2(p.vy), r2(p.vz), MODES.indexOf(p.mode), Math.round(p.hp), Math.round(p.shield),
      p.kills, p.place, p.current, r2(p.reload), (p.aiming ? 1 : 0) | (p.sprinting ? 2 : 0) | (p.moving ? 4 : 0) | (p.grounded ? 8 : 0) | (p.building ? 16 : 0) | (p.swimming ? 32 : 0),
      r2(p.shootT), r2(p.hurtT), r2(p.interact),
      ...p.weapons.flatMap(w => (w ? [ITEM_IDS.indexOf(w.w), heldCode(w), w.mag] : [-1, 0, 0])),
      ...AMMO_KINDS.map(k => p.ammo[k]), Math.round(p.damage), p.killedBy, r1(p.aliveTime),
      ...p.mats, p.piece, p.mat, r2(p.useT), r2(p.swingT), r2(p.revive),
      r2(p.ads), r3(p.kick), r3(p.kickYaw), (p.crouch ? 1 : 0) | (p.slideT > 0 ? 2 : 0) | (p.tactical ? 4 : 0) | (p.mantleT > 0 ? 8 : 0), r1(p.tacT),
      p.lethal * 10 + p.lethalN, p.tacKind * 10 + p.tacN, r2(p.streak), p.ready, p.cls, p.points, r1(p.respawn), r1(p.blind), p.deaths, r1(p.spotted),
    ]),
    q: state.players.map(p => [FIGHTER_IDS.indexOf(p.fighter), p.human ? 1 : 0, p.slot, p.team]),
    n: state.players.map(p => (p.human ? '' : p.name.slice(0, 40))),
    st: [state.storm.stage, r1(state.storm.cx), r1(state.storm.cz), r1(state.storm.r), r1(state.storm.toX), r1(state.storm.toZ), r1(state.storm.toR), r2(state.storm.timer), state.storm.shrinking ? 1 : 0, state.storm.damage],
    ch: state.chests.map(c => (c ? '1' : '0')).join(''),
    r: state.rockets.map(r => [r.id, r2(r.x), r2(r.y), r2(r.z), r2(r.vx), r2(r.vy), r2(r.vz)]),
    e: evs.map(e => {
      const row = [e.id - prevEvent, EVENTS.indexOf(e.type), e.player, e.target, e.value, e.flag, r1(e.x), r1(e.y), r1(e.z)];
      prevEvent = e.id;
      if (WITH_END.has(e.type)) row.push(r1(e.x2), r1(e.y2), r1(e.z2));
      return row;
    }),
    pd: state.pads.slice(0, MAX_PADS).flatMap(p => [r1(p.x), r1(p.y), r1(p.z)]),
    dr: state.drops.slice(0, MAX_DROPS).map(d => [d.id, r1(d.x), r2(d.y), r1(d.z), r2(d.ground), d.landed ? 1 : 0]),
    fl: state.felled.slice(0, MAX_NODES),
    ms: [obj.score[0], obj.score[1], obj.limit, r1(obj.time), r1(obj.over), r1(obj.ax), r1(obj.az), r1(obj.ar), obj.round, obj.wave, r1(obj.timer), obj.left, obj.lastRound, r1(obj.lzX), r1(obj.lzY), r1(obj.lzZ), obj.aboard],
    cp: obj.points.flatMap(pt => [r1(pt.x), r1(pt.y), r1(pt.z), r1(pt.r), pt.own, r2(pt.cap), pt.by]),
    // A full floor of loot never rides with more than Apocalypse Royale's horde (the sim keeps both under these caps).
    zb: flatIds(state.infected.slice(0, withLoot && state.loot.length > ZOMBIE_LOOT_CAP ? APOC_CAP : MAX_INFECTED), z => [z.kind + (z.swing > 0 ? 4 : 0) + (z.stun > 0 ? 8 : 0), dm(z.x), dm(z.y), dm(z.z), cr(z.yaw), Math.max(0, Math.min(100, Math.round((z.hp / z.max) * 100)))]),
    nd: state.nades.slice(0, MAX_FLYING).flatMap(n => [n.kind, r1(n.x), r1(n.y), r1(n.z), n.owner]),
    ar: state.areas.slice(0, MAX_AREAS).flatMap(a => [a.kind, r1(a.x), r1(a.y), r1(a.z), r1(a.r), r1(a.t), a.team]),
    dn: state.drones.slice(0, MAX_DRONES).flatMap(d => [d.owner, r1(d.x), r1(d.y), r1(d.z), r2(d.yaw)]),
    uv: state.uav.slice(0, MAX_FIGHTERS).map(r1),
    lr: state.lootRev, br: state.buildRev,
  };
  if (withLoot) snap.l = flatIds(state.loot, l => [lootCode(l), l.amount, dm(l.x), dm(l.y), dm(l.z)]);
  if (withBuilds) snap.b = state.builds.flatMap(b => [packBuild(b), Math.max(1, Math.round(b.hp)) * (b.grow > 0 ? -1 : 1)]);
  return snap;
}

const nums = (row: unknown, n: number): row is number[] => Array.isArray(row) && row.length === n && row.every(finite);
const flat = (list: unknown, n: number, max: number): list is number[] => Array.isArray(list) && list.length % n === 0 && list.length <= max * n && list.every(finite);
const int = (v: number, lo: number, hi: number) => Number.isInteger(v) && v >= lo && v <= hi;

function decodeLoot(row: number[], id: number): Loot | null {
  if (!int(row[0], 0, LOOT_KINDS.length * ITEM_SLOTS * 5 * 6 - 1)) return null;
  let c = row[0];
  const sub = (c % 6) - 1; c = Math.floor(c / 6);
  const rarity = c % 5; c = Math.floor(c / 5);
  const item = (c % ITEM_SLOTS) - 1, kind = LOOT_KINDS[Math.floor(c / ITEM_SLOTS)];
  // The item must match the kind: weapons carry a weapon, items a consumable, the rest nothing.
  const itemId = item >= 0 ? ITEM_IDS[item] : null;
  if (!kind || (kind === 'weapon' ? !isWeapon(itemId) : kind === 'item' ? !isConsumable(itemId) : itemId !== null)) return null;
  if (kind === 'attach' && !int(row[1], 0, ATTACHMENTS.length - 1)) return null;
  if (kind === 'nade' && !int(row[1], 0, 3)) return null;
  const ammo = kind === 'ammo' && sub >= 0 ? AMMO_KINDS[sub] ?? null : null;
  const mat = kind === 'mats' && sub >= 0 && sub <= 2 ? (sub as Material) : null;
  return { id, kind, weapon: itemId, rarity: rarity as Rarity, ammo, mat, amount: row[1], x: row[2] / 10, y: row[3] / 10, z: row[4] / 10 };
}

/** Structures from the flat [code, hp, …] list (null when malformed). */
export function decodeBuilds(list: unknown): Build[] | null {
  if (!Array.isArray(list) || list.length % 2 !== 0 || list.length > MAX_BUILDS * 2 || !list.every(finite)) return null;
  const out: Build[] = [];
  for (let i = 0; i < list.length; i += 2) {
    const slot = unpackBuild(list[i]);
    const hp = list[i + 1];
    if (!slot || !int(Math.abs(hp), 1, MATERIALS[slot.mat].hp)) return null;
    out.push({ id: i / 2, ...slot, hp: Math.abs(hp), owner: -1, grow: hp < 0 ? 1 : 0, ...buildGeometry(slot) });
  }
  return out;
}

/** Mode state (scores, Zombies round, landing zone) and capture points (null when malformed). */
function decodeMode(ms: unknown, cp: unknown): ModeState | null {
  if (!nums(ms, MS_N) || !flat(cp, CP_N, 3)) return null;
  const obj = emptyMode();
  obj.score = [ms[0], ms[1]]; obj.limit = ms[2]; obj.time = ms[3]; obj.over = ms[4]; obj.ax = ms[5]; obj.az = ms[6]; obj.ar = ms[7];
  obj.round = ms[8]; obj.wave = ms[9]; obj.timer = ms[10]; obj.left = ms[11]; obj.lastRound = ms[12]; obj.lzX = ms[13]; obj.lzY = ms[14]; obj.lzZ = ms[15]; obj.aboard = ms[16];
  if (!int(obj.wave, 0, 4) || !int(obj.round, 0, 999)) return null;
  for (let i = 0; i < cp.length; i += CP_N) {
    const pt: CapturePoint = { x: cp[i], y: cp[i + 1], z: cp[i + 2], r: cp[i + 3], own: cp[i + 4], cap: cp[i + 5], by: cp[i + 6] };
    if (!int(pt.own, -1, 1) || !int(pt.by, -1, 1) || pt.cap < 0 || pt.cap > 1) return null;
    obj.points.push(pt);
  }
  return obj;
}

/** The infected (render / HUD only: no AI state). */
function decodeInfected(list: unknown): Zombie[] | null {
  if (!flat(list, ZB_N, MAX_INFECTED)) return null;
  const out: Zombie[] = [];
  let id = 0;
  for (let i = 0; i < list.length; i += ZB_N) {
    id += list[i];
    const code = list[i + 1], kind = code & 3;
    if (!int(code, 0, 15) || !ZOMBIE_KINDS[kind] || !int(list[i + 6], 0, 100)) return null;
    out.push({
      id, kind: kind as ZombieKind, x: list[i + 2] / 10, y: list[i + 3] / 10, z: list[i + 4] / 10, vx: 0, vy: 0, vz: 0, yaw: list[i + 5] / 100, grounded: true, hp: list[i + 6], max: 100,
      target: -1, think: 0, attackT: 0, spitT: 0, stun: code & 8 ? 1 : 0, stuck: 0, lastX: 0, lastZ: 0, side: 1, sideT: 0, wx: 0, wz: 0, aggro: false, swing: code & 4 ? 0.3 : 0, los: false, idle: 0,
    });
  }
  return out;
}

/** Rebuilds a render/HUD state. `prevLoot` / `prevBuilds` are reused when the snapshot omits them. */
export function decodeSnapshot(data: unknown, names: (slot: number) => string, prevLoot: Loot[] | null = null, prevBuilds: Build[] | null = null): ShooterState | null {
  if (!data || typeof data !== 'object') return null;
  const s = data as Partial<ShooterSnapshot>;
  if (s.v !== 4 || !SHOOTER_MAP_IDS.includes(s.m as ShooterMapId) || !finite(s.p) || !PHASES[s.p] || !finite(s.d) || !DIFFS[s.d] || ![s.c, s.et, s.w, s.lr, s.br, s.wt].every(finite)) return null;
  if (!SQUADS.includes(s.sq as TeamSize) || !finite(s.mo) || !SHOOTER_MODE_IDS[s.mo]) return null;
  if (!nums(s.bus, 6) || !nums(s.st, 10) || typeof s.ch !== 'string' || !/^[01]*$/.test(s.ch)) return null;
  if (!Array.isArray(s.n) || s.n.length !== s.f?.length || !s.n.every(n => typeof n === 'string' && n.length <= 40)) return null;
  if (!Array.isArray(s.f) || !Array.isArray(s.q) || s.f.length !== s.q.length || s.f.length === 0 || s.f.length > MAX_FIGHTERS) return null;
  if (!Array.isArray(s.r) || s.r.length > MAX_ROCKETS || !Array.isArray(s.e) || s.e.length > 64) return null;
  const map = mapFor(s.m!);
  if (s.ch.length !== map.chests.length) return null;
  if (!Array.isArray(s.pd) || s.pd.length % 3 !== 0 || s.pd.length > MAX_PADS * 3 || !s.pd.every(finite)) return null;
  if (!Array.isArray(s.dr) || s.dr.length > MAX_DROPS || !s.dr.every(d => nums(d, 6) && (d[5] === 0 || d[5] === 1))) return null;
  if (!Array.isArray(s.fl) || s.fl.length > MAX_NODES || !s.fl.every(i => int(i, 0, map.cylinders.length - 1))) return null;
  if (!flat(s.nd, ND_N, MAX_FLYING) || !flat(s.ar, AR_N, MAX_AREAS) || !flat(s.dn, DN_N, MAX_DRONES) || !Array.isArray(s.uv) || s.uv.length > MAX_FIGHTERS || !s.uv.every(finite)) return null;
  const obj = decodeMode(s.ms, s.cp);
  const infected = decodeInfected(s.zb);
  if (!obj || !infected) return null;
  const players: Fighter[] = [];
  for (let i = 0; i < s.f.length; i++) {
    const row = s.f[i], q = s.q[i];
    if (!nums(row, FIGHTER_FIELDS) || !nums(q, 4) || !FIGHTER_IDS[q[0]] || !int(q[3], 0, MAX_FIGHTERS - 1) || !int(row[8], 0, MODES.length - 1) || !int(row[13], 0, PICKAXE_SLOT)) return null;
    const t = row.slice(TAIL_AT), g = row.slice(GEAR_AT);
    if (!t.slice(3, 6).every(m => int(m, 0, MAX_MATS)) || !int(t[6], 0, 2) || !int(t[7], 0, 2) || t[10] < 0 || t[10] > REVIVE_TIME + 0.1) return null;
    if (MODES[row[8]] === 'down' && !(row[9] >= 0 && row[9] <= DOWN_HP)) return null;
    // Grenades (kind * 10 + count), streak bits, class.
    if (!int(g[5], 0, NADE_KINDS * 10 - 1) || !int(g[6], 0, NADE_KINDS * 10 - 1) || !int(g[8], 0, (1 << STREAKS.length) - 1) || !int(g[9], 0, CLASSES.length - 1)) return null;
    const weapons = Array.from({ length: SLOTS }, (_, k): Held | null => {
      const [w, code, mag] = row.slice(SLOT_AT + k * 3, SLOT_AT + 3 + k * 3);
      return w >= 0 && ITEM_IDS[w] && int(code, 0, 79) ? { w: ITEM_IDS[w], rarity: (code % 5) as Rarity, att: Math.floor(code / 5), mag } : null;
    });
    const ammo = Object.fromEntries(AMMO_KINDS.map((k, j) => [k, row[AMMO_AT + j]])) as Fighter['ammo'];
    const flags = row[15], stance = g[3];
    players.push({
      id: i, fighter: FIGHTER_IDS[q[0]], name: q[1] === 1 ? names(q[2]) : s.n![i], human: q[1] === 1, slot: q[2], team: q[3], revive: t[10], knocker: -1,
      x: row[0], y: row[1], z: row[2], yaw: row[3], pitch: row[4], vx: row[5], vy: row[6], vz: row[7], grounded: (flags & 8) === 8,
      mode: MODES[row[8]], hp: row[9], shield: row[10], kills: row[11], place: row[12], killedBy: t[1], aliveTime: t[2],
      weapons, current: row[13], ammo, mats: [t[3], t[4], t[5]], building: (flags & 16) === 16, piece: t[6] as Fighter['piece'], mat: t[7] as Material, swimming: (flags & 32) === 32,
      cooldown: 0, reload: row[14], aiming: (flags & 1) === 1, sprinting: (flags & 2) === 2, moving: (flags & 4) === 4,
      shootT: row[16], hurtT: row[17], bloom: 0, useT: t[8], swingT: t[9], interact: row[18], prevFire: false, prevJump: false, prevInteract: false, prevReload: false, prevEdit: false, stormT: 0, damage: t[0],
      ads: g[0], kick: g[1], kickYaw: g[2], shots: 0, crouch: (stance & 1) === 1, slideT: stance & 2 ? 0.3 : 0, tacT: g[4], tactical: (stance & 4) === 4, mantleT: stance & 8 ? 0.2 : 0, mantleX: 0, mantleY: 0, mantleZ: 0,
      lethal: Math.floor(g[5] / 10) as NadeKind, lethalN: g[5] % 10, tacKind: Math.floor(g[6] / 10) as NadeKind, tacN: g[6] % 10, streak: g[7], ready: g[8],
      cls: g[9], points: g[10], respawn: g[11], blind: g[12], deaths: g[13], spotted: g[14],
      prevCrouch: false, prevLethal: false, prevTactical: false, prevStreak: false,
      bot: emptyBot(),
    });
  }
  let loot: Loot[];
  if (s.l !== undefined) {
    if (!flat(s.l, LOOT_N, MAX_LOOT)) return null;
    loot = [];
    let id = 0;
    for (let i = 0; i < s.l.length; i += LOOT_N) { id += s.l[i]; const l = decodeLoot(s.l.slice(i + 1, i + LOOT_N), id); if (!l) return null; loot.push(l); }
  } else loot = prevLoot ?? [];
  let builds: Build[];
  if (s.b !== undefined) { const b = decodeBuilds(s.b); if (!b) return null; builds = b; } else builds = prevBuilds ?? [];
  const events: ShooterEvent[] = [];
  let eid = 0;
  for (const e of s.e) {
    if (!(nums(e, 9) || nums(e, 12)) || !EVENTS[e[1]]) return null;
    eid += e[0];
    events.push({ id: eid, type: EVENTS[e[1]], player: e[2], target: e[3], value: e[4], flag: e[5], x: e[6], y: e[7], z: e[8], x2: e[9] ?? 0, y2: e[10] ?? 0, z2: e[11] ?? 0 });
  }
  for (const r of s.r) if (!nums(r, 7)) return null;
  const st = s.st, nd = s.nd, ar = s.ar, dn = s.dn;
  return {
    map: map.id, difficulty: DIFFS[s.d!], phase: PHASES[s.p!], clock: s.c!, endTimer: s.et!, winner: s.w!,
    bus: { ax: s.bus[0], az: s.bus[1], bx: s.bus[2], bz: s.bus[3], t: s.bus[4], duration: s.bus[5] },
    players, loot, lootRev: s.lr!, chests: [...s.ch].map(c => c === '1'), builds, buildRev: s.br!,
    squad: s.sq as TeamSize, winTeam: s.wt!, nodes: [], felled: s.fl, stormScale: 1,
    mode: SHOOTER_MODE_IDS[s.mo!] as ShooterMode, obj,
    nades: Array.from({ length: nd.length / ND_N }, (_, i) => ({ id: i, kind: Math.max(0, Math.min(NADE_KINDS - 1, nd[i * ND_N] | 0)) as NadeKind, x: nd[i * ND_N + 1], y: nd[i * ND_N + 2], z: nd[i * ND_N + 3], owner: nd[i * ND_N + 4], team: -1, vx: 0, vy: 0, vz: 0, fuse: 1, rest: false })),
    areas: Array.from({ length: ar.length / AR_N }, (_, i) => ({ id: i, kind: ar[i * AR_N], x: ar[i * AR_N + 1], y: ar[i * AR_N + 2], z: ar[i * AR_N + 3], r: ar[i * AR_N + 4], t: ar[i * AR_N + 5], team: ar[i * AR_N + 6], owner: -1, tick: 0 })),
    drones: Array.from({ length: dn.length / DN_N }, (_, i) => ({ id: i, owner: dn[i * DN_N], team: -1, x: dn[i * DN_N + 1], y: dn[i * DN_N + 2], z: dn[i * DN_N + 3], yaw: dn[i * DN_N + 4], t: 1, cool: 0 })),
    strikes: [], uav: s.uv!, infected,
    pads: Array.from({ length: s.pd.length / 3 }, (_, i) => ({ id: i, x: s.pd![i * 3], y: s.pd![i * 3 + 1], z: s.pd![i * 3 + 2] })),
    drops: s.dr.map(d => ({ id: d[0], x: d[1], y: d[2], z: d[3], ground: d[4], landed: d[5] === 1 })),
    rockets: s.r.map(r => ({ id: r[0], owner: -1, rarity: 0 as Rarity, x: r[1], y: r[2], z: r[3], vx: r[4], vy: r[5], vz: r[6], life: 1 })),
    storm: { stage: st[0], cx: st[1], cz: st[2], r: st[3], toX: st[4], toZ: st[5], toR: st[6], timer: st[7], shrinking: st[8] === 1, damage: st[9], fromX: st[1], fromZ: st[2], fromR: st[3] },
    events, eventSeq: events.length ? events[events.length - 1].id : 0, nextId: 0, rng: 0, accumulator: 0,
  };
}

export interface ShooterWireInput { k: number[] }
export function encodeInput(i: ShooterInput, fighter: FighterId): ShooterWireInput {
  const flags = (i.fire ? 1 : 0) | (i.aim ? 2 : 0) | (i.jump ? 4 : 0) | (i.sprint ? 8 : 0) | (i.reload ? 16 : 0) | (i.interact ? 32 : 0) | (i.edit ? 64 : 0) | (i.build ? 128 : 0)
    | (i.crouch ? 256 : 0) | (i.tac ? 512 : 0) | (i.lethal ? 1024 : 0) | (i.tactical ? 2048 : 0) | (i.streak ? 4096 : 0);
  return { k: [r2(i.mx), r2(i.mz), r3(i.yaw), r3(i.pitch), flags, i.slot, FIGHTER_IDS.indexOf(fighter), i.piece, i.mat, i.cls] };
}
export function decodeInput(data: unknown): { input: ShooterInput; fighter: FighterId } | null {
  if (!data || typeof data !== 'object') return null;
  const k = (data as Partial<ShooterWireInput>).k;
  if (!nums(k, 10) || Math.abs(k[0]) > 1 || Math.abs(k[1]) > 1 || Math.abs(k[2]) > 7 || Math.abs(k[3]) > 1.6 || !int(k[4], 0, 8191) || !int(k[5], 0, PICKAXE_KEY) || !FIGHTER_IDS[k[6]] || !int(k[7], 0, 2) || !int(k[8], 0, 2) || !int(k[9], 0, CLASSES.length - 1)) return null;
  const f = k[4];
  return {
    input: {
      mx: k[0], mz: k[1], yaw: k[2], pitch: k[3], fire: (f & 1) > 0, aim: (f & 2) > 0, jump: (f & 4) > 0, sprint: (f & 8) > 0, reload: (f & 16) > 0, interact: (f & 32) > 0, slot: k[5], build: (f & 128) > 0, piece: k[7], mat: k[8], edit: (f & 64) > 0,
      crouch: (f & 256) > 0, tac: (f & 512) > 0, lethal: (f & 1024) > 0, tactical: (f & 2048) > 0, streak: (f & 4096) > 0, cls: k[9],
    },
    fighter: FIGHTER_IDS[k[6]],
  };
}
