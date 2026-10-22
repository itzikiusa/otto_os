/**
 * Storm Royale simulation (host-authoritative, fixed 60 Hz, no three.js).
 * Up to 16 fighters (humans + bots) in five modes (see `modes/`): the battle
 * royale solo, in duos or in squads (drop from the flight path, skydive and
 * glide, loot weapons / attachments / ammo / consumables / launch pads /
 * materials, chests and supply drops into a five-slot hotbar, survive a storm
 * that closes in seven phases), Apocalypse Royale (the same under a toxic
 * cloud, with infected roaming the island), Team Deathmatch and Domination
 * (two teams, class loadouts, respawns, three capture points) and Zombies
 * (co-op against escalating waves of infected, buy-walls, barricades and an
 * extraction). Gunplay is Call of Duty-style: aim down sights with zoom,
 * learnable recoil patterns, hip-fire bloom, headshots and damage falloff,
 * reload cancels, tactical sprint, slides and mantling; grenades and
 * killstreaks live in `modes/gear.ts`, the infected in `zombies.ts`. Harvest
 * with the pickaxe (trees and rocks run dry, then regrow) and build walls /
 * floors / ramps (edit and destroy them), swim, ride launch pads, get knocked
 * and revived by teammates.
 */
import { approach, clamp, Rng, wrapAngle } from '../common/math.ts';
import { chestY, HALF, mapFor, terrainHeight, waterLevel, dryLand, type ShooterMap } from './map.ts';
import { groundAt, HEIGHT, raycast, resolve, syncFelled, worldFor, type World } from './collide.ts';
import {
  AMMO_KINDS, AMMO_MAX, AMMO_PACK, applyDamage, ATTACHMENTS, attachFits, BUILD_COST, canUse, CONSUMABLE_PACK, CONSUMABLES, damageAt, isConsumable, isWeapon, MATERIALS, MAX_MATS, PICKAXE,
  MAX_NADES, NADES, rollAttach, rollConsumable, rollRarity, rollWeapon, STREAKS, weaponStats, WEAPONS, WEAPON_IDS, type AmmoKind, type ItemId, type Material, type NadeKind, type Rarity, type WeaponId,
} from './weapons.ts';
import { canPlace, makeBuild, MAX_BUILDS, maxHp, planBuild, refreshGeometry, unsupported, type Build, type BuildKind } from './builds.ts';
import { botThink } from './bots.ts';
import type { FighterId, ShooterMapId, ShooterMode } from './catalog.ts';
import { hasInfected, isRoyale, respawns, setupMode, stepMode, type ModeState } from './modes/index.ts';
import { stepGear, throwNade, useStreak, type Area, type Drone, type Nade, type Strike } from './modes/gear.ts';
import { buyWall, hurtZombie, rayZombie, stepInfected, type Zombie } from './zombies.ts';
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
/** Team size: solo, duos or squads. */
export type TeamSize = 1 | 2 | 4;
export const TEAM_SIZES: TeamSize[] = [1, 2, 4];
/**
 * `teamSize` groups seats into teams (a seat's own `team` wins, otherwise
 * seats fill teams in order, so humans who join together play together).
 * `stormScale` shortens every storm wait and shrink (a test hook; 1 = normal).
 */
export interface ShooterSetup {
  map: ShooterMapId; difficulty: Difficulty; seed: number; seats: ShooterSeat[]; teamSize?: TeamSize; stormScale?: number;
  /** Game mode (default: the battle royale); `rounds` is the Zombies round the extraction follows (a test hook; default 8). */
  mode?: ShooterMode; rounds?: number;
}
export interface ShooterLink {
  role: 'host' | 'guest';
  inputs?: Map<number, { input: ShooterInput; fighter: FighterId }>;
  snapshots?: { state: ShooterState; at: number }[];
}
/**
 * Camera-relative movement (mz forward, mx right) and absolute aim angles.
 * `build` is build mode (fire places `piece` in material `mat`); `edit`
 * reshapes the structure in the crosshair. `crouch` (held) crouches, or
 * slides out of a sprint; `tac` turns a sprint into a tactical sprint;
 * `lethal` / `tactical` throw a grenade, `streak` calls in a killstreak and
 * `cls` is the class to respawn with (respawn modes).
 */
export interface ShooterInput {
  mx: number; mz: number; yaw: number; pitch: number;
  fire: boolean; aim: boolean; jump: boolean; sprint: boolean; reload: boolean; interact: boolean; slot: number;
  build: boolean; piece: number; mat: number; edit: boolean;
  crouch: boolean; tac: boolean; lethal: boolean; tactical: boolean; streak: boolean; cls: number;
}
export const idleShooterInput = (yaw = 0, pitch = 0): ShooterInput => ({
  mx: 0, mz: 0, yaw, pitch, fire: false, aim: false, jump: false, sprint: false, reload: false, interact: false, slot: 0, build: false, piece: 0, mat: 0, edit: false,
  crouch: false, tac: false, lethal: false, tactical: false, streak: false, cls: 0,
});

/** A hotbar item: a weapon (`mag` rounds loaded, `att` attachment bits) or a consumable stack (`mag` = count). */
export interface Held { w: ItemId; rarity: Rarity; mag: number; att: number }
export interface BotMemory {
  goalX: number; goalZ: number; think: number; target: number; seen: number; strafe: number; strafeT: number;
  stuck: number; lastX: number; lastZ: number; dropAt: number; burst: number; errX: number; errY: number; loot: number; jumpT: number;
  /** Seconds until the bot may build again / since it last saw a fight. */
  buildT: number; calm: number;
  /** Harvest target (cylinder index, or -1) and a cover spot to run to while reloading. */
  harvest: number; coverX: number; coverZ: number; coverT: number;
  /** Goal height (NaN: on the ground) and the building being climbed toward it (-1: none). */
  goalY: number; climb: number;
  /** Ramp rush: seconds left pushing toward the target and pieces placed. */
  pushT: number; pushN: number;
  /** Seconds left on a detour (a random nearby goal after getting stuck) before picking goals again. */
  detour: number;
  /** Infected being fought (zombie id, -1: none); where and when the fighter target was last seen; grenade and flank timers. */
  zt: number; seenX: number; seenZ: number; lost: number; nadeT: number; flankT: number; flankX: number; flankZ: number;
}
/** `down`: knocked (team modes), crawling and bleeding out until a teammate revives them. */
export type FighterMode = 'bus' | 'sky' | 'glide' | 'ground' | 'dead' | 'down';
export interface Fighter {
  id: number; fighter: FighterId; name: string; human: boolean; slot: number;
  /** Team index (solo: everyone is their own team), revive progress (s, while knocked) and who knocked them. */
  team: number; revive: number; knocker: number;
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
  /**
   * Gunplay: sights raised (0..1), recoil kick (up / sideways, radians) and
   * shots into the pattern; stance (crouching, slide time left), tactical
   * sprint stamina (s), a mantle in progress (time left and the ledge).
   */
  ads: number; kick: number; kickYaw: number; shots: number;
  crouch: boolean; slideT: number; tacT: number; tactical: boolean; mantleT: number; mantleX: number; mantleY: number; mantleZ: number;
  /** Grenades carried (kind and count), killstreak progress (eliminations since the last death) and earned streaks (bits). */
  lethal: NadeKind; lethalN: number; tacKind: NadeKind; tacN: number; streak: number; ready: number;
  /** Class (respawn modes), Zombies points, seconds to respawn, flash-blinded seconds, deaths, unsuppressed-fire radar ping (s). */
  cls: number; points: number; respawn: number; blind: number; deaths: number; spotted: number;
  prevCrouch: boolean; prevLethal: boolean; prevTactical: boolean; prevStreak: boolean;
  bot: BotMemory;
}
/**
 * heal / shield: instant forage; item: a consumable for the hotbar (`weapon`
 * holds its id); mats: building materials (`mat`); attach: a weapon
 * attachment (`amount` is its index in ATTACHMENTS); nade: a grenade
 * (`amount` is its kind).
 */
export type LootKind = 'weapon' | 'ammo' | 'heal' | 'shield' | 'item' | 'mats' | 'attach' | 'nade';
export interface Loot { id: number; kind: LootKind; weapon: ItemId | null; rarity: Rarity; ammo: AmmoKind | null; mat: Material | null; amount: number; x: number; y: number; z: number }
export interface Rocket { id: number; owner: number; rarity: Rarity; x: number; y: number; z: number; vx: number; vy: number; vz: number; life: number }
export interface Storm { stage: number; cx: number; cz: number; r: number; fromX: number; fromZ: number; fromR: number; toX: number; toZ: number; toR: number; timer: number; shrinking: boolean; damage: number }
/** A launch pad on the ground (map set pieces and pads survivors deploy). */
export interface Pad { id: number; x: number; y: number; z: number }
/** A supply drop: falls from the sky to `ground`, then opens like a chest. */
export interface Drop { id: number; x: number; y: number; z: number; ground: number; landed: boolean }
/** A prop someone harvested (host only): materials left and seconds until it is whole again. Key: box index, or -1 - trunk index. */
export interface HarvestNode { key: number; left: number; regrow: number }
export type ShooterEventType = 'shot' | 'hit' | 'kill' | 'pickup' | 'chest' | 'explode' | 'reload' | 'land' | 'jump' | 'storm' | 'win' | 'glide' | 'build' | 'break' | 'harvest' | 'use' | 'swing' | 'edit'
  | 'knock' | 'revive' | 'supply' | 'launch' | 'fell'
  /** Gear and modes: a grenade thrown (value kind), a smoke / fire / acid patch, a flash (target blinded, value tenths of a second). */
  | 'nade' | 'smoke' | 'fire' | 'flash'
  /** Killstreaks: earned (flag 0) / called in (flag 1), value the streak index. */
  | 'streak'
  /** Domination: a point captured (value point, flag team). Zombies: a round starts (value round), an infected hit / killed (target zombie id, value kind), a buy-wall bought, the helicopter (flag 0 called, 1 landed, 2 left; value survivors aboard). */
  | 'capture' | 'round' | 'zhit' | 'zkill' | 'buy' | 'extract'
  /** A fighter respawned; mantled a ledge; started a slide; repaired a barricade. */
  | 'spawn' | 'mantle' | 'slide' | 'repair';
export interface ShooterEvent { id: number; type: ShooterEventType; player: number; target: number; value: number; flag: number; x: number; y: number; z: number; x2: number; y2: number; z2: number }
export interface ShooterState {
  map: ShooterMapId; difficulty: Difficulty; phase: 'bus' | 'play' | 'done'; clock: number; endTimer: number;
  bus: { ax: number; az: number; bx: number; bz: number; t: number; duration: number };
  players: Fighter[]; loot: Loot[]; lootRev: number; chests: boolean[]; rockets: Rocket[]; storm: Storm;
  builds: Build[]; buildRev: number;
  /** Team size and the winning team (-1 until crowned). */
  squad: TeamSize; winTeam: number;
  pads: Pad[]; drops: Drop[];
  /** Harvested props (host only) and the trunks harvested away (replaced, never mutated, when it changes). */
  nodes: HarvestNode[]; felled: number[];
  stormScale: number;
  /** Game mode and its state (scores, capture points, Zombies rounds; see `modes/`). */
  mode: ShooterMode; obj: ModeState;
  /** Grenades and globs in flight, lingering clouds / fires / acid / airstrike markers, attack drones, airstrikes on the way (host only). */
  nades: Nade[]; areas: Area[]; drones: Drone[]; strikes: Strike[];
  /** UAV time left per team (index: team). */
  uav: number[];
  /** The infected (Zombies, Apocalypse Royale). */
  infected: Zombie[];
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
export const START_RADIUS = HALF * 1.45;
/**
 * Kill-event weapon codes: weapon index, 6 storm, 7 pickaxe, 8 bled out
 * while knocked, 9 frag, 10 molotov, 11 airstrike, 12 attack drone, 13 the
 * infected, 14 acid.
 */
export const KILL_STORM = 6;
export const KILL_PICKAXE = 7;
export const KILL_BLEED = 8;
export const KILL_FRAG = 9;
export const KILL_FIRE = 10;
export const KILL_AIRSTRIKE = 11;
export const KILL_DRONE = 12;
export const KILL_INFECTED = 13;
export const KILL_ACID = 14;
/** Damage sources that are not a fighter: the storm / toxic cloud, the infected. */
export const FROM_STORM = -2;
export const FROM_INFECTED = -3;
/** What dealt the damage: a weapon, the pickaxe, or one of the kill codes above. */
export type DamageSource = WeaponId | 'pickaxe' | number;
/** Tactical sprint stamina (s) and speed; slide speed and length; crouch speed. */
export const TAC_TIME = 4;
export const SLIDE_TIME = 0.75;
/** Knocked: health to bleed through (per second), crawl speed; revive time, reach and the health it restores. */
export const DOWN_HP = 100;
export const BLEED = 3;
export const CRAWL = 1.7;
export const REVIVE_TIME = 4;
export const REVIVE_RANGE = 2.1;
export const REVIVE_HP = 30;
/** Launch pads: take-off speed (about 20 m of climb before the glider opens) and the deploy cap. */
export const LAUNCH_SPEED = 30;
export const MAX_PADS = 16;
/** Supply drops: one falls when these storm phases start closing; spawn height and fall speed. */
export const SUPPLY_STAGES = [0, 2, 4];
export const MAX_DROPS = 4;
export const DROP_HEIGHT = 90;
export const DROP_SPEED = 7;
/** Harvesting: tracked props, seconds until a harvested prop is whole again, materials per prop. */
export const MAX_NODES = 128;
export const REGROW = 60;
export const HARVEST_LEFT = [60, 72, 54];

export const emptyAmmo = (): Record<AmmoKind, number> => ({ light: 0, medium: 0, shells: 0, heavy: 0, rockets: 0 });
export const emptyBot = (think = 0): BotMemory => ({
  goalX: 0, goalZ: 0, think, target: -1, seen: 0, strafe: 1, strafeT: 0, stuck: 0, lastX: 0, lastZ: 0, dropAt: 0, burst: 0, errX: 0, errY: 0, loot: -1, jumpT: 0,
  buildT: 0, calm: 0, harvest: -1, coverX: 0, coverZ: 0, coverT: 0, goalY: NaN, climb: -1, pushT: 0, pushN: 0, detour: 0,
  zt: -1, seenX: 0, seenZ: 0, lost: 99, nadeT: 0, flankT: 0, flankX: 0, flankZ: 0,
});
/** Standing: alive and not knocked. */
export const standing = (p: Pick<Fighter, 'mode'>): boolean => p.mode !== 'dead' && p.mode !== 'down';
/** Does `team` have anyone still standing, other than fighter `except`? */
export function teamUp(state: ShooterState, team: number, except = -1): boolean {
  for (const p of state.players) if (p.team === team && p.id !== except && standing(p)) return true;
  return false;
}
/** Teams with anyone not yet eliminated (knocked fighters still count). */
export function teamsAlive(state: ShooterState): number {
  let n = 0;
  const ps = state.players;
  for (let i = 0; i < ps.length; i++) {
    if (ps[i].mode === 'dead') continue;
    let first = true;
    for (let j = 0; j < i; j++) if (ps[j].team === ps[i].team && ps[j].mode !== 'dead') { first = false; break; }
    if (first) n++;
  }
  return n;
}
/** The hotbar item in hand (null with the pickaxe out or an empty slot). */
export const heldItem = (p: Pick<Fighter, 'weapons' | 'current'>): Held | null => (p.current >= 0 && p.current < SLOTS ? p.weapons[p.current] ?? null : null);

/** A fighter in the balloon (royale) with empty pockets; the modes place and equip it. */
export function newFighter(seat: ShooterSeat, id: number, team: number, rng: Rng): Fighter {
  return {
    id, fighter: seat.fighter, name: seat.name, human: seat.human, slot: seat.slot, team, revive: 0, knocker: -1,
    x: 0, y: BUS_HEIGHT, z: 0, vx: 0, vy: 0, vz: 0, yaw: 0, pitch: -0.3, grounded: false,
    mode: 'bus', hp: 100, shield: 0, kills: 0, damage: 0, place: 0, killedBy: -1, aliveTime: 0,
    weapons: Array.from({ length: SLOTS }, () => null), current: PICKAXE_SLOT, ammo: emptyAmmo(), mats: [0, 0, 0],
    building: false, piece: 0, mat: 0, swimming: false,
    cooldown: 0, reload: 0, aiming: false, sprinting: false, moving: false, shootT: 0, hurtT: 0, bloom: 0, useT: 0, swingT: 0,
    interact: 0, prevFire: false, prevJump: false, prevInteract: false, prevReload: false, prevEdit: false, stormT: 0,
    ads: 0, kick: 0, kickYaw: 0, shots: 0, crouch: false, slideT: 0, tacT: TAC_TIME, tactical: false, mantleT: 0, mantleX: 0, mantleY: 0, mantleZ: 0,
    lethal: 0, lethalN: 0, tacKind: 2, tacN: 0, streak: 0, ready: 0,
    cls: seat.human ? 0 : rng.int(4), points: 0, respawn: 0, blind: 0, deaths: 0, spotted: 0,
    prevCrouch: false, prevLethal: false, prevTactical: false, prevStreak: false,
    bot: emptyBot(rng.range(0, 0.3)),
  };
}

export function createMatch(setup: ShooterSetup): ShooterState {
  const map = mapFor(setup.map);
  const rng = new Rng(setup.seed);
  const mode: ShooterMode = setup.mode === 'tdm' || setup.mode === 'dom' || setup.mode === 'zombies' || setup.mode === 'apoc' ? setup.mode : 'royale';
  // Flight path across the island through a point near the centre.
  const angle = rng.range(0, Math.PI * 2), off = rng.range(-60, 60);
  const cx = Math.cos(angle + Math.PI / 2) * off, cz = Math.sin(angle + Math.PI / 2) * off;
  const reach = HALF + 20;
  // Team modes play as teams of up to eight (`squad` 4 marks team play for the HUD and bots).
  const squad: TeamSize = !isRoyale(mode) ? 4 : setup.teamSize === 2 || setup.teamSize === 4 ? setup.teamSize : 1;
  const stormScale = clamp(Number.isFinite(setup.stormScale) ? setup.stormScale! : 1, 0.02, 1);
  const bus = { ax: cx - Math.cos(angle) * reach, az: cz - Math.sin(angle) * reach, bx: cx + Math.cos(angle) * reach, bz: cz + Math.sin(angle) * reach, t: 0, duration: (reach * 2) / BUS_SPEED };
  const seats = setup.seats.slice(0, MAX_FIGHTERS);
  const half = Math.ceil(seats.length / 2);
  const players = seats.map((seat, i): Fighter => {
    const pinned = Number.isInteger(seat.team) && seat.team! >= 0 ? seat.team! : -1;
    // Two teams (people first, together), one co-op team, or the royale's solo / duos / squads.
    const team = mode === 'zombies' ? 0 : respawns(mode) ? (pinned === 0 || pinned === 1 ? pinned : i < half ? 0 : 1) : squad === 1 ? i : pinned >= 0 ? pinned : Math.floor(i / squad);
    const p = newFighter(seat, i, team, rng);
    p.x = bus.ax; p.z = bus.az; p.yaw = Math.atan2(Math.cos(angle), Math.sin(angle));
    return p;
  });
  if (isRoyale(mode)) for (const p of players) {
    if (!p.human) {
      // Bots pick a landing spot and jump when the flight passes closest to it (teammates drop together).
      const lead = squad > 1 ? players.find(q => q.team === p.team && q.id < p.id && !q.human) : undefined;
      if (lead) { p.bot.goalX = lead.bot.goalX + rng.range(-5, 5); p.bot.goalZ = lead.bot.goalZ + rng.range(-5, 5); p.bot.dropAt = lead.bot.dropAt + rng.range(0, 0.6); continue; }
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
    squad, winTeam: -1, pads: [], drops: [], nodes: [], felled: [], stormScale,
    storm: { stage: -1, cx: 0, cz: 0, r: START_RADIUS, fromX: 0, fromZ: 0, fromR: START_RADIUS, toX: 0, toZ: 0, toR: START_RADIUS, timer: STORM[0].wait * stormScale, shrinking: false, damage: 0 },
    mode, obj: null!, nades: [], areas: [], drones: [], strikes: [], uav: [], infected: [],
    events: [], eventSeq: 0, nextId: 1, rng: 0, accumulator: 0, winner: -1,
  };
  for (const pad of map.pads) state.pads.push({ id: state.nextId++, x: pad.x, y: pad.y, z: pad.z });
  syncFelled(worldFor(map), state.felled);
  // Floor loot in the royale modes (the arena and Zombies hand out loadouts instead).
  if (isRoyale(mode)) for (const spot of map.lootSpots) {
    if (!rng.chance(0.85)) continue;
    const y = spot.y ?? terrainHeight(map, spot.x, spot.z);
    const roll = rng.next();
    if (roll < 0.5) {
      const weapon = rollWeapon(rng.next());
      dropItem(state, 'weapon', weapon, rollRarity(rng.next()), WEAPONS[weapon].mag, spot.x, y, spot.z);
      dropLoot(state, 'ammo', WEAPONS[weapon].ammo, AMMO_PACK[WEAPONS[weapon].ammo], spot.x + 0.9, y, spot.z + 0.4);
    } else if (roll < 0.56) dropAttach(state, rollAttach(rng.next()), spot.x, y, spot.z);
    else if (roll < 0.62) dropNade(state, rng.int(4) as NadeKind, spot.x, y, spot.z);
    else if (roll < 0.76) {
      const item = rollConsumable(rng.next());
      dropItem(state, 'item', item, CONSUMABLES[item].rarity, CONSUMABLE_PACK[item], spot.x, y, spot.z);
    } else if (roll < 0.87) { const kind = rng.pick(AMMO_KINDS); dropLoot(state, 'ammo', kind, AMMO_PACK[kind], spot.x, y, spot.z); }
    else if (roll < 0.94) dropMats(state, rng.int(3) as Material, 30, spot.x, y, spot.z);
    else dropLoot(state, rng.chance(0.5) ? 'heal' : 'shield', null, 25, spot.x, y, spot.z);
  }
  pickStormTarget(state, rng);
  state.obj = setupMode(state, map, rng, setup.rounds);
  state.rng = rng.state;
  return state;
}

export function emit(state: ShooterState, type: ShooterEventType, player: number, x: number, y: number, z: number, extra: Partial<ShooterEvent> = {}): void {
  state.events.push({ id: ++state.eventSeq, type, player, target: -1, value: 0, flag: 0, x, y, z, x2: 0, y2: 0, z2: 0, ...extra });
  if (state.events.length > 64) state.events.shift();
}
export function dropLoot(state: ShooterState, kind: LootKind, ammo: AmmoKind | null, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind, weapon: null, rarity: 0, ammo, mat: null, amount, x, y, z });
  state.lootRev++;
}
export function dropItem(state: ShooterState, kind: 'weapon' | 'item', item: ItemId, rarity: Rarity, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind, weapon: item, rarity, ammo: null, mat: null, amount, x, y, z });
  state.lootRev++;
}
function dropMats(state: ShooterState, mat: Material, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT || amount <= 0) return;
  state.loot.push({ id: state.nextId++, kind: 'mats', weapon: null, rarity: 0, ammo: null, mat, amount, x, y, z });
  state.lootRev++;
}
/** An attachment on the floor (`index` into ATTACHMENTS, carried in `amount`). */
export function dropAttach(state: ShooterState, index: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind: 'attach', weapon: null, rarity: 2, ammo: null, mat: null, amount: index, x, y, z });
  state.lootRev++;
}
/** A grenade on the floor (`kind`: frag, molotov, smoke, flash; carried in `amount`). */
export function dropNade(state: ShooterState, kind: NadeKind, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind: 'nade', weapon: null, rarity: 1, ammo: null, mat: null, amount: kind, x, y, z });
  state.lootRev++;
}
/** Drops a hotbar item as floor loot (a weapon keeps its attachments only while held; they fall beside it). */
function dropHeld(state: ShooterState, h: Held, x: number, y: number, z: number): void {
  dropItem(state, isWeapon(h.w) ? 'weapon' : 'item', h.w, h.rarity, isWeapon(h.w) ? Math.min(h.mag, WEAPONS[h.w].mag) : h.mag, x, y, z);
  for (let i = 0; i < ATTACHMENTS.length; i++) if (h.att & ATTACHMENTS[i].bit) dropAttach(state, i, x + 0.5 + i * 0.3, y, z - 0.5);
}
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
    crouch: i.crouch === true, tac: i.tac === true, lethal: i.lethal === true, tactical: i.tactical === true, streak: i.streak === true, cls: pick(i.cls, CLASS_COUNT - 1),
  };
}
const CLASS_COUNT = 4;

export function tickMatch(state: ShooterState, inputs: (ShooterInput | undefined)[]): void {
  if (state.phase === 'done') return;
  const map = mapFor(state.map), world = worldFor(map);
  syncFelled(world, state.felled);
  const rng = new Rng(state.rng || 1);
  state.clock += STEP;
  if (state.phase === 'bus') {
    state.bus.t += STEP;
    if (state.players.every(p => p.mode !== 'bus') || state.bus.t > state.bus.duration) state.phase = 'play';
  }
  // Revive progress only lasts while a teammate keeps holding interact this tick.
  for (const p of state.players) if (p.mode === 'down') p.revive = 0;
  for (const p of state.players) {
    if (p.mode === 'dead') continue;
    p.aliveTime += STEP;
    const input = p.human ? sanitizeInput(inputs[p.id]) : botThink(state, map, world, p, rng);
    stepFighter(state, map, world, p, input, rng);
  }
  stepRockets(state, world);
  growBuilds(state);
  stepStorm(state, map, world, rng);
  stepNodes(state);
  stepDrops(state);
  stepGear(state, map, world, rng);
  if (hasInfected(state.mode)) stepInfected(state, map, world, rng);
  // Respawns, scores, capture points, Zombies rounds; the arena modes and Zombies end themselves there.
  stepMode(state, map, world, rng, inputs);
  if (isRoyale(state.mode)) royaleEnd(state);
  state.rng = rng.state;
}

/** Battle royale end: the last team standing wins (or the match wraps up once every person is out). */
function royaleEnd(state: ShooterState): void {
  const teams = teamsAlive(state);
  // A team with a person on it is still in the match while any member is (they spectate their teammates).
  const humansAlive = state.players.some(p => p.mode !== 'dead' && (p.human || (state.squad > 1 && state.players.some(h => h.human && h.team === p.team))));
  const hadHumans = state.players.some(p => p.human);
  if (!((teams <= 1 || (hadHumans && !humansAlive)) && state.phase === 'play')) return;
  if (state.endTimer === 0 && teams <= 1) {
    const last = state.players.find(p => standing(p)) ?? state.players.find(p => p.mode !== 'dead');
    if (last) { crown(state, last.team); emit(state, 'win', state.winner, last.x, last.y, last.z, { value: last.team }); }
  }
  state.endTimer += STEP;
  if (state.endTimer > 3) {
    // Teams still in it when every human is out share the remaining places (healthiest first).
    const ranked: { team: number; health: number }[] = [];
    for (const p of state.players) {
      if (p.mode === 'dead' || p.place !== 0) continue;
      const health = p.mode === 'down' ? p.hp * 0.25 : p.hp + p.shield;
      const t = ranked.find(r => r.team === p.team);
      if (t) t.health += health; else ranked.push({ team: p.team, health });
    }
    ranked.sort((a, b) => b.health - a.health).forEach((t, i) => { for (const p of state.players) if (p.team === t.team && p.place === 0) p.place = i + 1; });
    if (state.winner < 0 && ranked[0]) { state.winTeam = ranked[0].team; state.winner = (state.players.find(p => p.team === ranked[0].team && standing(p)) ?? state.players.find(p => p.team === ranked[0].team))!.id; }
    state.phase = 'done';
  }
}

/** Crowns a team: every member places first; `winner` is a standing member. */
export function crown(state: ShooterState, team: number): void {
  state.winTeam = team;
  for (const p of state.players) if (p.team === team) p.place = 1;
  state.winner = (state.players.find(p => p.team === team && standing(p)) ?? state.players.find(p => p.team === team && p.mode !== 'dead') ?? state.players.find(p => p.team === team))!.id;
}

export const fwd = (yaw: number): [number, number] => [Math.sin(yaw), Math.cos(yaw)];

/** Tactical sprint, sprint, walk, crouch and aimed (sights fully up) speeds. */
const SPEED = { tac: 9.4, sprint: 7.8, walk: 5.4, crouch: 2.8, ads: 3.4, slide: 11 };

function stepFighter(state: ShooterState, map: ShooterMap, world: World, p: Fighter, input: ShooterInput, rng: Rng): void {
  p.cooldown = Math.max(0, p.cooldown - STEP);
  p.shootT = Math.max(0, p.shootT - STEP);
  p.hurtT = Math.max(0, p.hurtT - STEP);
  p.swingT = Math.max(0, p.swingT - STEP);
  p.blind = Math.max(0, p.blind - STEP);
  p.spotted = Math.max(0, p.spotted - STEP);
  p.bloom = Math.max(0, p.bloom - STEP * 0.25);
  p.yaw = input.yaw; p.pitch = input.pitch;
  const jumpPressed = input.jump && !p.prevJump, interactPressed = input.interact && !p.prevInteract, editPressed = input.edit && !p.prevEdit;
  const crouchPressed = input.crouch && !p.prevCrouch, lethalPressed = input.lethal && !p.prevLethal, tacticalPressed = input.tactical && !p.prevTactical, streakPressed = input.streak && !p.prevStreak;
  p.prevJump = input.jump; p.prevInteract = input.interact; p.prevEdit = input.edit;
  p.prevCrouch = input.crouch; p.prevLethal = input.lethal; p.prevTactical = input.tactical; p.prevStreak = input.streak;
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
    p.crouch = false; p.slideT = 0; p.ads = 0; p.tactical = false;
    const g = terrainHeight(map, p.x, p.z);
    if (p.mode === 'sky' && p.y - g < 34 && p.vy <= 0) { p.mode = 'glide'; emit(state, 'glide', p.id, p.x, p.y, p.z); }
    const glide = p.mode === 'glide';
    const maxH = glide ? 11 : 17, fall = glide ? -6.5 : -32 - Math.max(0, input.mz) * 6;
    p.vx = approach(p.vx, dirX * maxH, glide ? 1.6 : 1.2, STEP);
    p.vz = approach(p.vz, dirZ * maxH, glide ? 1.6 : 1.2, STEP);
    // Off a launch pad: a ballistic climb first, the glider opens on the way down.
    p.vy = p.vy > 0 ? p.vy - 22 * STEP : approach(p.vy, fall, glide ? 3 : 1.4, STEP);
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
  // Mantling: carried up and over the ledge, nothing else happens meanwhile.
  if (p.mantleT > 0) {
    const k = Math.min(1, STEP / p.mantleT);
    p.mantleT = Math.max(0, p.mantleT - STEP);
    p.x += (p.mantleX - p.x) * k; p.y += (p.mantleY - p.y) * k; p.z += (p.mantleZ - p.z) * k;
    p.vx = 0; p.vy = 0; p.vz = 0;
    if (p.mantleT <= 0) { p.grounded = true; resolve(world, p, state.builds); p.y = Math.max(p.y, groundAt(world, p.x, p.z, p.y, state.builds)); }
    return;
  }

  // Ground movement (knocked: a slow crawl while bleeding out, nothing else).
  const down = p.mode === 'down';
  if (down) {
    p.hp -= BLEED * STEP;
    p.crouch = false; p.slideT = 0; p.ads = 0;
    if (p.hp <= 0) { eliminate(state, p, p.knocker, KILL_BLEED); return; }
  } else if (input.slot >= 1 && input.slot <= PICKAXE_KEY && input.slot - 1 !== p.current) { p.current = input.slot - 1; p.reload = 0; p.useT = 0; p.ads = 0; p.shots = 0; p.cooldown = Math.max(p.cooldown, 0.25); }
  const held = down ? null : heldItem(p);
  p.building = !down && input.build && !p.swimming;
  p.piece = input.piece as BuildKind; p.mat = input.mat as Material;
  const using = !p.building && !!held && isConsumable(held.w) && input.fire && canUse(held.w, p.hp, p.shield);
  // Reviving a knocked teammate roots the survivor in place.
  const mate = !down && input.interact && p.grounded ? nearestDowned(state, p, REVIVE_RANGE) : -1;
  const gun = held && isWeapon(held.w) ? WEAPONS[held.w] : null;
  const stats = gun ? weaponStats(gun, held!.att) : null;
  const wasSprinting = p.sprinting;
  p.aiming = !down && input.aim && p.grounded && !p.building && !p.swimming;
  p.sprinting = !down && input.sprint && input.mz > 0.3 && !input.fire && !p.aiming && !p.swimming && !using && p.slideT <= 0 && !input.crouch;
  // Tactical sprint: faster, burns stamina (which refills while walking), weapon lowered further.
  p.tactical = p.sprinting && input.tac && p.tacT > 0;
  if (p.tactical) p.tacT = Math.max(0, p.tacT - STEP); else if (!p.sprinting) p.tacT = Math.min(TAC_TIME, p.tacT + STEP * 0.5);
  // Crouch out of a sprint: a slide that carries the sprint's speed and lets the survivor fire.
  if (crouchPressed && wasSprinting && p.grounded && !down && !p.swimming && !p.building) {
    p.slideT = SLIDE_TIME;
    const sx = wl > 0.1 ? dirX : fx, sz = wl > 0.1 ? dirZ : fz, sl = Math.hypot(sx, sz) || 1;
    p.vx = (sx / sl) * SPEED.slide; p.vz = (sz / sl) * SPEED.slide;
    p.sprinting = false; p.tactical = false;
    emit(state, 'slide', p.id, p.x, p.y, p.z);
  }
  if (p.slideT > 0) p.slideT = Math.max(0, p.slideT - STEP);
  p.crouch = !down && p.grounded && !p.swimming && (input.crouch || p.slideT > 0);
  // Sights come up over the weapon's ADS time (a scope is slower) and drop at once when sprinting.
  if (p.aiming && stats && !p.sprinting) p.ads = Math.min(1, p.ads + STEP / stats.ads); else p.ads = Math.max(0, p.ads - STEP / 0.12);
  // Coming out of a sprint the weapon needs a moment before it fires.
  if (wasSprinting && !p.sprinting && p.slideT <= 0) p.cooldown = Math.max(p.cooldown, p.tacT < TAC_TIME && input.tac ? 0.22 : 0.12);
  const walk = p.tactical ? SPEED.tac : p.sprinting ? SPEED.sprint : p.crouch ? SPEED.crouch : SPEED.walk + (SPEED.ads - SPEED.walk) * p.ads;
  const speed = mate >= 0 ? 0 : down ? CRAWL : walk * (p.swimming ? 0.55 : using ? 0.55 : 1);
  p.moving = wl > 0.1 && speed > 0;
  const accel = p.grounded ? 14 : 3;
  if (p.slideT > 0) {
    // Sliding: momentum bleeds off, a little steering.
    const k = Math.exp(-1.5 * STEP);
    p.vx = p.vx * k + dirX * 2 * STEP; p.vz = p.vz * k + dirZ * 2 * STEP;
  } else {
    p.vx = approach(p.vx, dirX * speed, accel, STEP);
    p.vz = approach(p.vz, dirZ * speed, accel, STEP);
  }
  if (!down && jumpPressed && p.grounded && !p.swimming && tryMantle(state, world, p)) return;
  if (p.grounded && jumpPressed && !down) { p.vy = 7.4; p.grounded = false; p.slideT = 0; }
  // Jumping at a ledge: grab it on the way up (hold jump and push forward).
  else if (!down && !p.grounded && input.jump && input.mz > 0.3 && p.vy < 4 && !p.swimming && tryMantle(state, world, p)) return;
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
  if (p.swimming) { p.y = Math.max(p.y, surface); if (p.vy < 0) p.vy = 0; p.grounded = true; p.crouch = false; p.slideT = 0; }
  if (p.mode !== 'ground' && p.mode !== 'down') return;
  if (down) { p.interact = 0; return; }
  // Launch pads: step on one to be thrown into the sky (the glider opens on the way down).
  if (!p.swimming && p.vy <= 0.1) for (const pad of state.pads) {
    const dx = pad.x - p.x, dz = pad.z - p.z;
    if (dx * dx + dz * dz > 1.2 * 1.2 || p.y - pad.y > 0.8 || p.y - pad.y < -0.6) continue;
    p.mode = 'sky'; p.vy = LAUNCH_SPEED; p.grounded = false; p.building = false; p.useT = 0; p.reload = 0; p.interact = 0;
    emit(state, 'launch', p.id, pad.x, pad.y, pad.z, { value: pad.id });
    return;
  }

  // Interact: revive a teammate, open chests and supply drops, buy from a wall, swap items.
  pickupAuto(state, p);
  if (input.interact) {
    const chest = mate >= 0 ? -1 : nearestChest(state, map, p, 2.4);
    const drop = mate >= 0 || chest >= 0 ? -1 : nearestDrop(state, p, 2.6);
    if (mate >= 0) {
      const q = state.players[mate];
      p.interact += STEP;
      q.revive = p.interact;
      if (p.interact >= REVIVE_TIME) {
        q.mode = 'ground'; q.hp = REVIVE_HP; q.shield = 0; q.revive = 0; q.knocker = -1; q.grounded = true;
        p.interact = 0;
        emit(state, 'revive', p.id, q.x, q.y, q.z, { target: q.id });
        if (state.mode === 'zombies') p.points += 50;
      }
    } else if (chest >= 0) {
      p.interact += STEP;
      if (p.interact >= 0.55) { openChest(state, map, chest, rng); p.interact = 0; }
    } else if (drop >= 0) {
      p.interact += STEP;
      if (p.interact >= 1) { openDrop(state, drop, p.id, rng); p.interact = 0; }
    } else {
      // Nothing to hold E on in reach (walked away mid-revive): progress is lost.
      p.interact = 0;
      if (interactPressed && !(state.mode === 'zombies' && buyWall(state, p))) swapWeapon(state, p);
    }
  } else p.interact = 0;

  // Grenades (an arc along the aim) and killstreaks.
  if (!p.building && !p.swimming && p.cooldown <= 0 && p.mantleT <= 0) {
    if (lethalPressed && p.lethalN > 0) { throwNade(state, p, p.lethal); p.lethalN--; p.cooldown = 0.6; p.reload = 0; p.ads = 0; }
    else if (tacticalPressed && p.tacN > 0) { throwNade(state, p, p.tacKind); p.tacN--; p.cooldown = 0.6; p.reload = 0; p.ads = 0; }
  }
  if (streakPressed && p.ready) useStreak(state, map, world, p, rng);

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
      if (c.pad && !placePad(state, world, p)) { p.useT = 0; return; }
      if (c.heal) p.hp = Math.min(Math.max(p.hp, c.cap), p.hp + c.heal);
      if (c.shield) p.shield = Math.min(Math.max(p.shield, c.cap), p.shield + c.shield);
      held.mag--;
      if (held.mag <= 0) p.weapons[p.current] = null;
      p.useT = 0;
      emit(state, 'use', p.id, p.x, p.y, p.z, { value: c.pad ? 2 : c.heal ? 0 : 1 });
    }
    return;
  }
  // Weapons: recoil recovers (faster once the trigger is released), reload, then fire.
  const def = gun!, mag = stats!.mag;
  const rec = Math.exp(-def.recover * (input.fire && held.mag > 0 ? 0.3 : 1) * STEP);
  p.kick *= rec; p.kickYaw *= rec;
  if (!input.fire && p.cooldown <= 0 && Math.abs(p.kick) < 0.003) p.shots = 0;
  if (p.reload > 0) {
    // Firing with rounds still in the mag, or breaking into a tactical sprint, cancels the reload.
    if ((firePressed && held.mag > 0) || p.tactical) p.reload = 0;
    else {
      p.reload -= STEP;
      if (p.reload <= 0) {
        const take = Math.min(mag - held.mag, p.ammo[def.ammo]);
        held.mag += take; p.ammo[def.ammo] -= take; p.reload = 0;
      }
    }
  } else if ((reloadPressed || (held.mag === 0 && input.fire)) && held.mag < mag && p.ammo[def.ammo] > 0) {
    // A tactical reload (rounds left in the mag) is quicker than an empty one.
    p.reload = def.reload * (held.mag > 0 ? 0.85 : 1);
    p.shots = 0;
    emit(state, 'reload', p.id, p.x, p.y, p.z);
  }
  const trigger = input.fire && p.reload <= 0 && held.mag > 0 && !p.sprinting && !p.swimming && (def.auto || firePressed);
  if (trigger && p.cooldown <= 0) fire(state, map, world, p, held, held.w as WeaponId, rng);
  // A semi-auto click just before the weapon is ready (between shots, out of a sprint) is kept until it is.
  else if (trigger && !def.auto && p.cooldown < 0.2) p.prevFire = false;
}

/**
 * Mantle: a ledge 0.7–2.3 m above the feet just ahead, with room to stand on
 * top, is climbed over in about half a second (walls, crates, cars, rooftops
 * from a ramp). Returns whether the mantle started.
 */
function tryMantle(state: ShooterState, world: World, p: Fighter): boolean {
  const [fx, fz] = fwd(p.yaw);
  const base = p.grounded ? p.y : Math.max(p.y - 0.6, groundAt(world, p.x, p.z, p.y, state.builds));
  for (const reach of [0.75, 1.1]) {
    const x = p.x + fx * reach, z = p.z + fz * reach;
    const top = groundAt(world, x, z, base + 1.75, state.builds);
    const rise = top - base;
    if (rise < 0.7 || rise > 2.35) continue;
    // Headroom over the ledge, and nothing in the way at chest height before it.
    const over = raycast(world, x, top + 0.15, z, 0, 1, 0, HEIGHT, true, state.builds);
    if (over.kind !== 'none') continue;
    p.mantleT = 0.3 + rise * 0.1; p.mantleX = x + fx * 0.25; p.mantleY = top; p.mantleZ = z + fz * 0.25;
    p.slideT = 0; p.reload = 0; p.ads = 0; p.sprinting = false; p.tactical = false; p.vy = 0;
    emit(state, 'mantle', p.id, x, top, z);
    return true;
  }
  return false;
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
export function damageBuild(state: ShooterState, world: World, b: Build, amount: number): void {
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
    if (q.id === p.id || (q.mode !== 'ground' && q.mode !== 'down') || q.team === p.team) continue;
    const h = rayFighter(ox, oy, oz, dx, dy, dz, q, tEnd);
    if (h.t >= 0 && h.t < tEnd) { tEnd = h.t; victim = q.id; }
  }
  // The infected take a pickaxe too (a last resort when the mag runs dry).
  let zv = -1;
  for (let i = 0; i < state.infected.length; i++) {
    const z = state.infected[i];
    if (z.hp <= 0) continue;
    const h = rayZombie(ox, oy, oz, dx, dy, dz, z, tEnd);
    if (h.t >= 0 && h.t < tEnd) { tEnd = h.t; zv = i; victim = -1; }
  }
  emit(state, 'swing', p.id, ox + dx * tEnd, oy + dy * tEnd, oz + dz * tEnd, { value: victim >= 0 || zv >= 0 ? 1 : hit.kind === 'none' ? 0 : 2 });
  if (victim >= 0) { hurt(state, state.players[victim], PICKAXE.damage, p.id, false, true, 'pickaxe'); return; }
  if (zv >= 0) { hurtZombie(state, state.infected[zv], PICKAXE.damage * 2, p.id, false, KILL_PICKAXE); return; }
  if (hit.kind === 'build') {
    const b = state.builds[hit.index];
    // Zombies: swinging at your team's barricade repairs it (and earns points) instead of breaking it.
    if (state.mode === 'zombies' && (b.owner < 0 || state.players[b.owner]?.team === p.team)) {
      if (b.hp < maxHp(b) && b.grow <= 0) {
        b.hp = Math.min(maxHp(b), b.hp + PICKAXE.structure);
        state.buildRev++;
        p.points += 10;
        emit(state, 'repair', p.id, ox + dx * tEnd, oy + dy * tEnd, oz + dz * tEnd, { value: b.kind, flag: b.mat });
      }
      return;
    }
    damageBuild(state, world, b, PICKAXE.structure);
    return;
  }
  if (hit.kind !== 'prop') return;
  const mat = hit.cyl ? 0 : map.boxMats[hit.index];
  if (mat < 0) return;
  // Every prop holds a limited amount: trees fall when spent, rocks and cars run dry; all regrow later.
  const key = hit.cyl ? -1 - hit.index : hit.index;
  let node: HarvestNode | null = null;
  for (const n of state.nodes) if (n.key === key) { node = n; break; }
  if (!node) {
    if (state.nodes.length >= MAX_NODES) dropNode(state, 0);
    node = { key, left: HARVEST_LEFT[mat], regrow: REGROW };
    state.nodes.push(node);
  }
  node.regrow = REGROW;
  const before = p.mats[mat];
  const ex = ox + dx * tEnd, ey = oy + dy * tEnd, ez = oz + dz * tEnd;
  if (node.left > 0) {
    const take = Math.min(PICKAXE.harvest, node.left);
    node.left -= take;
    p.mats[mat] = Math.min(MAX_MATS, before + take);
    if (node.left <= 0 && hit.cyl) {
      state.felled = [...state.felled, hit.index];
      const c = map.cylinders[hit.index];
      emit(state, 'fell', p.id, c.x, c.y0 + 1, c.z, { value: hit.index });
    }
  }
  emit(state, 'harvest', p.id, ex, ey, ez, { value: mat, flag: p.mats[mat] - before });
}

/** Forgets a harvested prop (it is whole again; a felled tree stands back up). */
function dropNode(state: ShooterState, index: number): void {
  const n = state.nodes[index];
  state.nodes.splice(index, 1);
  if (n.key < 0 && state.felled.includes(-1 - n.key)) state.felled = state.felled.filter(i => i !== -1 - n.key);
}
function stepNodes(state: ShooterState): void {
  for (let i = state.nodes.length - 1; i >= 0; i--) if ((state.nodes[i].regrow -= STEP) <= 0) dropNode(state, i);
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
  if (rng.chance(0.45)) dropNade(state, rng.int(4) as NadeKind, c.x + ox + 1.1, y, c.z + oz + 1.1);
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
      if (l.amount > 0 && free >= 0) { p.weapons[free] = { w: l.weapon, rarity: l.rarity, mag: l.amount, att: 0 }; l.amount = 0; }
      if (l.amount === before) continue;
      state.lootRev++;
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 5 });
      if (l.amount <= 0) takeLoot(state, i);
    } else if (l.kind === 'nade') {
      // Into the matching pocket: lethal (frag / molotov) or tactical (smoke / flash); a new kind replaces an empty pocket's.
      const kind = l.amount as NadeKind, lethal = NADES[kind]?.lethal;
      if (!NADES[kind]) continue;
      if (lethal) { if (p.lethalN >= MAX_NADES || (p.lethalN > 0 && p.lethal !== kind)) continue; p.lethal = kind; p.lethalN++; }
      else { if (p.tacN >= MAX_NADES || (p.tacN > 0 && p.tacKind !== kind)) continue; p.tacKind = kind; p.tacN++; }
      takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 7, flag: kind });
    } else if (l.kind === 'attach') {
      // Onto the weapon in hand if it takes it, else the first weapon that takes it and lacks it.
      const bit = ATTACHMENTS[l.amount]?.bit ?? 0;
      const fits = (h: Held | null) => !!h && isWeapon(h.w) && attachFits(h.w, bit) && !(h.att & bit);
      const h = fits(heldItem(p)) ? heldItem(p) : p.weapons.find(fits) ?? null;
      if (!h || !bit) continue;
      h.att |= bit;
      takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 6, flag: l.amount });
    } else if (l.kind === 'weapon' && isWeapon(l.weapon)) {
      const free = p.weapons.findIndex(w => w === null);
      if (free < 0 || p.weapons.some(w => w?.w === l.weapon && w.rarity >= l.rarity)) continue;
      p.weapons[free] = { w: l.weapon, rarity: l.rarity, mag: l.amount, att: 0 };
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

/** Nearest knocked teammate within reach (index into players, or -1). */
export function nearestDowned(state: ShooterState, p: Fighter, range: number): number {
  let best = -1, bd = range;
  for (const q of state.players) {
    if (q.mode !== 'down' || q.team !== p.team || q.id === p.id || Math.abs(q.y - p.y) > 2) continue;
    const d = Math.hypot(q.x - p.x, q.z - p.z);
    if (d < bd) { bd = d; best = q.id; }
  }
  return best;
}
/** Nearest landed supply drop within reach (index into drops, or -1). */
export function nearestDrop(state: ShooterState, p: Fighter, range = 2.6): number {
  let best = -1, bd = range;
  state.drops.forEach((d, i) => {
    if (!d.landed || Math.abs(d.y - p.y) > 2.2) return;
    const r = Math.hypot(d.x - p.x, d.z - p.z);
    if (r < bd) { bd = r; best = i; }
  });
  return best;
}

/** A supply drop over a dry spot inside the next safe circle. */
function spawnDrop(state: ShooterState, map: ShooterMap, world: World, rng: Rng): void {
  if (state.drops.length >= MAX_DROPS) return;
  const s = state.storm;
  for (let tries = 0; tries < 20; tries++) {
    const a = rng.range(0, Math.PI * 2), r = rng.range(0, Math.max(4, s.toR * 0.6));
    const x = clamp(s.toX + Math.cos(a) * r, -HALF + 12, HALF - 12), z = clamp(s.toZ + Math.sin(a) * r, -HALF + 12, HALF - 12);
    if (!dryLand(map, x, z) && tries < 19) continue;
    const ground = Math.max(groundAt(world, x, z, DROP_HEIGHT, state.builds), waterLevel(map, x, z));
    state.drops.push({ id: state.nextId++, x, y: DROP_HEIGHT, z, ground, landed: false });
    emit(state, 'supply', -1, x, DROP_HEIGHT, z, { value: state.drops[state.drops.length - 1].id, flag: 0 });
    return;
  }
}
function stepDrops(state: ShooterState): void {
  for (const d of state.drops) {
    if (d.landed) continue;
    d.y = Math.max(d.ground, d.y - DROP_SPEED * STEP);
    if (d.y <= d.ground) { d.landed = true; emit(state, 'supply', -1, d.x, d.y, d.z, { value: d.id, flag: 1 }); }
  }
}
/** Opens a supply drop: an epic-or-better weapon with ammo, shield potions, metal and sometimes a launch pad. */
function openDrop(state: ShooterState, index: number, by: number, rng: Rng): void {
  const d = state.drops[index];
  state.drops.splice(index, 1);
  const weapon = rollWeapon(rng.next());
  dropItem(state, 'weapon', weapon, Math.max(3, rollRarity(rng.next(), 2)) as Rarity, WEAPONS[weapon].mag, d.x + 1.4, d.y, d.z);
  dropLoot(state, 'ammo', WEAPONS[weapon].ammo, AMMO_PACK[WEAPONS[weapon].ammo] * 2, d.x + 1.2, d.y, d.z + 1.2);
  dropItem(state, 'item', 'shieldpot', 2, 2, d.x - 1.3, d.y, d.z + 0.4);
  dropMats(state, 2, 60, d.x - 0.6, d.y, d.z - 1.3);
  if (rng.chance(0.4)) dropItem(state, 'item', 'launchpad', 3, 1, d.x + 0.4, d.y, d.z - 1.5);
  dropAttach(state, rollAttach(rng.next()), d.x + 0.2, d.y, d.z + 1.6);
  dropNade(state, 0, d.x - 1.5, d.y, d.z - 0.2);
  // Whoever opens it is handed a killstreak (a UAV, sometimes an airstrike or a drone).
  const opener = state.players[by];
  if (opener) { const k = rng.chance(0.55) ? 0 : rng.chance(0.6) ? 1 : 2; opener.ready |= 1 << k; emit(state, 'streak', by, d.x, d.y, d.z, { value: k, flag: 0 }); }
  state.lootRev++;
  emit(state, 'supply', by, d.x, d.y, d.z, { value: d.id, flag: 2 });
}
/** Deploys a launch pad on open ground just ahead (false when there is no room). */
function placePad(state: ShooterState, world: World, p: Fighter): boolean {
  if (state.pads.length >= MAX_PADS) return false;
  const [fx, fz] = fwd(p.yaw);
  const x = p.x + fx * 2.2, z = p.z + fz * 2.2;
  const y = groundAt(world, x, z, p.y + 0.6, state.builds);
  if (Math.abs(y - p.y) > 1.2 || y < waterLevel(mapFor(state.map), x, z)) return false;
  for (const pad of state.pads) if (Math.hypot(pad.x - x, pad.z - z) < 3) return false;
  state.pads.push({ id: state.nextId++, x, y, z });
  emit(state, 'use', p.id, x, y, z, { value: 2, flag: 1 });
  return true;
}

function swapWeapon(state: ShooterState, p: Fighter): void {
  const l = nearestWeaponLoot(state, p);
  if (!l || !l.weapon) return;
  const free = p.weapons.findIndex(w => w === null);
  const slot = free >= 0 ? free : p.current < SLOTS ? p.current : 0;
  const old = p.weapons[slot];
  p.weapons[slot] = { w: l.weapon, rarity: l.rarity, mag: l.amount, att: 0 };
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
  const def = WEAPONS[w], st = weaponStats(def, held.att);
  held.mag--;
  p.cooldown = 1 / def.rate;
  p.shootT = 0.25;
  // Unsuppressed fire shows on the enemy radar for a moment.
  if (st.loud) p.spotted = 1.6;
  // The bullet goes where the sights are after the kick so far; then this shot's step of the pattern kicks.
  const yaw = p.yaw + p.kickYaw, pitch = clamp(p.pitch + p.kick, -1.5, 1.5);
  const step = def.pattern[p.shots % def.pattern.length];
  p.shots++;
  const kickMul = st.kick * (p.crouch ? 0.8 : 1);
  p.kick += step[0] * kickMul; p.kickYaw += step[1] * kickMul;
  const [fx, fz] = fwd(yaw);
  // Muzzle sits over the right shoulder; right of facing (fx, fz) is (-fz, fx).
  const ox = p.x - fz * 0.3, oy = p.y + (p.crouch ? EYE - 0.45 : EYE), oz = p.z + fx * 0.3;
  const cp = Math.cos(pitch);
  const baseX = fx * cp, baseY = Math.sin(pitch), baseZ = fz * cp;
  // Hip fire blooms; raised sights tighten towards the aimed spread; crouching steadies, moving / airborne / sliding loosen.
  const cone = def.spread + (st.adsSpread - def.spread) * p.ads;
  const spread = cone * (p.moving ? 1.4 : 1) * (p.grounded ? 1 : 2) * (p.crouch && p.slideT <= 0 ? 0.75 : 1) * (p.slideT > 0 ? 1.3 : 1) + p.bloom * (1 - 0.7 * p.ads) + (p.blind > 0 ? 0.06 : 0);
  p.bloom = Math.min(0.05, p.bloom + def.recoil * 0.5);
  if (def.projectile) {
    if (state.rockets.length < MAX_ROCKETS) {
      const s = def.projectile.speed;
      state.rockets.push({ id: state.nextId++, owner: p.id, rarity: held.rarity, x: ox + baseX, y: oy + baseY, z: oz + baseZ, vx: baseX * s, vy: baseY * s, vz: baseZ * s, life: 5 });
    }
    emit(state, 'shot', p.id, ox, oy, oz, { value: 5, x2: ox + baseX * 3, y2: oy + baseY * 3, z2: oz + baseZ * 3 });
    return;
  }
  const range = def.range * st.range;
  let firstEnd: [number, number, number] | null = null;
  _buildHits.length = 0; _buildDmg.length = 0;
  for (let k = 0; k < def.pellets; k++) {
    // Random direction inside the spread cone.
    const a = rng.range(0, Math.PI * 2), r = Math.sqrt(rng.next()) * spread;
    const ux = -fz, uz = fx; // horizontal perpendicular
    let dx = baseX + (ux * Math.cos(a) + 0) * r, dy = baseY + Math.sin(a) * r, dz = baseZ + uz * Math.cos(a) * r;
    const l = Math.hypot(dx, dy, dz); dx /= l; dy /= l; dz /= l;
    const wall = raycast(world, ox, oy, oz, dx, dy, dz, range, true, state.builds);
    let tEnd = wall.t, victim = -1, head = false, zv = -1;
    const struct = wall.kind === 'build' ? state.builds[wall.index] : null;
    for (const q of state.players) {
      // Shots pass through teammates.
      if (q.id === p.id || q.mode === 'dead' || q.mode === 'bus' || q.team === p.team) continue;
      const t = rayFighter(ox, oy, oz, dx, dy, dz, q, tEnd);
      if (t.t >= 0 && t.t < tEnd) { tEnd = t.t; victim = q.id; head = t.head; }
    }
    for (let i = 0; i < state.infected.length; i++) {
      const z = state.infected[i];
      if (z.hp <= 0) continue;
      const t = rayZombie(ox, oy, oz, dx, dy, dz, z, tEnd);
      if (t.t >= 0 && t.t < tEnd) { tEnd = t.t; zv = i; victim = -1; head = t.head; }
    }
    const ex = ox + dx * tEnd, ey = oy + dy * tEnd, ez = oz + dz * tEnd;
    if (!firstEnd) firstEnd = [ex, ey, ez];
    // A suppressor costs a little reach: falloff starts sooner.
    const far = tEnd / st.range;
    if (victim >= 0) {
      const q = state.players[victim];
      hurt(state, q, damageAt(def, held.rarity, far, head), p.id, head, true, w);
    } else if (zv >= 0) hurtZombie(state, state.infected[zv], damageAt(def, held.rarity, far, head), p.id, head, WEAPON_IDS.indexOf(w));
    else if (struct) {
      const at = _buildHits.indexOf(struct);
      const dmg = damageAt(def, held.rarity, far, false);
      if (at < 0) { _buildHits.push(struct); _buildDmg.push(dmg); } else _buildDmg[at] += dmg;
    }
  }
  for (let i = 0; i < _buildHits.length; i++) if (state.builds.includes(_buildHits[i])) damageBuild(state, world, _buildHits[i], _buildDmg[i]);
  const e = firstEnd!;
  emit(state, 'shot', p.id, ox, oy, oz, { value: WEAPON_IDS.indexOf(w), flag: st.loud ? 0 : 1, x2: e[0], y2: e[1], z2: e[2] });
  void map;
}

const _fh = { t: -1, head: false };
const SPHERES: [number, number, boolean][] = [[1.62, 0.3, true], [1.15, 0.46, false], [0.55, 0.42, false]];
/** Ray vs fighter: head sphere + two body spheres (squashed low while knocked). */
export function rayFighter(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, q: { x: number; y: number; z: number; mode?: FighterMode }, max: number): { t: number; head: boolean } {
  _fh.t = -1; _fh.head = false;
  const low = q.mode === 'down' ? 0.3 : 1;
  for (const [h, r, head] of SPHERES) {
    const cx = q.x - ox, cy = q.y + h * low + (low < 1 ? 0.2 : 0) - oy, cz = q.z - oz;
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

/** Kill / knock event weapon code for a source and weapon. */
const killCode = (from: number, src: DamageSource): number => (typeof src === 'number' ? src : from === FROM_STORM ? KILL_STORM : src === 'pickaxe' ? KILL_PICKAXE : WEAPON_IDS.indexOf(src));

/**
 * Damages a fighter. `from` is the fighter who dealt it (or FROM_STORM,
 * FROM_INFECTED, -1 for a fall); `src` names the weapon / kill code.
 */
export function hurt(state: ShooterState, q: Fighter, amount: number, from: number, head: boolean, useShield: boolean, src: DamageSource): void {
  // Once a winner is crowned nothing can take it away (storm, splash, falls); no friendly fire.
  if (q.mode === 'dead' || amount <= 0 || state.phase === 'done' || state.winner >= 0) return;
  if (from >= 0 && from !== q.id && state.players[from]?.team === q.team) return;
  const res = useShield ? applyDamage(q.hp, q.shield, amount) : { hp: Math.max(0, q.hp - amount), shield: q.shield, absorbed: 0 };
  q.hp = res.hp; q.shield = res.shield; q.hurtT = 0.4; q.useT = 0;
  if (from >= 0 && from !== q.id) {
    state.players[from].damage += amount;
    if (!q.human && q.mode !== 'down') { q.bot.target = from; q.bot.seen = 0.6; q.bot.calm = 0; q.bot.pushT = 0; q.bot.zt = -1; q.bot.seenX = state.players[from].x; q.bot.seenZ = state.players[from].z; q.bot.lost = 0; }
  }
  emit(state, 'hit', from, q.x, q.y + (q.mode === 'down' ? 0.5 : head ? 1.7 : 1.1), q.z, { target: q.id, value: amount, flag: (res.absorbed > 0 ? 1 : 0) | (head ? 2 : 0) });
  if (q.hp > 0) return;
  // Royale: the last team in it cannot be wiped out in the same instant as the one before it (storm ticks, splash): it wins instead.
  if (isRoyale(state.mode) && teamsAlive(state) <= 1) { q.hp = 1; return; }
  // Team play: knocked while a teammate still stands; finishing a knocked survivor eliminates them.
  const knocks = (isRoyale(state.mode) && state.squad > 1) || state.mode === 'zombies';
  if (q.mode !== 'down' && knocks && teamUp(state, q.team, q.id)) knock(state, q, from, src);
  else if (q.mode === 'down' && from < 0 && q.knocker >= 0) eliminate(state, q, q.knocker, KILL_BLEED);
  else eliminate(state, q, from, killCode(from, src));
}

function knock(state: ShooterState, q: Fighter, from: number, src: DamageSource): void {
  q.mode = 'down'; q.hp = DOWN_HP; q.shield = 0; q.knocker = from; q.revive = 0;
  q.building = false; q.useT = 0; q.reload = 0; q.aiming = false; q.sprinting = false; q.interact = 0; q.crouch = false; q.slideT = 0; q.ads = 0; q.mantleT = 0;
  emit(state, 'knock', from, q.x, q.y, q.z, { target: q.id, value: killCode(from, src) });
}

/** How long a fighter waits to respawn in the arena modes (seconds). */
export const RESPAWN = 4;

export function eliminate(state: ShooterState, q: Fighter, from: number, code: number): void {
  q.mode = 'dead'; q.killedBy = from; q.hp = 0; q.shield = 0; q.building = false; q.useT = 0; q.revive = 0;
  q.crouch = false; q.slideT = 0; q.ads = 0; q.kick = 0; q.kickYaw = 0; q.mantleT = 0; q.tactical = false; q.reload = 0; q.blind = 0;
  q.deaths++; q.streak = 0;
  const killer = from >= 0 && from !== q.id ? state.players[from] : null;
  if (killer) { killer.kills++; if (killer.team !== q.team) creditStreak(state, killer, 1); }
  emit(state, 'kill', from, q.x, q.y, q.z, { target: q.id, value: code });
  // The arena modes: back in after a short wait (the loadout comes back with the respawn); TDM scores the elimination.
  if (respawns(state.mode)) {
    q.respawn = RESPAWN;
    q.weapons = Array.from({ length: SLOTS }, () => null); q.current = PICKAXE_SLOT;
    if (state.mode === 'tdm' && (!killer || killer.team !== q.team) && state.obj.over < 0) state.obj.score[1 - q.team]++;
    return;
  }
  // Zombies: out until the next round starts (respawned with a pistol), or for good if nobody is left.
  if (state.mode === 'zombies') {
    q.weapons = Array.from({ length: SLOTS }, () => null); q.current = PICKAXE_SLOT; q.ammo = emptyAmmo(); q.lethalN = 0; q.tacN = 0;
    q.respawn = -1;
    return;
  }
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
  q.lethalN = 0; q.tacN = 0;
  // Nobody left standing on the team: the knocked go too (credited to whoever knocked them).
  if (!teamUp(state, q.team)) for (const m of state.players) if (m.team === q.team && m.mode === 'down') eliminate(state, m, m.knocker, KILL_BLEED);
  // Team wiped: every member takes the team's place (solo: the fighter's own).
  if (state.players.some(m => m.team === q.team && m.mode !== 'dead')) return;
  const place = teamsAlive(state) + 1;
  for (const m of state.players) if (m.team === q.team && m.place === 0) m.place = place;
}

/**
 * Killstreak progress: eliminations since the last death (`weight` per kill;
 * an infected counts a quarter). Crossing a streak's count earns it (kept
 * until called in).
 */
export function creditStreak(state: ShooterState, p: Fighter, weight: number): void {
  const before = p.streak;
  p.streak += weight;
  for (let k = 0; k < STREAKS.length; k++) {
    const need = STREAKS[k].kills;
    if (before < need && p.streak >= need && !(p.ready & (1 << k))) { p.ready |= 1 << k; emit(state, 'streak', p.id, p.x, p.y, p.z, { value: k, flag: 0 }); }
  }
}

function stepRockets(state: ShooterState, world: World): void {
  for (let i = state.rockets.length - 1; i >= 0; i--) {
    const r = state.rockets[i];
    r.life -= STEP;
    r.vy -= 2 * STEP;
    const sp = Math.hypot(r.vx, r.vy, r.vz), dx = r.vx / sp, dy = r.vy / sp, dz = r.vz / sp, step = sp * STEP;
    const hit = raycast(world, r.x, r.y, r.z, dx, dy, dz, step, true, state.builds);
    let t = hit.kind === 'none' ? -1 : hit.t;
    const team = state.players[r.owner]?.team ?? -1;
    for (const q of state.players) {
      if (q.id === r.owner || q.mode === 'dead' || q.mode === 'bus' || q.team === team) continue;
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
  blast(state, world, r.owner, r.rarity, WEAPONS.rocket.projectile!.radius, damageAt(WEAPONS.rocket, r.rarity, 0, false), 320, 'rocket', x, y, z);
}

/**
 * An explosion (rockets, frags, airstrike bombs): splash damage to fighters
 * and the infected (half to the thrower), heavy damage to every structure in
 * the radius.
 */
export function blast(state: ShooterState, world: World, owner: number, rarity: Rarity, radius: number, damage: number, structure: number, src: DamageSource, x: number, y: number, z: number): void {
  void rarity;
  emit(state, 'explode', owner, x, y, z, { value: radius });
  for (const q of state.players) {
    if (q.mode === 'dead' || q.mode === 'bus') continue;
    const d = Math.hypot(q.x - x, q.y + 1 - y, q.z - z);
    if (d > radius) continue;
    const amount = damage * (1 - (d / radius) * 0.7) * (q.id === owner ? 0.5 : 1);
    hurt(state, q, Math.round(amount), owner, false, true, src);
  }
  const code = killCode(owner, src);
  for (const zb of state.infected) {
    if (zb.hp <= 0) continue;
    const d = Math.hypot(zb.x - x, zb.y + 1 - y, zb.z - z);
    if (d <= radius) hurtZombie(state, zb, Math.round(damage * (1 - (d / radius) * 0.6)), owner, false, code);
  }
  // Rockets are the structure breakers: heavy damage to every piece in the blast.
  _buildHits.length = 0; _buildDmg.length = 0;
  for (const b of state.builds) {
    const a = b.aabb;
    const d = Math.hypot(clamp(x, a.minX, a.maxX) - x, clamp(y, a.minY, a.maxY) - y, clamp(z, a.minZ, a.maxZ) - z);
    if (d <= radius) { _buildHits.push(b); _buildDmg.push(Math.round(structure * (1 - (d / radius) * 0.6))); }
  }
  const hits = _buildHits.slice(), dmg = _buildDmg.slice();
  for (let i = 0; i < hits.length; i++) if (state.builds.includes(hits[i])) damageBuild(state, world, hits[i], dmg[i]);
}

/**
 * The storm (Apocalypse Royale: a toxic cloud that hits half as hard again).
 * In the arena modes and Zombies it is a fixed boundary instead: the schedule
 * never runs, only the damage outside it.
 */
function stepStorm(state: ShooterState, map: ShooterMap, world: World, rng: Rng): void {
  if (state.phase !== 'play') return;
  const s = state.storm, scale = state.stormScale || 1;
  if (isRoyale(state.mode)) s.timer -= STEP;
  if (!isRoyale(state.mode)) { /* fixed boundary */ } else if (!s.shrinking && s.timer <= 0) {
    const next = STORM[Math.min(STORM.length - 1, s.stage + 1)];
    s.stage = Math.min(STORM.length - 1, s.stage + 1);
    s.shrinking = true; s.timer = next.shrink * scale;
    s.fromX = s.cx; s.fromZ = s.cz; s.fromR = s.r;
    s.damage = Math.round(next.damage * (state.mode === 'apoc' ? 1.5 : 1));
    emit(state, 'storm', -1, s.toX, 0, s.toZ, { value: s.stage, flag: 1 });
    if (SUPPLY_STAGES.includes(s.stage)) spawnDrop(state, map, world, rng);
  } else if (s.shrinking) {
    const total = STORM[s.stage].shrink * scale, k = clamp(1 - s.timer / total, 0, 1);
    s.cx = s.fromX + (s.toX - s.fromX) * k; s.cz = s.fromZ + (s.toZ - s.fromZ) * k; s.r = s.fromR + (s.toR - s.fromR) * k;
    if (s.timer <= 0) {
      s.shrinking = false; s.cx = s.toX; s.cz = s.toZ; s.r = s.toR;
      if (s.stage < STORM.length - 1) { s.timer = STORM[s.stage + 1].wait * scale; pickStormTarget(state, rng); emit(state, 'storm', -1, s.toX, 0, s.toZ, { value: s.stage + 1, flag: 0 }); }
      else s.timer = 9999;
    }
  }
  for (const p of state.players) {
    if (p.mode !== 'ground' && p.mode !== 'glide' && p.mode !== 'down') continue;
    if (Math.hypot(p.x - s.cx, p.z - s.cz) <= s.r) { p.stormT = 0; continue; }
    p.stormT += STEP;
    if (p.stormT >= 1) { p.stormT -= 1; hurt(state, p, Math.max(1, s.damage), FROM_STORM, false, false, KILL_STORM); }
  }
}

export interface ShooterResult { id: number; name: string; fighter: FighterId; human: boolean; slot: number; team: number; place: number; kills: number; deaths: number; points: number; damage: number; time: number }
export function shooterResults(state: ShooterState): ShooterResult[] {
  return [...state.players].sort((a, b) => (a.place || 99) - (b.place || 99) || (state.squad > 1 ? a.team - b.team : 0) || b.kills - a.kills || a.deaths - b.deaths)
    .map(p => ({ id: p.id, name: p.name, fighter: p.fighter, human: p.human, slot: p.slot, team: p.team, place: p.place || (isRoyale(state.mode) ? teamsAlive(state) : 2), kills: p.kills, deaths: p.deaths, points: p.points, damage: Math.round(p.damage), time: p.aliveTime }));
}
export { HEIGHT };
