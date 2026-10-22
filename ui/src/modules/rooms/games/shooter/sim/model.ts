// Storm Royale model: the tuning constants, fighter / loot / event shapes,
// the replicated ShooterState and small read-only helpers (re-exported by ../sim.ts).
import { HALF } from '../map.ts';
import type { AmmoKind, ItemId, Material, NadeKind, Rarity, WeaponId } from '../weapons.ts';
import type { Build, BuildKind } from '../builds.ts';
import type { FighterId, ShooterMapId, ShooterMode } from '../catalog.ts';
import type { ModeState } from '../modes/index.ts';
import type { Area, Drone, Nade, Strike } from '../modes/gear.ts';
import type { Zombie } from '../zombies.ts';
import type { ShooterSeat } from '../../setup.ts';

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
export const EYE = 1.55;
