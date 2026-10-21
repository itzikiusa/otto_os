/**
 * Storm Royale simulation (host-authoritative, fixed 60 Hz, no three.js).
 * Battle royale for up to 16 fighters (humans + bots): drop from the flight
 * path, skydive and glide, loot weapons / ammo / consumables / materials and
 * chests into a five-slot hotbar, fight with hitscan weapons and splash
 * rockets, harvest with the pickaxe and build walls / floors / ramps (edit
 * and destroy them), swim, and survive a storm that closes in seven phases.
 */
import { approach, clamp, Rng, wrapAngle } from '../common/math.ts';
import { chestY, HALF, mapFor, terrainHeight, waterLevel, dryLand, type ShooterMap } from './map.ts';
import { groundAt, HEIGHT, raycast, resolve, worldFor, type World } from './collide.ts';
import {
  AMMO_KINDS, AMMO_MAX, AMMO_PACK, applyDamage, BUILD_COST, canUse, CONSUMABLE_PACK, CONSUMABLES, damageAt, isConsumable, isWeapon, MATERIALS, MAX_MATS, PICKAXE,
  rollConsumable, rollRarity, rollWeapon, WEAPONS, WEAPON_IDS, type AmmoKind, type ItemId, type Material, type Rarity, type WeaponId,
} from './weapons.ts';
import { canPlace, makeBuild, MAX_BUILDS, maxHp, planBuild, refreshGeometry, unsupported, type Build, type BuildKind } from './builds.ts';
import { botThink } from './bots.ts';
import type { FighterId, ShooterMapId } from './catalog.ts';
import type { ShooterSeat } from '../setup.ts';

export type Difficulty = 'easy' | 'normal' | 'hard';
export const STEP = 1 / 60;
export const MAX_FIGHTERS = 16;
export const MAX_LOOT = 240;
export const MAX_ROCKETS = 12;
export const BUS_HEIGHT = 120;
export const BUS_SPEED = 24;
/** Hotbar slots; `current === SLOTS` means the pickaxe is in hand. */
export const SLOTS = 5;
export const PICKAXE_SLOT = SLOTS;
/** Input `slot`: 0 keep, 1..SLOTS hotbar, SLOTS + 1 pickaxe. */
export const PICKAXE_KEY = SLOTS + 1;
/** Feet sit this far under the surface while swimming. */
export const SWIM_DEPTH = 0.85;
export interface ShooterSetup { map: ShooterMapId; difficulty: Difficulty; seed: number; seats: ShooterSeat[] }
export interface ShooterLink {
  role: 'host' | 'guest';
  inputs?: Map<number, { input: ShooterInput; fighter: FighterId }>;
  snapshots?: { state: ShooterState; at: number }[];
}
/**
 * Camera-relative movement (mz forward, mx right) and absolute aim angles.
 * `build` is build mode (fire places `piece` in material `mat`); `edit`
 * reshapes the structure in the crosshair.
 */
export interface ShooterInput {
  mx: number; mz: number; yaw: number; pitch: number;
  fire: boolean; aim: boolean; jump: boolean; sprint: boolean; reload: boolean; interact: boolean; slot: number;
  build: boolean; piece: number; mat: number; edit: boolean;
}
export const idleShooterInput = (yaw = 0, pitch = 0): ShooterInput => ({ mx: 0, mz: 0, yaw, pitch, fire: false, aim: false, jump: false, sprint: false, reload: false, interact: false, slot: 0, build: false, piece: 0, mat: 0, edit: false });

/** A hotbar item: a weapon (`mag` rounds loaded) or a consumable stack (`mag` = count). */
export interface Held { w: ItemId; rarity: Rarity; mag: number }
export interface BotMemory {
  goalX: number; goalZ: number; think: number; target: number; seen: number; strafe: number; strafeT: number;
  stuck: number; lastX: number; lastZ: number; dropAt: number; burst: number; errX: number; errY: number; loot: number; jumpT: number;
  /** Seconds until the bot may build again / since it last saw a fight. */
  buildT: number; calm: number;
  /** Harvest target (cylinder index, or -1) and a cover spot to run to while reloading. */
  harvest: number; coverX: number; coverZ: number; coverT: number;
}
export type FighterMode = 'bus' | 'sky' | 'glide' | 'ground' | 'dead';
export interface Fighter {
  id: number; fighter: FighterId; name: string; human: boolean; slot: number;
  x: number; y: number; z: number; vx: number; vy: number; vz: number; yaw: number; pitch: number; grounded: boolean;
  mode: FighterMode; hp: number; shield: number; kills: number; damage: number; place: number; killedBy: number; aliveTime: number;
  weapons: (Held | null)[]; current: number; ammo: Record<AmmoKind, number>;
  /** Wood, brick, metal. */
  mats: [number, number, number];
  building: boolean; piece: BuildKind; mat: Material; swimming: boolean;
  cooldown: number; reload: number; aiming: boolean; sprinting: boolean; moving: boolean; shootT: number; hurtT: number; bloom: number;
  /** Consumable progress (s) and pickaxe swing animation timer. */
  useT: number; swingT: number;
  interact: number; prevFire: boolean; prevJump: boolean; prevInteract: boolean; prevReload: boolean; prevEdit: boolean; stormT: number;
  bot: BotMemory;
}
/** heal / shield: instant forage; item: a consumable for the hotbar (`weapon` holds its id); mats: building materials (`mat`). */
export type LootKind = 'weapon' | 'ammo' | 'heal' | 'shield' | 'item' | 'mats';
export interface Loot { id: number; kind: LootKind; weapon: ItemId | null; rarity: Rarity; ammo: AmmoKind | null; mat: Material | null; amount: number; x: number; y: number; z: number }
export interface Rocket { id: number; owner: number; rarity: Rarity; x: number; y: number; z: number; vx: number; vy: number; vz: number; life: number }
export interface Storm { stage: number; cx: number; cz: number; r: number; fromX: number; fromZ: number; fromR: number; toX: number; toZ: number; toR: number; timer: number; shrinking: boolean; damage: number }
export type ShooterEventType = 'shot' | 'hit' | 'kill' | 'pickup' | 'chest' | 'explode' | 'reload' | 'land' | 'jump' | 'storm' | 'win' | 'glide' | 'build' | 'break' | 'harvest' | 'use' | 'swing' | 'edit';
export interface ShooterEvent { id: number; type: ShooterEventType; player: number; target: number; value: number; flag: number; x: number; y: number; z: number; x2: number; y2: number; z2: number }
export interface ShooterState {
  map: ShooterMapId; difficulty: Difficulty; phase: 'bus' | 'play' | 'done'; clock: number; endTimer: number;
  bus: { ax: number; az: number; bx: number; bz: number; t: number; duration: number };
  players: Fighter[]; loot: Loot[]; lootRev: number; chests: boolean[]; rockets: Rocket[]; storm: Storm;
  builds: Build[]; buildRev: number;
  events: ShooterEvent[]; eventSeq: number; nextId: number; rng: number; accumulator: number; winner: number;
}

/** Storm schedule: wait, shrink duration, final radius, damage per second (about seven minutes in all). */
export const STORM: { wait: number; shrink: number; radius: number; damage: number }[] = [
  { wait: 60, shrink: 40, radius: 165, damage: 1 },
  { wait: 45, shrink: 32, radius: 100, damage: 2 },
  { wait: 40, shrink: 26, radius: 60, damage: 4 },
  { wait: 32, shrink: 22, radius: 34, damage: 6 },
  { wait: 26, shrink: 18, radius: 16, damage: 8 },
  { wait: 20, shrink: 15, radius: 6, damage: 10 },
  { wait: 15, shrink: 12, radius: 0, damage: 14 },
];
const START_RADIUS = HALF * 1.45;
/** Kill-event weapon codes: weapon index, 6 storm, 7 pickaxe. */
export const KILL_STORM = 6;
export const KILL_PICKAXE = 7;

const emptyAmmo = (): Record<AmmoKind, number> => ({ light: 0, medium: 0, shells: 0, heavy: 0, rockets: 0 });
export const emptyBot = (think = 0): BotMemory => ({
  goalX: 0, goalZ: 0, think, target: -1, seen: 0, strafe: 1, strafeT: 0, stuck: 0, lastX: 0, lastZ: 0, dropAt: 0, burst: 0, errX: 0, errY: 0, loot: -1, jumpT: 0,
  buildT: 0, calm: 0, harvest: -1, coverX: 0, coverZ: 0, coverT: 0,
});
/** The hotbar item in hand (null with the pickaxe out or an empty slot). */
export const heldItem = (p: Pick<Fighter, 'weapons' | 'current'>): Held | null => (p.current >= 0 && p.current < SLOTS ? p.weapons[p.current] ?? null : null);

export function createMatch(setup: ShooterSetup): ShooterState {
  const map = mapFor(setup.map);
  const rng = new Rng(setup.seed);
  // Flight path across the island through a point near the centre.
  const angle = rng.range(0, Math.PI * 2), off = rng.range(-60, 60);
  const cx = Math.cos(angle + Math.PI / 2) * off, cz = Math.sin(angle + Math.PI / 2) * off;
  const reach = HALF + 20;
  const bus = { ax: cx - Math.cos(angle) * reach, az: cz - Math.sin(angle) * reach, bx: cx + Math.cos(angle) * reach, bz: cz + Math.sin(angle) * reach, t: 0, duration: (reach * 2) / BUS_SPEED };
  const players = setup.seats.slice(0, MAX_FIGHTERS).map((seat, i): Fighter => ({
    id: i, fighter: seat.fighter, name: seat.name, human: seat.human, slot: seat.slot,
    x: bus.ax, y: BUS_HEIGHT, z: bus.az, vx: 0, vy: 0, vz: 0, yaw: Math.atan2(Math.cos(angle), Math.sin(angle)), pitch: -0.3, grounded: false,
    mode: 'bus', hp: 100, shield: 0, kills: 0, damage: 0, place: 0, killedBy: -1, aliveTime: 0,
    weapons: Array.from({ length: SLOTS }, () => null), current: PICKAXE_SLOT, ammo: emptyAmmo(), mats: [0, 0, 0],
    building: false, piece: 0, mat: 0, swimming: false,
    cooldown: 0, reload: 0, aiming: false, sprinting: false, moving: false, shootT: 0, hurtT: 0, bloom: 0, useT: 0, swingT: 0,
    interact: 0, prevFire: false, prevJump: false, prevInteract: false, prevReload: false, prevEdit: false, stormT: 0,
    bot: emptyBot(rng.range(0, 0.3)),
  }));
  for (const p of players) {
    if (!p.human) {
      // Bots pick a landing spot and jump when the flight passes closest to it.
      const spot = rng.chance(0.75) ? rng.pick(map.lootSpots) : rng.pick(map.pois);
      p.bot.goalX = spot.x + rng.range(-6, 6); p.bot.goalZ = spot.z + rng.range(-6, 6);
      const dx = bus.bx - bus.ax, dz = bus.bz - bus.az, len = Math.hypot(dx, dz);
      const t = clamp(((p.bot.goalX - bus.ax) * dx + (p.bot.goalZ - bus.az) * dz) / (len * len), 0.05, 0.92);
      p.bot.dropAt = t * bus.duration + rng.range(-1.5, 1.5);
    }
  }
  const state: ShooterState = {
    map: map.id, difficulty: setup.difficulty, phase: 'bus', clock: 0, endTimer: 0, bus, players,
    loot: [], lootRev: 1, chests: map.chests.map(() => false), rockets: [], builds: [], buildRev: 1,
    storm: { stage: -1, cx: 0, cz: 0, r: START_RADIUS, fromX: 0, fromZ: 0, fromR: START_RADIUS, toX: 0, toZ: 0, toR: START_RADIUS, timer: STORM[0].wait, shrinking: false, damage: 0 },
    events: [], eventSeq: 0, nextId: 1, rng: 0, accumulator: 0, winner: -1,
  };
  for (const spot of map.lootSpots) {
    if (!rng.chance(0.85)) continue;
    const y = spot.y ?? terrainHeight(map, spot.x, spot.z);
    const roll = rng.next();
    if (roll < 0.52) {
      const weapon = rollWeapon(rng.next());
      dropItem(state, 'weapon', weapon, rollRarity(rng.next()), WEAPONS[weapon].mag, spot.x, y, spot.z);
      dropLoot(state, 'ammo', WEAPONS[weapon].ammo, AMMO_PACK[WEAPONS[weapon].ammo], spot.x + 0.9, y, spot.z + 0.4);
    } else if (roll < 0.74) {
      const item = rollConsumable(rng.next());
      dropItem(state, 'item', item, CONSUMABLES[item].rarity, CONSUMABLE_PACK[item], spot.x, y, spot.z);
    } else if (roll < 0.86) { const kind = rng.pick(AMMO_KINDS); dropLoot(state, 'ammo', kind, AMMO_PACK[kind], spot.x, y, spot.z); }
    else if (roll < 0.94) dropMats(state, rng.int(3) as Material, 30, spot.x, y, spot.z);
    else dropLoot(state, rng.chance(0.5) ? 'heal' : 'shield', null, 25, spot.x, y, spot.z);
  }
  pickStormTarget(state, rng);
  state.rng = rng.state;
  return state;
}

function emit(state: ShooterState, type: ShooterEventType, player: number, x: number, y: number, z: number, extra: Partial<ShooterEvent> = {}): void {
  state.events.push({ id: ++state.eventSeq, type, player, target: -1, value: 0, flag: 0, x, y, z, x2: 0, y2: 0, z2: 0, ...extra });
  if (state.events.length > 64) state.events.shift();
}
function dropLoot(state: ShooterState, kind: LootKind, ammo: AmmoKind | null, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind, weapon: null, rarity: 0, ammo, mat: null, amount, x, y, z });
  state.lootRev++;
}
function dropItem(state: ShooterState, kind: 'weapon' | 'item', item: ItemId, rarity: Rarity, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind, weapon: item, rarity, ammo: null, mat: null, amount, x, y, z });
  state.lootRev++;
}
function dropMats(state: ShooterState, mat: Material, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT || amount <= 0) return;
  state.loot.push({ id: state.nextId++, kind: 'mats', weapon: null, rarity: 0, ammo: null, mat, amount, x, y, z });
  state.lootRev++;
}
/** Drops a hotbar item as floor loot. */
const dropHeld = (state: ShooterState, h: Held, x: number, y: number, z: number) => dropItem(state, isWeapon(h.w) ? 'weapon' : 'item', h.w, h.rarity, h.mag, x, y, z);
function takeLoot(state: ShooterState, index: number): void { state.loot.splice(index, 1); state.lootRev++; }

function pickStormTarget(state: ShooterState, rng: Rng): void {
  const s = state.storm;
  const next = STORM[Math.min(STORM.length - 1, s.stage + 1)];
  const map = mapFor(state.map);
  // New circle inside the current one, centre kept on dry land.
  for (let tries = 0; tries < 30; tries++) {
    const room = Math.max(0, Math.min(s.r, START_RADIUS * 0.7) - next.radius);
    const a = rng.range(0, Math.PI * 2), d = rng.range(0, room * 0.85);
    const x = clamp(s.cx + Math.cos(a) * d, -HALF * 0.7, HALF * 0.7), z = clamp(s.cz + Math.sin(a) * d, -HALF * 0.7, HALF * 0.7);
    if (dryLand(map, x, z) || tries === 29) { s.toX = x; s.toZ = z; break; }
  }
  s.toR = next.radius;
}

export function aliveCount(state: ShooterState): number { let n = 0; for (const p of state.players) if (p.mode !== 'dead') n++; return n; }

export function stepMatch(state: ShooterState, inputs: (ShooterInput | undefined)[], dt: number): void {
  if (!Number.isFinite(dt) || dt <= 0) return;
  state.accumulator = Math.min(state.accumulator + Math.min(dt, 0.1), STEP * 6);
  while (state.accumulator >= STEP - 1e-9) { state.accumulator -= STEP; tickMatch(state, inputs); }
}

export function sanitizeInput(i: ShooterInput | undefined): ShooterInput {
  if (!i) return idleShooterInput();
  const n = (v: unknown, lim: number) => (typeof v === 'number' && Number.isFinite(v) ? clamp(v, -lim, lim) : 0);
  const pick = (v: unknown, max: number) => (Number.isInteger(v) && (v as number) >= 0 && (v as number) <= max ? (v as number) : 0);
  return {
    mx: n(i.mx, 1), mz: n(i.mz, 1), yaw: wrapAngle(n(i.yaw, 100)), pitch: n(i.pitch, 1.45),
    fire: i.fire === true, aim: i.aim === true, jump: i.jump === true, sprint: i.sprint === true, reload: i.reload === true, interact: i.interact === true,
    slot: pick(i.slot, PICKAXE_KEY), build: i.build === true, piece: pick(i.piece, 2), mat: pick(i.mat, 2), edit: i.edit === true,
  };
}

export function tickMatch(state: ShooterState, inputs: (ShooterInput | undefined)[]): void {
  if (state.phase === 'done') return;
  const map = mapFor(state.map), world = worldFor(map);
  const rng = new Rng(state.rng || 1);
  state.clock += STEP;
  if (state.phase === 'bus') {
    state.bus.t += STEP;
    if (state.players.every(p => p.mode !== 'bus') || state.bus.t > state.bus.duration) state.phase = 'play';
  }
  for (const p of state.players) {
    if (p.mode === 'dead') continue;
    p.aliveTime += STEP;
    const input = p.human ? sanitizeInput(inputs[p.id]) : botThink(state, map, world, p, rng);
    stepFighter(state, map, world, p, input, rng);
  }
  stepRockets(state, world);
  growBuilds(state);
  stepStorm(state, rng);
  const alive = aliveCount(state);
  const humansAlive = state.players.some(p => p.human && p.mode !== 'dead');
  const hadHumans = state.players.some(p => p.human);
  if ((alive <= 1 || (hadHumans && !humansAlive)) && state.phase === 'play') {
    if (state.endTimer === 0) {
      const last = state.players.filter(p => p.mode !== 'dead');
      if (alive <= 1 && last[0]) { last[0].place = 1; state.winner = last[0].id; emit(state, 'win', last[0].id, last[0].x, last[0].y, last[0].z); }
    }
    state.endTimer += STEP;
    if (state.endTimer > 3) {
      // Bots still standing when every human is out share the remaining places.
      const standing = state.players.filter(p => p.mode !== 'dead' && p.place === 0).sort((a, b) => b.hp + b.shield - (a.hp + a.shield));
      standing.forEach((p, i) => { p.place = i + 1; });
      if (state.winner < 0 && standing[0]) state.winner = standing[0].id;
      state.phase = 'done';
    }
  }
  state.rng = rng.state;
}

const fwd = (yaw: number): [number, number] => [Math.sin(yaw), Math.cos(yaw)];

function stepFighter(state: ShooterState, map: ShooterMap, world: World, p: Fighter, input: ShooterInput, rng: Rng): void {
  p.cooldown = Math.max(0, p.cooldown - STEP);
  p.shootT = Math.max(0, p.shootT - STEP);
  p.hurtT = Math.max(0, p.hurtT - STEP);
  p.swingT = Math.max(0, p.swingT - STEP);
  p.bloom = Math.max(0, p.bloom - STEP * 0.25);
  p.yaw = input.yaw; p.pitch = input.pitch;
  const jumpPressed = input.jump && !p.prevJump, interactPressed = input.interact && !p.prevInteract, editPressed = input.edit && !p.prevEdit;
  p.prevJump = input.jump; p.prevInteract = input.interact; p.prevEdit = input.edit;
  const [fx, fz] = fwd(input.yaw);
  // Right of facing (fx, fz) is (-fz, fx) because +X is on the left when facing +Z.
  const wx = fx * input.mz - fz * input.mx, wz = fz * input.mz + fx * input.mx;
  const wl = Math.hypot(wx, wz);
  const dirX = wl > 1 ? wx / wl : wx, dirZ = wl > 1 ? wz / wl : wz;

  if (p.mode === 'bus') {
    const t = clamp(state.bus.t / state.bus.duration, 0, 1);
    p.x = state.bus.ax + (state.bus.bx - state.bus.ax) * t; p.z = state.bus.az + (state.bus.bz - state.bus.az) * t; p.y = BUS_HEIGHT;
    const autoJump = state.bus.t > state.bus.duration - 0.6;
    if ((p.human && (jumpPressed || interactPressed)) || (!p.human && state.bus.t >= p.bot.dropAt) || autoJump) {
      p.mode = 'sky';
      const [bx, bz] = [(state.bus.bx - state.bus.ax) / (state.bus.duration * BUS_SPEED), (state.bus.bz - state.bus.az) / (state.bus.duration * BUS_SPEED)];
      p.vx = bx * 12; p.vz = bz * 12; p.vy = 0; p.y -= 3;
      emit(state, 'jump', p.id, p.x, p.y, p.z);
    }
    return;
  }
  if (p.mode === 'sky' || p.mode === 'glide') {
    const g = terrainHeight(map, p.x, p.z);
    if (p.mode === 'sky' && p.y - g < 34) { p.mode = 'glide'; emit(state, 'glide', p.id, p.x, p.y, p.z); }
    const glide = p.mode === 'glide';
    const maxH = glide ? 11 : 17, fall = glide ? -6.5 : -32 - Math.max(0, input.mz) * 6;
    p.vx = approach(p.vx, dirX * maxH, glide ? 1.6 : 1.2, STEP);
    p.vz = approach(p.vz, dirZ * maxH, glide ? 1.6 : 1.2, STEP);
    p.vy = approach(p.vy, fall, glide ? 3 : 1.4, STEP);
    p.x = clamp(p.x + p.vx * STEP, -HALF + 4, HALF - 4); p.z = clamp(p.z + p.vz * STEP, -HALF + 4, HALF - 4); p.y += p.vy * STEP;
    const floor = groundAt(world, p.x, p.z, p.y + 50, state.builds), surface = waterLevel(map, p.x, p.z) - SWIM_DEPTH;
    const land = Math.max(floor, surface);
    if (p.y <= land) {
      p.y = land; p.mode = 'ground'; p.grounded = true; p.vy = 0; p.vx *= 0.3; p.vz *= 0.3; p.swimming = surface > floor;
      resolve(world, p, state.builds);
      emit(state, 'land', p.id, p.x, p.y, p.z);
    }
    return;
  }

  // Ground movement.
  if (input.slot >= 1 && input.slot <= PICKAXE_KEY && input.slot - 1 !== p.current) { p.current = input.slot - 1; p.reload = 0; p.useT = 0; p.cooldown = Math.max(p.cooldown, 0.25); }
  const held = heldItem(p);
  p.building = input.build && !p.swimming;
  p.piece = input.piece as BuildKind; p.mat = input.mat as Material;
  const using = !p.building && !!held && isConsumable(held.w) && input.fire && canUse(held.w, p.hp, p.shield);
  p.aiming = input.aim && p.grounded && !p.building && !p.swimming;
  p.sprinting = input.sprint && input.mz > 0.3 && !input.fire && !p.aiming && !p.swimming && !using;
  const speed = (p.aiming ? 3.6 : p.sprinting ? 7.8 : 5.4) * (p.swimming ? 0.55 : using ? 0.55 : 1);
  p.moving = wl > 0.1;
  const accel = p.grounded ? 14 : 3;
  p.vx = approach(p.vx, dirX * speed, accel, STEP);
  p.vz = approach(p.vz, dirZ * speed, accel, STEP);
  if (p.grounded && jumpPressed) { p.vy = 7.4; p.grounded = false; }
  p.vy -= 22 * STEP;
  p.x += p.vx * STEP; p.z += p.vz * STEP;
  resolve(world, p, state.builds);
  const g = groundAt(world, p.x, p.z, p.y, state.builds);
  p.y += p.vy * STEP;
  if (p.y <= g) {
    if (!p.grounded && p.vy < -14) { const fallDmg = Math.round((-p.vy - 14) * 4); if (fallDmg > 0) hurt(state, p, fallDmg, -1, false, false, 'pistol'); }
    p.y = g; p.vy = 0; p.grounded = true;
  } else if (p.grounded && p.vy <= 0 && p.y - g < 0.45) {
    // Walking down stairs, ramps and slopes: stay on the ground.
    p.y = g; p.vy = 0;
  } else if (p.y > g + 0.05) p.grounded = false;
  // Water: float at the surface and swim (slow, no sprint, no shooting or building).
  const surface = waterLevel(map, p.x, p.z) - SWIM_DEPTH;
  p.swimming = p.y <= surface + 0.02 && g < surface;
  if (p.swimming) { p.y = Math.max(p.y, surface); if (p.vy < 0) p.vy = 0; p.grounded = true; }

  // Interact: chests and item swaps.
  pickupAuto(state, p);
  if (input.interact) {
    const chest = nearestChest(state, map, p, 2.4);
    if (chest >= 0) {
      p.interact += STEP;
      if (p.interact >= 0.55) { openChest(state, map, chest, rng); p.interact = 0; }
    } else if (interactPressed) swapWeapon(state, p);
  } else p.interact = 0;

  const reloadPressed = input.reload && !p.prevReload;
  p.prevReload = input.reload;
  const firePressed = input.fire && !p.prevFire;
  p.prevFire = input.fire;
  if (p.building) {
    p.reload = 0; p.useT = 0;
    if (input.fire && p.cooldown <= 0) tryBuild(state, world, p);
    if (editPressed) tryEdit(state, world, p);
    return;
  }
  if (p.current === PICKAXE_SLOT) {
    if (input.fire && p.cooldown <= 0 && !p.swimming) swing(state, map, world, p);
    return;
  }
  if (!held) return;
  if (isConsumable(held.w)) {
    if (!using) { p.useT = 0; return; }
    p.useT += STEP;
    const c = CONSUMABLES[held.w];
    if (p.useT >= c.time) {
      if (c.heal) p.hp = Math.min(Math.max(p.hp, c.cap), p.hp + c.heal);
      if (c.shield) p.shield = Math.min(Math.max(p.shield, c.cap), p.shield + c.shield);
      held.mag--;
      if (held.mag <= 0) p.weapons[p.current] = null;
      p.useT = 0;
      emit(state, 'use', p.id, p.x, p.y, p.z, { value: c.heal ? 0 : 1 });
    }
    return;
  }
  // Weapons: reload, then fire.
  const def = WEAPONS[held.w];
  if (p.reload > 0) {
    p.reload -= STEP;
    if (p.reload <= 0) {
      const take = Math.min(def.mag - held.mag, p.ammo[def.ammo]);
      held.mag += take; p.ammo[def.ammo] -= take; p.reload = 0;
    }
  } else if ((reloadPressed || (held.mag === 0 && input.fire)) && held.mag < def.mag && p.ammo[def.ammo] > 0) {
    p.reload = def.reload;
    emit(state, 'reload', p.id, p.x, p.y, p.z);
  }
  if (input.fire && p.reload <= 0 && p.cooldown <= 0 && held.mag > 0 && !p.sprinting && !p.swimming && (def.auto || firePressed)) fire(state, map, world, p, held, held.w, rng);
}

// ------------------------------------------------------------ building

/** Static ground (terrain + props) for structural support checks. */
const groundFor = (world: World) => (x: number, z: number, feet: number) => groundAt(world, x, z, feet);

function tryBuild(state: ShooterState, world: World, p: Fighter): void {
  if (p.mats[p.mat] < BUILD_COST || state.builds.length >= MAX_BUILDS) return;
  const slot = planBuild(p.piece, p.x, p.y, p.z, p.yaw, p.pitch, state.builds);
  if (!canPlace(slot, state.builds, groundFor(world))) return;
  const b = makeBuild(state.nextId++, slot, p.mat, p.id);
  state.builds.push(b);
  state.buildRev++;
  p.mats[p.mat] -= BUILD_COST;
  p.cooldown = 0.12;
  const a = b.aabb;
  emit(state, 'build', p.id, (a.minX + a.maxX) / 2, a.minY, (a.minZ + a.maxZ) / 2, { value: b.kind, flag: b.mat });
}

/** Edits the survivor's own structure in the crosshair: walls cycle solid → door → window, ramps flip. */
function tryEdit(state: ShooterState, world: World, p: Fighter): void {
  const [fx, fz] = fwd(p.yaw), cp = Math.cos(p.pitch);
  const hit = raycast(world, p.x, p.y + EYE, p.z, fx * cp, Math.sin(p.pitch), fz * cp, 6, true, state.builds);
  if (hit.kind !== 'build') return;
  const b = state.builds[hit.index];
  if (b.owner !== p.id || b.kind === 1) return;
  if (b.kind === 0) b.variant = (b.variant + 1) % 3;
  else b.dir = (b.dir + 2) % 4;
  refreshGeometry(b);
  state.buildRev++;
  p.cooldown = Math.max(p.cooldown, 0.2);
  emit(state, 'edit', p.id, (b.aabb.minX + b.aabb.maxX) / 2, b.aabb.minY + 1.5, (b.aabb.minZ + b.aabb.maxZ) / 2, { value: b.kind, flag: b.variant });
}

/** Damages a structure; destroyed pieces fall, and so does anything they were holding up. */
function damageBuild(state: ShooterState, world: World, b: Build, amount: number): void {
  b.hp -= amount;
  state.buildRev++;
  if (b.hp > 0) return;
  removeBuilds(state, [state.builds.indexOf(b)]);
  const falling = unsupported(state.builds, groundFor(world));
  if (falling.length) removeBuilds(state, falling);
}
function removeBuilds(state: ShooterState, indices: number[]): void {
  for (let k = indices.length - 1; k >= 0; k--) {
    const i = indices[k];
    if (i < 0) continue;
    const b = state.builds[i], a = b.aabb;
    state.builds.splice(i, 1);
    emit(state, 'break', b.owner, (a.minX + a.maxX) / 2, (a.minY + a.maxY) / 2, (a.minZ + a.maxZ) / 2, { value: b.kind, flag: b.mat });
  }
  state.buildRev++;
}
function growBuilds(state: ShooterState): void {
  for (const b of state.builds) {
    if (b.grow <= 0) continue;
    const m = MATERIALS[b.mat];
    b.grow = Math.max(0, b.grow - STEP);
    b.hp = Math.min(maxHp(b), b.hp + (m.hp * (1 - m.start) / m.time) * STEP);
    // Finished going up: guests and the renderer drop the "under construction" look.
    if (b.grow === 0) state.buildRev++;
  }
}

/** Pickaxe: hurts a fighter, chips a structure, or harvests the prop in front. */
function swing(state: ShooterState, map: ShooterMap, world: World, p: Fighter): void {
  p.cooldown = 1 / PICKAXE.rate;
  p.swingT = 0.35;
  const [fx, fz] = fwd(p.yaw), cp = Math.cos(p.pitch);
  const ox = p.x, oy = p.y + EYE, oz = p.z, dx = fx * cp, dy = Math.sin(p.pitch), dz = fz * cp;
  const hit = raycast(world, ox, oy, oz, dx, dy, dz, PICKAXE.range, true, state.builds);
  let victim = -1, tEnd = hit.t;
  for (const q of state.players) {
    if (q.id === p.id || q.mode !== 'ground') continue;
    const h = rayFighter(ox, oy, oz, dx, dy, dz, q, tEnd);
    if (h.t >= 0 && h.t < tEnd) { tEnd = h.t; victim = q.id; }
  }
  emit(state, 'swing', p.id, ox + dx * tEnd, oy + dy * tEnd, oz + dz * tEnd, { value: victim >= 0 ? 1 : hit.kind === 'none' ? 0 : 2 });
  if (victim >= 0) { hurt(state, state.players[victim], PICKAXE.damage, p.id, false, true, 'pickaxe'); return; }
  if (hit.kind === 'build') { damageBuild(state, world, state.builds[hit.index], PICKAXE.structure); return; }
  if (hit.kind !== 'prop') return;
  const mat = hit.cyl ? 0 : map.boxMats[hit.index];
  if (mat < 0) return;
  const before = p.mats[mat];
  p.mats[mat] = Math.min(MAX_MATS, before + PICKAXE.harvest);
  emit(state, 'harvest', p.id, ox + dx * tEnd, oy + dy * tEnd, oz + dz * tEnd, { value: mat, flag: p.mats[mat] - before });
}

// ------------------------------------------------------------ loot

function nearestChest(state: ShooterState, map: ShooterMap, p: Fighter, range: number): number {
  let best = -1, bd = range;
  map.chests.forEach((c, i) => {
    if (state.chests[i] || Math.abs(chestY(map, c) - p.y) > 2.2) return;
    const d = Math.hypot(c.x - p.x, c.z - p.z);
    if (d < bd) { bd = d; best = i; }
  });
  return best;
}
function openChest(state: ShooterState, map: ShooterMap, index: number, rng: Rng): void {
  const c = map.chests[index];
  state.chests[index] = true;
  state.lootRev++;
  const y = chestY(map, c);
  const weapon = rollWeapon(rng.next());
  const ox = Math.sin(c.yaw) * 1.6, oz = Math.cos(c.yaw) * 1.6;
  dropItem(state, 'weapon', weapon, rollRarity(rng.next(), 1.2), WEAPONS[weapon].mag, c.x + ox, y, c.z + oz);
  dropLoot(state, 'ammo', WEAPONS[weapon].ammo, AMMO_PACK[WEAPONS[weapon].ammo] * 2, c.x + ox + 1, y, c.z + oz - 0.6);
  const item = rollConsumable(rng.next());
  dropItem(state, 'item', item, CONSUMABLES[item].rarity, CONSUMABLE_PACK[item], c.x + ox - 1, y, c.z + oz + 0.6);
  dropMats(state, 0, 30, c.x + ox, y, c.z + oz + 1.2);
  emit(state, 'chest', -1, c.x, y + 0.6, c.z, { value: index });
}

function pickupAuto(state: ShooterState, p: Fighter): void {
  for (let i = state.loot.length - 1; i >= 0; i--) {
    const l = state.loot[i];
    const dx = l.x - p.x, dz = l.z - p.z;
    if (dx * dx + dz * dz > 1.6 * 1.6 || Math.abs(l.y - p.y) > 2) continue;
    if (l.kind === 'ammo' && l.ammo) {
      const room = AMMO_MAX[l.ammo] - p.ammo[l.ammo];
      if (room <= 0) continue;
      const take = Math.min(room, l.amount);
      p.ammo[l.ammo] += take; l.amount -= take; state.lootRev++;
      if (l.amount <= 0) takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 1 });
    } else if (l.kind === 'mats' && l.mat !== null) {
      const room = MAX_MATS - p.mats[l.mat];
      if (room <= 0) continue;
      const take = Math.min(room, l.amount);
      p.mats[l.mat] += take; l.amount -= take; state.lootRev++;
      if (l.amount <= 0) takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 4 });
    } else if (l.kind === 'heal' && p.hp < 100) {
      p.hp = Math.min(100, p.hp + l.amount); takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 2 });
    } else if (l.kind === 'shield' && p.shield < 100) {
      p.shield = Math.min(100, p.shield + l.amount); takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 3 });
    } else if (l.kind === 'item' && isConsumable(l.weapon)) {
      // Top up a stack of the same item, then use a free slot.
      const stack = CONSUMABLES[l.weapon].stack, before = l.amount;
      for (const h of p.weapons) {
        if (!h || h.w !== l.weapon || h.mag >= stack || l.amount <= 0) continue;
        const take = Math.min(stack - h.mag, l.amount);
        h.mag += take; l.amount -= take;
      }
      const free = p.weapons.findIndex(w => w === null);
      if (l.amount > 0 && free >= 0) { p.weapons[free] = { w: l.weapon, rarity: l.rarity, mag: l.amount }; l.amount = 0; }
      if (l.amount === before) continue;
      state.lootRev++;
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 5 });
      if (l.amount <= 0) takeLoot(state, i);
    } else if (l.kind === 'weapon' && isWeapon(l.weapon)) {
      const free = p.weapons.findIndex(w => w === null);
      if (free < 0 || p.weapons.some(w => w?.w === l.weapon && w.rarity >= l.rarity)) continue;
      p.weapons[free] = { w: l.weapon, rarity: l.rarity, mag: l.amount };
      // Holding the pickaxe or an empty slot: draw the new weapon.
      if (!heldItem(p) && !p.building) p.current = free;
      takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 0 });
    }
  }
}

/** Nearest weapon or consumable on the floor within reach, for the "E to pick up" prompt. */
export function nearestWeaponLoot(state: ShooterState, p: Fighter, range = 2.3): Loot | null {
  let best: Loot | null = null, bd = range;
  for (const l of state.loot) if (l.kind === 'weapon' || l.kind === 'item') { const d = Math.hypot(l.x - p.x, l.z - p.z); if (d < bd && Math.abs(l.y - p.y) < 2) { bd = d; best = l; } }
  return best;
}
export function nearestClosedChest(state: ShooterState, p: Fighter, range = 2.4): number { return nearestChest(state, mapFor(state.map), p, range); }

function swapWeapon(state: ShooterState, p: Fighter): void {
  const l = nearestWeaponLoot(state, p);
  if (!l || !l.weapon) return;
  const free = p.weapons.findIndex(w => w === null);
  const slot = free >= 0 ? free : p.current < SLOTS ? p.current : 0;
  const old = p.weapons[slot];
  p.weapons[slot] = { w: l.weapon, rarity: l.rarity, mag: l.amount };
  p.current = slot; p.reload = 0; p.useT = 0;
  takeLoot(state, state.loot.indexOf(l));
  if (old) dropHeld(state, old, p.x - Math.sin(p.yaw) * 1.2, p.y, p.z - Math.cos(p.yaw) * 1.2);
  emit(state, 'pickup', p.id, p.x, p.y, p.z, { value: 0 });
}

// ------------------------------------------------------------ combat

export const EYE = 1.55;
const _buildHits: Build[] = [];
const _buildDmg: number[] = [];
function fire(state: ShooterState, map: ShooterMap, world: World, p: Fighter, held: Held, w: WeaponId, rng: Rng): void {
  const def = WEAPONS[w];
  held.mag--;
  p.cooldown = 1 / def.rate;
  p.shootT = 0.25;
  const [fx, fz] = fwd(p.yaw);
  // Muzzle sits over the right shoulder; right of facing (fx, fz) is (-fz, fx).
  const ox = p.x - fz * 0.3, oy = p.y + EYE, oz = p.z + fx * 0.3;
  const cp = Math.cos(p.pitch);
  const baseX = fx * cp, baseY = Math.sin(p.pitch), baseZ = fz * cp;
  const spread = (p.aiming ? def.adsSpread : def.spread) * (p.moving ? 1.4 : 1) * (p.grounded ? 1 : 2) + p.bloom;
  p.bloom = Math.min(0.05, p.bloom + def.recoil * 0.5);
  if (def.projectile) {
    if (state.rockets.length < MAX_ROCKETS) {
      const s = def.projectile.speed;
      state.rockets.push({ id: state.nextId++, owner: p.id, rarity: held.rarity, x: ox + baseX, y: oy + baseY, z: oz + baseZ, vx: baseX * s, vy: baseY * s, vz: baseZ * s, life: 5 });
    }
    emit(state, 'shot', p.id, ox, oy, oz, { value: 5, x2: ox + baseX * 3, y2: oy + baseY * 3, z2: oz + baseZ * 3 });
    return;
  }
  let firstEnd: [number, number, number] | null = null;
  _buildHits.length = 0; _buildDmg.length = 0;
  for (let k = 0; k < def.pellets; k++) {
    // Random direction inside the spread cone.
    const a = rng.range(0, Math.PI * 2), r = Math.sqrt(rng.next()) * spread;
    const ux = -fz, uz = fx; // horizontal perpendicular
    let dx = baseX + (ux * Math.cos(a) + 0) * r, dy = baseY + Math.sin(a) * r, dz = baseZ + uz * Math.cos(a) * r;
    const l = Math.hypot(dx, dy, dz); dx /= l; dy /= l; dz /= l;
    const wall = raycast(world, ox, oy, oz, dx, dy, dz, def.range, true, state.builds);
    let tEnd = wall.t, victim = -1, head = false;
    const struct = wall.kind === 'build' ? state.builds[wall.index] : null;
    for (const q of state.players) {
      if (q.id === p.id || q.mode === 'dead' || q.mode === 'bus') continue;
      const t = rayFighter(ox, oy, oz, dx, dy, dz, q, tEnd);
      if (t.t >= 0 && t.t < tEnd) { tEnd = t.t; victim = q.id; head = t.head; }
    }
    const ex = ox + dx * tEnd, ey = oy + dy * tEnd, ez = oz + dz * tEnd;
    if (!firstEnd) firstEnd = [ex, ey, ez];
    if (victim >= 0) {
      const q = state.players[victim];
      hurt(state, q, damageAt(def, held.rarity, tEnd, head), p.id, head, true, w);
    } else if (struct) {
      const at = _buildHits.indexOf(struct);
      const dmg = damageAt(def, held.rarity, tEnd, false);
      if (at < 0) { _buildHits.push(struct); _buildDmg.push(dmg); } else _buildDmg[at] += dmg;
    }
  }
  for (let i = 0; i < _buildHits.length; i++) if (state.builds.includes(_buildHits[i])) damageBuild(state, world, _buildHits[i], _buildDmg[i]);
  const e = firstEnd!;
  emit(state, 'shot', p.id, ox, oy, oz, { value: WEAPON_IDS.indexOf(w), x2: e[0], y2: e[1], z2: e[2] });
  void map;
}

const _fh = { t: -1, head: false };
const SPHERES: [number, number, boolean][] = [[1.62, 0.3, true], [1.15, 0.46, false], [0.55, 0.42, false]];
/** Ray vs fighter: head sphere + two body spheres. */
export function rayFighter(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, q: { x: number; y: number; z: number }, max: number): { t: number; head: boolean } {
  _fh.t = -1; _fh.head = false;
  for (const [h, r, head] of SPHERES) {
    const cx = q.x - ox, cy = q.y + h - oy, cz = q.z - oz;
    const b = cx * dx + cy * dy + cz * dz;
    if (b < 0) continue;
    const c = cx * cx + cy * cy + cz * cz - r * r;
    const disc = b * b - c;
    if (disc < 0) continue;
    const t = b - Math.sqrt(disc);
    if (t >= 0 && t < max && (_fh.t < 0 || t < _fh.t)) { _fh.t = t; _fh.head = head; }
  }
  return _fh;
}

function hurt(state: ShooterState, q: Fighter, amount: number, from: number, head: boolean, useShield: boolean, weapon: WeaponId | 'pickaxe'): void {
  // Once the last survivor is crowned nothing can take it away (storm, splash, falls).
  if (q.mode === 'dead' || amount <= 0 || state.phase === 'done' || state.winner >= 0) return;
  const res = useShield ? applyDamage(q.hp, q.shield, amount) : { hp: Math.max(0, q.hp - amount), shield: q.shield, absorbed: 0 };
  q.hp = res.hp; q.shield = res.shield; q.hurtT = 0.4; q.useT = 0;
  if (from >= 0 && from !== q.id) { state.players[from].damage += amount; if (!q.human) { q.bot.target = from; q.bot.seen = 0.6; q.bot.calm = 0; } }
  emit(state, 'hit', from, q.x, q.y + (head ? 1.7 : 1.1), q.z, { target: q.id, value: amount, flag: (res.absorbed > 0 ? 1 : 0) | (head ? 2 : 0) });
  if (q.hp <= 0) eliminate(state, q, from, weapon);
}

function eliminate(state: ShooterState, q: Fighter, from: number, weapon: WeaponId | 'pickaxe'): void {
  q.place = aliveCount(state);
  q.mode = 'dead'; q.killedBy = from; q.hp = 0; q.shield = 0; q.building = false; q.useT = 0;
  if (from >= 0 && from !== q.id) state.players[from].kills++;
  emit(state, 'kill', from, q.x, q.y, q.z, { target: q.id, value: from === -2 ? KILL_STORM : weapon === 'pickaxe' ? KILL_PICKAXE : WEAPON_IDS.indexOf(weapon) });
  // Drop the inventory in a little ring.
  let k = 0;
  const ring = (i: number): [number, number] => [q.x + Math.cos(i * 1.1) * (1.3 + i * 0.12), q.z + Math.sin(i * 1.1) * (1.3 + i * 0.12)];
  const y = q.y;
  for (const w of q.weapons) if (w) { const [x, z] = ring(k++); dropHeld(state, w, x, y, z); }
  for (const kind of AMMO_KINDS) if (q.ammo[kind] > 0) { const [x, z] = ring(k++); dropLoot(state, 'ammo', kind, q.ammo[kind], x, y, z); }
  for (let m = 0; m < 3; m++) if (q.mats[m] > 0) { const [x, z] = ring(k++); dropMats(state, m as Material, q.mats[m], x, y, z); }
  // Every elimination leaves a small heal for whoever won the fight.
  { const [x, z] = ring(k++); dropLoot(state, 'heal', null, 25, x, y, z); }
  q.weapons = Array.from({ length: SLOTS }, () => null);
  q.current = PICKAXE_SLOT;
  q.ammo = emptyAmmo();
  q.mats = [0, 0, 0];
}

function stepRockets(state: ShooterState, world: World): void {
  for (let i = state.rockets.length - 1; i >= 0; i--) {
    const r = state.rockets[i];
    r.life -= STEP;
    r.vy -= 2 * STEP;
    const sp = Math.hypot(r.vx, r.vy, r.vz), dx = r.vx / sp, dy = r.vy / sp, dz = r.vz / sp, step = sp * STEP;
    const hit = raycast(world, r.x, r.y, r.z, dx, dy, dz, step, true, state.builds);
    let t = hit.kind === 'none' ? -1 : hit.t;
    for (const q of state.players) {
      if (q.id === r.owner || q.mode === 'dead' || q.mode === 'bus') continue;
      const h = rayFighter(r.x, r.y, r.z, dx, dy, dz, q, step);
      if (h.t >= 0 && (t < 0 || h.t < t)) t = h.t;
    }
    if (t >= 0 || r.life <= 0) {
      const tt = Math.max(0, t);
      const ex = r.x + dx * tt, ey = r.y + dy * tt, ez = r.z + dz * tt;
      explode(state, world, r, ex, ey, ez);
      state.rockets.splice(i, 1);
    } else { r.x += r.vx * STEP; r.y += r.vy * STEP; r.z += r.vz * STEP; }
  }
}
function explode(state: ShooterState, world: World, r: Rocket, x: number, y: number, z: number): void {
  const def = WEAPONS.rocket, radius = def.projectile!.radius;
  emit(state, 'explode', r.owner, x, y, z, { value: radius });
  for (const q of state.players) {
    if (q.mode === 'dead' || q.mode === 'bus') continue;
    const d = Math.hypot(q.x - x, q.y + 1 - y, q.z - z);
    if (d > radius) continue;
    const amount = damageAt(def, r.rarity, 0, false) * (1 - (d / radius) * 0.7) * (q.id === r.owner ? 0.5 : 1);
    hurt(state, q, Math.round(amount), r.owner, false, true, 'rocket');
  }
  // Rockets are the structure breakers: heavy damage to every piece in the blast.
  _buildHits.length = 0; _buildDmg.length = 0;
  for (const b of state.builds) {
    const a = b.aabb;
    const d = Math.hypot(clamp(x, a.minX, a.maxX) - x, clamp(y, a.minY, a.maxY) - y, clamp(z, a.minZ, a.maxZ) - z);
    if (d <= radius) { _buildHits.push(b); _buildDmg.push(Math.round(320 * (1 - (d / radius) * 0.6))); }
  }
  const hits = _buildHits.slice(), dmg = _buildDmg.slice();
  for (let i = 0; i < hits.length; i++) if (state.builds.includes(hits[i])) damageBuild(state, world, hits[i], dmg[i]);
}

function stepStorm(state: ShooterState, rng: Rng): void {
  if (state.phase !== 'play') return;
  const s = state.storm;
  s.timer -= STEP;
  if (!s.shrinking && s.timer <= 0) {
    const next = STORM[Math.min(STORM.length - 1, s.stage + 1)];
    s.stage = Math.min(STORM.length - 1, s.stage + 1);
    s.shrinking = true; s.timer = next.shrink;
    s.fromX = s.cx; s.fromZ = s.cz; s.fromR = s.r;
    s.damage = next.damage;
    emit(state, 'storm', -1, s.toX, 0, s.toZ, { value: s.stage, flag: 1 });
  } else if (s.shrinking) {
    const total = STORM[s.stage].shrink, k = clamp(1 - s.timer / total, 0, 1);
    s.cx = s.fromX + (s.toX - s.fromX) * k; s.cz = s.fromZ + (s.toZ - s.fromZ) * k; s.r = s.fromR + (s.toR - s.fromR) * k;
    if (s.timer <= 0) {
      s.shrinking = false; s.cx = s.toX; s.cz = s.toZ; s.r = s.toR;
      if (s.stage < STORM.length - 1) { s.timer = STORM[s.stage + 1].wait; pickStormTarget(state, rng); emit(state, 'storm', -1, s.toX, 0, s.toZ, { value: s.stage + 1, flag: 0 }); }
      else s.timer = 9999;
    }
  }
  for (const p of state.players) {
    if (p.mode !== 'ground' && p.mode !== 'glide') continue;
    if (Math.hypot(p.x - s.cx, p.z - s.cz) <= s.r) { p.stormT = 0; continue; }
    p.stormT += STEP;
    if (p.stormT >= 1) { p.stormT -= 1; hurt(state, p, Math.max(1, s.damage), -2, false, false, 'pistol'); }
  }
}

export interface ShooterResult { id: number; name: string; fighter: FighterId; human: boolean; slot: number; place: number; kills: number; damage: number; time: number }
export function shooterResults(state: ShooterState): ShooterResult[] {
  return [...state.players].sort((a, b) => (a.place || 99) - (b.place || 99) || b.kills - a.kills)
    .map(p => ({ id: p.id, name: p.name, fighter: p.fighter, human: p.human, slot: p.slot, place: p.place || aliveCount(state), kills: p.kills, damage: Math.round(p.damage), time: p.aliveTime }));
}
export { HEIGHT };
