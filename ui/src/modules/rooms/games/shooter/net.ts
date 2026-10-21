/**
 * Compact relay payloads for Storm Royale. Snapshots carry every fighter
 * (with team and revive progress), the storm, rockets, launch pads, supply
 * drops, felled trees and recent events; the two bulky lists are optional and
 * sent only when they changed (or periodically), never both at once, so a
 * snapshot stays under the relay's 16 KiB cap: loot (six numbers per item)
 * and structures (two numbers per piece: packed slot + hp). Inputs are a
 * nine-number array. Everything is validated before use.
 */
import { finite, r2, r3 } from '../common/math.ts';
import { FIGHTER_IDS, type FighterId } from './catalog.ts';
import { AMMO_KINDS, isConsumable, isWeapon, ITEM_IDS, ITEM_SLOTS, MATERIALS, MAX_MATS, type Material, type Rarity } from './weapons.ts';
import { buildGeometry, MAX_BUILDS, packBuild, unpackBuild, type Build } from './builds.ts';
import {
  DOWN_HP, emptyBot, MAX_DROPS, MAX_FIGHTERS, MAX_LOOT, MAX_NODES, MAX_PADS, MAX_ROCKETS, PICKAXE_KEY, PICKAXE_SLOT, REVIVE_TIME, SLOTS,
  type Fighter, type FighterMode, type Held, type Loot, type LootKind, type ShooterEvent, type ShooterEventType, type ShooterInput, type ShooterState, type TeamSize,
} from './sim.ts';
import { mapFor } from './map.ts';

const PHASES = ['bus', 'play', 'done'] as const;
const MODES: FighterMode[] = ['bus', 'sky', 'glide', 'ground', 'dead', 'down'];
const LOOT_KINDS: LootKind[] = ['weapon', 'ammo', 'heal', 'shield', 'item', 'mats'];
/** Append-only: the index is the wire value. */
const EVENTS: ShooterEventType[] = ['shot', 'hit', 'kill', 'pickup', 'chest', 'explode', 'reload', 'land', 'jump', 'storm', 'win', 'glide', 'build', 'break', 'harvest', 'use', 'swing', 'edit', 'knock', 'revive', 'supply', 'launch', 'fell'];
const SQUADS: TeamSize[] = [1, 2, 4];
const DIFFS = ['easy', 'normal', 'hard'] as const;
const r1 = (v: number) => Math.round(v * 10) / 10;
const SLOT_AT = 19;
const AMMO_AT = SLOT_AT + SLOTS * 3;
const TAIL_AT = AMMO_AT + AMMO_KINDS.length;
export const FIGHTER_FIELDS = TAIL_AT + 11;

export interface ShooterSnapshot {
  v: 3; m: string; d: number; p: number; c: number; et: number; w: number; bus: number[];
  /** Team size, winning team. */
  sq: number; wt: number;
  /** Fighter rows; per-fighter fighter id, human flag, member slot and team. */
  f: number[][]; q: number[][]; n: string[]; st: number[]; ch: string; r: number[][]; e: number[][];
  /** Launch pads (x, y, z per pad), supply drops (id, x, y, z, landing height, landed) and felled trunk indices. */
  pd: number[]; dr: number[][]; fl: number[];
  lr: number; l?: number[][];
  /** Structures: packed slot, then hp (negative while the piece is still going up). */
  br: number; b?: number[];
}

/** Loot kind, item, rarity and ammo/material in one small integer. */
const lootCode = (l: Loot) => ((LOOT_KINDS.indexOf(l.kind) * ITEM_SLOTS + (l.weapon ? ITEM_IDS.indexOf(l.weapon) : -1) + 1) * 5 + l.rarity) * 6 + (l.ammo ? AMMO_KINDS.indexOf(l.ammo) : l.mat ?? -1) + 1;

export function encodeSnapshot(state: ShooterState, events: ShooterEvent[], withLoot = true, withBuilds = false): ShooterSnapshot {
  const snap: ShooterSnapshot = {
    v: 3, m: state.map, d: DIFFS.indexOf(state.difficulty), p: PHASES.indexOf(state.phase), c: r2(state.clock), et: r2(state.endTimer), w: state.winner,
    sq: state.squad, wt: state.winTeam,
    bus: [r1(state.bus.ax), r1(state.bus.az), r1(state.bus.bx), r1(state.bus.bz), r2(state.bus.t), r2(state.bus.duration)],
    f: state.players.map(p => [
      r2(p.x), r2(p.y), r2(p.z), r3(p.yaw), r3(p.pitch), r2(p.vx), r2(p.vy), r2(p.vz), MODES.indexOf(p.mode), Math.round(p.hp), Math.round(p.shield),
      p.kills, p.place, p.current, r2(p.reload), (p.aiming ? 1 : 0) | (p.sprinting ? 2 : 0) | (p.moving ? 4 : 0) | (p.grounded ? 8 : 0) | (p.building ? 16 : 0) | (p.swimming ? 32 : 0),
      r2(p.shootT), r2(p.hurtT), r2(p.interact),
      ...p.weapons.flatMap(w => (w ? [ITEM_IDS.indexOf(w.w), w.rarity, w.mag] : [-1, 0, 0])),
      ...AMMO_KINDS.map(k => p.ammo[k]), Math.round(p.damage), p.killedBy, r1(p.aliveTime),
      ...p.mats, p.piece, p.mat, r2(p.useT), r2(p.swingT), r2(p.revive),
    ]),
    q: state.players.map(p => [FIGHTER_IDS.indexOf(p.fighter), p.human ? 1 : 0, p.slot, p.team]),
    n: state.players.map(p => (p.human ? '' : p.name.slice(0, 40))),
    st: [state.storm.stage, r1(state.storm.cx), r1(state.storm.cz), r1(state.storm.r), r1(state.storm.toX), r1(state.storm.toZ), r1(state.storm.toR), r2(state.storm.timer), state.storm.shrinking ? 1 : 0, state.storm.damage],
    ch: state.chests.map(c => (c ? '1' : '0')).join(''),
    r: state.rockets.map(r => [r.id, r2(r.x), r2(r.y), r2(r.z), r2(r.vx), r2(r.vy), r2(r.vz)]),
    e: events.slice(-24).map(e => [e.id, EVENTS.indexOf(e.type), e.player, e.target, e.value, e.flag, r2(e.x), r2(e.y), r2(e.z), r2(e.x2), r2(e.y2), r2(e.z2)]),
    pd: state.pads.slice(0, MAX_PADS).flatMap(p => [r1(p.x), r1(p.y), r1(p.z)]),
    dr: state.drops.slice(0, MAX_DROPS).map(d => [d.id, r1(d.x), r2(d.y), r1(d.z), r2(d.ground), d.landed ? 1 : 0]),
    fl: state.felled.slice(0, MAX_NODES),
    lr: state.lootRev, br: state.buildRev,
  };
  if (withLoot) snap.l = state.loot.map(l => [l.id, lootCode(l), l.amount, r1(l.x), r1(l.y), r1(l.z)]);
  if (withBuilds) snap.b = state.builds.flatMap(b => [packBuild(b), Math.max(1, Math.round(b.hp)) * (b.grow > 0 ? -1 : 1)]);
  return snap;
}

const nums = (row: unknown, n: number): row is number[] => Array.isArray(row) && row.length === n && row.every(finite);
const int = (v: number, lo: number, hi: number) => Number.isInteger(v) && v >= lo && v <= hi;

function decodeLoot(row: unknown): Loot | null {
  if (!nums(row, 6) || !int(row[1], 0, LOOT_KINDS.length * ITEM_SLOTS * 5 * 6 - 1)) return null;
  let c = row[1];
  const sub = (c % 6) - 1; c = Math.floor(c / 6);
  const rarity = c % 5; c = Math.floor(c / 5);
  const item = (c % ITEM_SLOTS) - 1, kind = LOOT_KINDS[Math.floor(c / ITEM_SLOTS)];
  // The item must match the kind: weapons carry a weapon, items a consumable, the rest nothing.
  const id = item >= 0 ? ITEM_IDS[item] : null;
  if (!kind || (kind === 'weapon' ? !isWeapon(id) : kind === 'item' ? !isConsumable(id) : id !== null)) return null;
  const ammo = kind === 'ammo' && sub >= 0 ? AMMO_KINDS[sub] ?? null : null;
  const mat = kind === 'mats' && sub >= 0 && sub <= 2 ? (sub as Material) : null;
  return { id: row[0], kind, weapon: id, rarity: rarity as Rarity, ammo, mat, amount: row[2], x: row[3], y: row[4], z: row[5] };
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

/** Rebuilds a render/HUD state. `prevLoot` / `prevBuilds` are reused when the snapshot omits them. */
export function decodeSnapshot(data: unknown, names: (slot: number) => string, prevLoot: Loot[] | null = null, prevBuilds: Build[] | null = null): ShooterState | null {
  if (!data || typeof data !== 'object') return null;
  const s = data as Partial<ShooterSnapshot>;
  if (s.v !== 3 || (s.m !== 'isle' && s.m !== 'mesa') || !finite(s.p) || !PHASES[s.p] || !finite(s.d) || !DIFFS[s.d] || ![s.c, s.et, s.w, s.lr, s.br, s.wt].every(finite)) return null;
  if (!SQUADS.includes(s.sq as TeamSize)) return null;
  if (!nums(s.bus, 6) || !nums(s.st, 10) || typeof s.ch !== 'string' || !/^[01]*$/.test(s.ch)) return null;
  if (!Array.isArray(s.n) || s.n.length !== s.f?.length || !s.n.every(n => typeof n === 'string' && n.length <= 40)) return null;
  if (!Array.isArray(s.f) || !Array.isArray(s.q) || s.f.length !== s.q.length || s.f.length === 0 || s.f.length > MAX_FIGHTERS) return null;
  if (!Array.isArray(s.r) || s.r.length > MAX_ROCKETS || !Array.isArray(s.e) || s.e.length > 64) return null;
  const map = mapFor(s.m);
  if (s.ch.length !== map.chests.length) return null;
  if (!Array.isArray(s.pd) || s.pd.length % 3 !== 0 || s.pd.length > MAX_PADS * 3 || !s.pd.every(finite)) return null;
  if (!Array.isArray(s.dr) || s.dr.length > MAX_DROPS || !s.dr.every(d => nums(d, 6) && (d[5] === 0 || d[5] === 1))) return null;
  if (!Array.isArray(s.fl) || s.fl.length > MAX_NODES || !s.fl.every(i => int(i, 0, map.cylinders.length - 1))) return null;
  const players: Fighter[] = [];
  for (let i = 0; i < s.f.length; i++) {
    const row = s.f[i], q = s.q[i];
    if (!nums(row, FIGHTER_FIELDS) || !nums(q, 4) || !FIGHTER_IDS[q[0]] || !int(q[3], 0, MAX_FIGHTERS - 1) || !int(row[8], 0, MODES.length - 1) || !int(row[13], 0, PICKAXE_SLOT)) return null;
    const t = row.slice(TAIL_AT);
    if (!t.slice(3, 6).every(m => int(m, 0, MAX_MATS)) || !int(t[6], 0, 2) || !int(t[7], 0, 2) || t[10] < 0 || t[10] > REVIVE_TIME + 0.1) return null;
    if (MODES[row[8]] === 'down' && !(row[9] >= 0 && row[9] <= DOWN_HP)) return null;
    const weapons = Array.from({ length: SLOTS }, (_, k): Held | null => {
      const [w, rarity, mag] = row.slice(SLOT_AT + k * 3, SLOT_AT + 3 + k * 3);
      return w >= 0 && ITEM_IDS[w] && int(rarity, 0, 4) ? { w: ITEM_IDS[w], rarity: rarity as Rarity, mag } : null;
    });
    const ammo = Object.fromEntries(AMMO_KINDS.map((k, j) => [k, row[AMMO_AT + j]])) as Fighter['ammo'];
    const flags = row[15];
    players.push({
      id: i, fighter: FIGHTER_IDS[q[0]], name: q[1] === 1 ? names(q[2]) : s.n![i], human: q[1] === 1, slot: q[2], team: q[3], revive: t[10], knocker: -1,
      x: row[0], y: row[1], z: row[2], yaw: row[3], pitch: row[4], vx: row[5], vy: row[6], vz: row[7], grounded: (flags & 8) === 8,
      mode: MODES[row[8]], hp: row[9], shield: row[10], kills: row[11], place: row[12], killedBy: t[1], aliveTime: t[2],
      weapons, current: row[13], ammo, mats: [t[3], t[4], t[5]], building: (flags & 16) === 16, piece: t[6] as Fighter['piece'], mat: t[7] as Material, swimming: (flags & 32) === 32,
      cooldown: 0, reload: row[14], aiming: (flags & 1) === 1, sprinting: (flags & 2) === 2, moving: (flags & 4) === 4,
      shootT: row[16], hurtT: row[17], bloom: 0, useT: t[8], swingT: t[9], interact: row[18], prevFire: false, prevJump: false, prevInteract: false, prevReload: false, prevEdit: false, stormT: 0, damage: t[0],
      bot: emptyBot(),
    });
  }
  let loot: Loot[];
  if (s.l !== undefined) {
    if (!Array.isArray(s.l) || s.l.length > MAX_LOOT) return null;
    loot = [];
    for (const row of s.l) { const l = decodeLoot(row); if (!l) return null; loot.push(l); }
  } else loot = prevLoot ?? [];
  let builds: Build[];
  if (s.b !== undefined) { const b = decodeBuilds(s.b); if (!b) return null; builds = b; } else builds = prevBuilds ?? [];
  const events: ShooterEvent[] = [];
  for (const e of s.e) {
    if (!nums(e, 12) || !EVENTS[e[1]]) return null;
    events.push({ id: e[0], type: EVENTS[e[1]], player: e[2], target: e[3], value: e[4], flag: e[5], x: e[6], y: e[7], z: e[8], x2: e[9], y2: e[10], z2: e[11] });
  }
  for (const r of s.r) if (!nums(r, 7)) return null;
  const st = s.st;
  return {
    map: map.id, difficulty: DIFFS[s.d!], phase: PHASES[s.p!], clock: s.c!, endTimer: s.et!, winner: s.w!,
    bus: { ax: s.bus[0], az: s.bus[1], bx: s.bus[2], bz: s.bus[3], t: s.bus[4], duration: s.bus[5] },
    players, loot, lootRev: s.lr!, chests: [...s.ch].map(c => c === '1'), builds, buildRev: s.br!,
    squad: s.sq as TeamSize, winTeam: s.wt!, nodes: [], felled: s.fl, stormScale: 1,
    pads: Array.from({ length: s.pd.length / 3 }, (_, i) => ({ id: i, x: s.pd![i * 3], y: s.pd![i * 3 + 1], z: s.pd![i * 3 + 2] })),
    drops: s.dr.map(d => ({ id: d[0], x: d[1], y: d[2], z: d[3], ground: d[4], landed: d[5] === 1 })),
    rockets: s.r.map(r => ({ id: r[0], owner: -1, rarity: 0 as Rarity, x: r[1], y: r[2], z: r[3], vx: r[4], vy: r[5], vz: r[6], life: 1 })),
    storm: { stage: st[0], cx: st[1], cz: st[2], r: st[3], toX: st[4], toZ: st[5], toR: st[6], timer: st[7], shrinking: st[8] === 1, damage: st[9], fromX: st[1], fromZ: st[2], fromR: st[3] },
    events, eventSeq: events.length ? events[events.length - 1].id : 0, nextId: 0, rng: 0, accumulator: 0,
  };
}

export interface ShooterWireInput { k: number[] }
export function encodeInput(i: ShooterInput, fighter: FighterId): ShooterWireInput {
  const flags = (i.fire ? 1 : 0) | (i.aim ? 2 : 0) | (i.jump ? 4 : 0) | (i.sprint ? 8 : 0) | (i.reload ? 16 : 0) | (i.interact ? 32 : 0) | (i.edit ? 64 : 0) | (i.build ? 128 : 0);
  return { k: [r2(i.mx), r2(i.mz), r3(i.yaw), r3(i.pitch), flags, i.slot, FIGHTER_IDS.indexOf(fighter), i.piece, i.mat] };
}
export function decodeInput(data: unknown): { input: ShooterInput; fighter: FighterId } | null {
  if (!data || typeof data !== 'object') return null;
  const k = (data as Partial<ShooterWireInput>).k;
  if (!nums(k, 9) || Math.abs(k[0]) > 1 || Math.abs(k[1]) > 1 || Math.abs(k[2]) > 7 || Math.abs(k[3]) > 1.6 || !int(k[4], 0, 255) || !int(k[5], 0, PICKAXE_KEY) || !FIGHTER_IDS[k[6]] || !int(k[7], 0, 2) || !int(k[8], 0, 2)) return null;
  const f = k[4];
  return {
    input: { mx: k[0], mz: k[1], yaw: k[2], pitch: k[3], fire: (f & 1) > 0, aim: (f & 2) > 0, jump: (f & 4) > 0, sprint: (f & 8) > 0, reload: (f & 16) > 0, interact: (f & 32) > 0, slot: k[5], build: (f & 128) > 0, piece: k[7], mat: k[8], edit: (f & 64) > 0 },
    fighter: FIGHTER_IDS[k[6]],
  };
}
