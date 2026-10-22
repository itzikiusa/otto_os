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
import { HALF, mapFor, terrainHeight, waterLevel, type ShooterMap } from './map.ts';
import { groundAt, HEIGHT, raycast, resolve, syncFelled, worldFor, type World } from './collide.ts';
import { AMMO_KINDS, AMMO_PACK, canUse, CONSUMABLE_PACK, CONSUMABLES, isConsumable, isWeapon, rollAttach, rollConsumable, rollRarity, rollWeapon, weaponStats, WEAPONS, type Material, type NadeKind, type WeaponId } from './weapons.ts';
import type { BuildKind } from './builds.ts';
import { botThink } from './bots.ts';
import type { FighterId, ShooterMode } from './catalog.ts';
import { hasInfected, isRoyale, respawns, setupMode, stepMode } from './modes/index.ts';
import { stepGear, throwNade, useStreak } from './modes/gear.ts';
import { buyWall, stepInfected } from './zombies.ts';
import type { ShooterSeat } from '../setup.ts';
import { STEP, MAX_FIGHTERS, BUS_HEIGHT, BUS_SPEED, SLOTS, PICKAXE_SLOT, PICKAXE_KEY, SWIM_DEPTH, idleShooterInput, STORM, START_RADIUS, KILL_BLEED, TAC_TIME, SLIDE_TIME, BLEED, CRAWL, REVIVE_TIME, REVIVE_RANGE, REVIVE_HP, LAUNCH_SPEED, emptyAmmo, emptyBot, standing, teamsAlive, heldItem, type TeamSize, type ShooterSetup, type ShooterInput, type Fighter, type ShooterState } from './sim/model.ts';
import { emit, dropLoot, dropItem, dropMats, dropAttach, dropNade, fwd } from './sim/core.ts';
import { pickStormTarget, stepStorm } from './sim/storm.ts';
import { tryBuild, tryEdit, growBuilds, stepNodes } from './sim/building.ts';
import { swing, fire, hurt, eliminate, stepRockets } from './sim/combat.ts';
import { nearestChest, openChest, pickupAuto, nearestDowned, nearestDrop, stepDrops, openDrop, placePad, swapWeapon } from './sim/loot.ts';
export * from './sim/model.ts';
export { emit, dropLoot, dropItem, dropAttach, dropNade, fwd } from './sim/core.ts';
export { damageBuild } from './sim/building.ts';
export { rayFighter, hurt, RESPAWN, eliminate, creditStreak, blast } from './sim/combat.ts';
export { nearestWeaponLoot, nearestClosedChest, nearestDowned, nearestDrop } from './sim/loot.ts';

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


export interface ShooterResult { id: number; name: string; fighter: FighterId; human: boolean; slot: number; team: number; place: number; kills: number; deaths: number; points: number; damage: number; time: number }
export function shooterResults(state: ShooterState): ShooterResult[] {
  return [...state.players].sort((a, b) => (a.place || 99) - (b.place || 99) || (state.squad > 1 ? a.team - b.team : 0) || b.kills - a.kills || a.deaths - b.deaths)
    .map(p => ({ id: p.id, name: p.name, fighter: p.fighter, human: p.human, slot: p.slot, team: p.team, place: p.place || (isRoyale(state.mode) ? teamsAlive(state) : 2), kills: p.kills, deaths: p.deaths, points: p.points, damage: Math.round(p.damage), time: p.aliveTime }));
}
export { HEIGHT };
