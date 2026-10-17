/**
 * Compact relay payloads for Storm Royale. Snapshots carry every fighter,
 * the storm, rockets and recent events; the loot list is only included when
 * it changed (or periodically) because it is the largest part. Inputs are a
 * seven-number array. Everything is validated before use.
 */
import { finite, r2, r3 } from '../common/math.ts';
import { FIGHTER_IDS, type FighterId } from './catalog.ts';
import { AMMO_KINDS, WEAPON_IDS, type Rarity } from './weapons.ts';
import { MAX_FIGHTERS, MAX_LOOT, MAX_ROCKETS, SLOTS, type Fighter, type FighterMode, type Loot, type LootKind, type ShooterEvent, type ShooterEventType, type ShooterInput, type ShooterState } from './sim.ts';
import { mapFor } from './map.ts';

const PHASES = ['bus', 'play', 'done'] as const;
const MODES: FighterMode[] = ['bus', 'sky', 'glide', 'ground', 'dead'];
const LOOT_KINDS: LootKind[] = ['weapon', 'ammo', 'heal', 'shield'];
const EVENTS: ShooterEventType[] = ['shot', 'hit', 'kill', 'pickup', 'chest', 'explode', 'reload', 'land', 'jump', 'storm', 'win', 'glide'];
const DIFFS = ['easy', 'normal', 'hard'] as const;
const r1 = (v: number) => Math.round(v * 10) / 10;
const FIGHTER_FIELDS = 36;

export interface ShooterSnapshot {
  v: 1; m: string; d: number; p: number; c: number; et: number; w: number; bus: number[];
  f: number[][]; q: number[][]; n: string[]; st: number[]; ch: string; r: number[][]; e: number[][];
  lr: number; l?: number[][];
}

export function encodeSnapshot(state: ShooterState, events: ShooterEvent[], withLoot = true): ShooterSnapshot {
  const snap: ShooterSnapshot = {
    v: 1, m: state.map, d: DIFFS.indexOf(state.difficulty), p: PHASES.indexOf(state.phase), c: r2(state.clock), et: r2(state.endTimer), w: state.winner,
    bus: [r1(state.bus.ax), r1(state.bus.az), r1(state.bus.bx), r1(state.bus.bz), r2(state.bus.t), r2(state.bus.duration)],
    f: state.players.map(p => [
      r2(p.x), r2(p.y), r2(p.z), r3(p.yaw), r3(p.pitch), r2(p.vx), r2(p.vy), r2(p.vz), MODES.indexOf(p.mode), Math.round(p.hp), Math.round(p.shield),
      p.kills, p.place, p.current, r2(p.reload), (p.aiming ? 1 : 0) | (p.sprinting ? 2 : 0) | (p.moving ? 4 : 0) | (p.grounded ? 8 : 0),
      r2(p.shootT), r2(p.hurtT), r2(p.interact),
      ...p.weapons.flatMap(w => (w ? [WEAPON_IDS.indexOf(w.w), w.rarity, w.mag] : [-1, 0, 0])),
      ...AMMO_KINDS.map(k => p.ammo[k]), Math.round(p.damage), p.killedBy, r1(p.aliveTime),
    ]),
    q: state.players.map(p => [FIGHTER_IDS.indexOf(p.fighter), p.human ? 1 : 0, p.slot]),
    n: state.players.map(p => (p.human ? '' : p.name.slice(0, 40))),
    st: [state.storm.stage, r1(state.storm.cx), r1(state.storm.cz), r1(state.storm.r), r1(state.storm.toX), r1(state.storm.toZ), r1(state.storm.toR), r2(state.storm.timer), state.storm.shrinking ? 1 : 0, state.storm.damage],
    ch: state.chests.map(c => (c ? '1' : '0')).join(''),
    r: state.rockets.map(r => [r.id, r2(r.x), r2(r.y), r2(r.z), r2(r.vx), r2(r.vy), r2(r.vz)]),
    e: events.slice(-24).map(e => [e.id, EVENTS.indexOf(e.type), e.player, e.target, e.value, e.flag, r2(e.x), r2(e.y), r2(e.z), r2(e.x2), r2(e.y2), r2(e.z2)]),
    lr: state.lootRev,
  };
  if (withLoot) snap.l = state.loot.map(l => [l.id, LOOT_KINDS.indexOf(l.kind), l.weapon ? WEAPON_IDS.indexOf(l.weapon) : -1, l.rarity, l.ammo ? AMMO_KINDS.indexOf(l.ammo) : -1, l.amount, r1(l.x), r1(l.y), r1(l.z)]);
  return snap;
}

const nums = (row: unknown, n: number): row is number[] => Array.isArray(row) && row.length === n && row.every(finite);

/** Rebuilds a render/HUD state. `prevLoot` is reused when the snapshot omits loot. */
export function decodeSnapshot(data: unknown, names: (slot: number) => string, prevLoot: Loot[] | null = null): ShooterState | null {
  if (!data || typeof data !== 'object') return null;
  const s = data as Partial<ShooterSnapshot>;
  if (s.v !== 1 || (s.m !== 'isle' && s.m !== 'mesa') || !finite(s.p) || !PHASES[s.p] || !finite(s.d) || !DIFFS[s.d] || ![s.c, s.et, s.w, s.lr].every(finite)) return null;
  if (!nums(s.bus, 6) || !nums(s.st, 10) || typeof s.ch !== 'string' || !/^[01]*$/.test(s.ch)) return null;
  if (!Array.isArray(s.n) || s.n.length !== s.f?.length || !s.n.every(n => typeof n === 'string' && n.length <= 40)) return null;
  if (!Array.isArray(s.f) || !Array.isArray(s.q) || s.f.length !== s.q.length || s.f.length === 0 || s.f.length > MAX_FIGHTERS) return null;
  if (!Array.isArray(s.r) || s.r.length > MAX_ROCKETS || !Array.isArray(s.e) || s.e.length > 64) return null;
  const map = mapFor(s.m);
  if (s.ch.length !== map.chests.length) return null;
  const players: Fighter[] = [];
  for (let i = 0; i < s.f.length; i++) {
    const row = s.f[i], q = s.q[i];
    if (!nums(row, FIGHTER_FIELDS) || !nums(q, 3) || !FIGHTER_IDS[q[0]] || !MODES[row[8]] || row[13] < 0 || row[13] >= SLOTS) return null;
    const weapons = [0, 1, 2].map(k => {
      const [w, rarity, mag] = row.slice(19 + k * 3, 22 + k * 3);
      return w >= 0 && WEAPON_IDS[w] && rarity >= 0 && rarity <= 4 ? { w: WEAPON_IDS[w], rarity: rarity as Rarity, mag } : null;
    });
    const ammo = Object.fromEntries(AMMO_KINDS.map((k, j) => [k, row[28 + j]])) as Fighter['ammo'];
    const flags = row[15];
    players.push({
      id: i, fighter: FIGHTER_IDS[q[0]], name: q[1] === 1 ? names(q[2]) : s.n![i], human: q[1] === 1, slot: q[2],
      x: row[0], y: row[1], z: row[2], yaw: row[3], pitch: row[4], vx: row[5], vy: row[6], vz: row[7], grounded: (flags & 8) === 8,
      mode: MODES[row[8]], hp: row[9], shield: row[10], kills: row[11], place: row[12], killedBy: row[34], aliveTime: row[35],
      weapons, current: row[13], ammo, cooldown: 0, reload: row[14], aiming: (flags & 1) === 1, sprinting: (flags & 2) === 2, moving: (flags & 4) === 4,
      shootT: row[16], hurtT: row[17], bloom: 0, interact: row[18], prevFire: false, prevJump: false, prevInteract: false, prevReload: false, stormT: 0, damage: row[33],
      bot: { goalX: 0, goalZ: 0, think: 0, target: -1, seen: 0, strafe: 1, strafeT: 0, stuck: 0, lastX: 0, lastZ: 0, dropAt: 0, burst: 0, errX: 0, errY: 0, loot: -1, jumpT: 0 },
    });
  }
  let loot: Loot[];
  if (s.l !== undefined) {
    if (!Array.isArray(s.l) || s.l.length > MAX_LOOT) return null;
    loot = [];
    for (const l of s.l) {
      if (!nums(l, 9) || !LOOT_KINDS[l[1]] || l[3] < 0 || l[3] > 4) return null;
      loot.push({ id: l[0], kind: LOOT_KINDS[l[1]], weapon: l[2] >= 0 ? WEAPON_IDS[l[2]] ?? null : null, rarity: l[3] as Rarity, ammo: l[4] >= 0 ? AMMO_KINDS[l[4]] ?? null : null, amount: l[5], x: l[6], y: l[7], z: l[8] });
    }
  } else loot = prevLoot ?? [];
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
    players, loot, lootRev: s.lr!, chests: [...s.ch].map(c => c === '1'),
    rockets: s.r.map(r => ({ id: r[0], owner: -1, rarity: 0 as Rarity, x: r[1], y: r[2], z: r[3], vx: r[4], vy: r[5], vz: r[6], life: 1 })),
    storm: { stage: st[0], cx: st[1], cz: st[2], r: st[3], toX: st[4], toZ: st[5], toR: st[6], timer: st[7], shrinking: st[8] === 1, damage: st[9], fromX: st[1], fromZ: st[2], fromR: st[3] },
    events, eventSeq: events.length ? events[events.length - 1].id : 0, nextId: 0, rng: 0, accumulator: 0,
  };
}

export interface ShooterWireInput { k: number[] }
export function encodeInput(i: ShooterInput, fighter: FighterId): ShooterWireInput {
  const flags = (i.fire ? 1 : 0) | (i.aim ? 2 : 0) | (i.jump ? 4 : 0) | (i.sprint ? 8 : 0) | (i.reload ? 16 : 0) | (i.interact ? 32 : 0);
  return { k: [r2(i.mx), r2(i.mz), r3(i.yaw), r3(i.pitch), flags, i.slot, FIGHTER_IDS.indexOf(fighter)] };
}
export function decodeInput(data: unknown): { input: ShooterInput; fighter: FighterId } | null {
  if (!data || typeof data !== 'object') return null;
  const k = (data as Partial<ShooterWireInput>).k;
  if (!nums(k, 7) || Math.abs(k[0]) > 1 || Math.abs(k[1]) > 1 || Math.abs(k[2]) > 7 || Math.abs(k[3]) > 1.6 || k[4] < 0 || k[4] > 63 || !Number.isInteger(k[5]) || k[5] < 0 || k[5] > SLOTS || !FIGHTER_IDS[k[6]]) return null;
  const f = k[4];
  return {
    input: { mx: k[0], mz: k[1], yaw: k[2], pitch: k[3], fire: (f & 1) > 0, aim: (f & 2) > 0, jump: (f & 4) > 0, sprint: (f & 8) > 0, reload: (f & 16) > 0, interact: (f & 32) > 0, slot: k[5] },
    fighter: FIGHTER_IDS[k[6]],
  };
}
