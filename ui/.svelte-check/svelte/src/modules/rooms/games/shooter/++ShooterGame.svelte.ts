///<reference types="svelte" />
;
import { onMount, untrack } from 'svelte';
import LoadState from '../../../../lib/components/LoadState.svelte';
import {
    aliveCount, createMatch, heldItem, idleShooterInput, nearestClosedChest, nearestDowned, nearestDrop, nearestWeaponLoot, PICKAXE_KEY, PICKAXE_SLOT, REVIVE_RANGE, REVIVE_TIME,
    shooterResults, SLOTS, stepMatch, STORM, teamsAlive,
    type Fighter, type Held, type ShooterEvent, type ShooterInput, type ShooterLink, type ShooterSetup, type ShooterState,
  } from './sim';
import { mapFor } from './map';
import {
    AMMO_KINDS, ATTACHMENTS, BUILD_COST, CLASSES, CONSUMABLES, canUse, isConsumable, isWeapon, itemName, MATERIAL_NAMES, RARITY_COLOURS, RARITY_NAMES, STREAKS, weaponStats, WEAPONS,
  } from './weapons';
import { BUILD_KIND_NAMES } from './builds';
import { fighterName, modeName as modeTitle, SHOOTER_MAPS, TEAM_MODES, teamColour, type FighterId } from './catalog';
import { isRoyale, POINT_NAMES, respawns, WAVE_BOARD, WAVE_HELI, WAVE_ON, WAVE_PRE } from './modes/index';
import { buyWallsFor, nearestWall } from './zombies';
import { InputState } from '../common/input';
import { lossBudget } from '../../../../lib/three/stage';
import { GameAudio } from '../common/audio';
import { clamp, lerp, ordinal, wrapAngle } from '../common/math';
import type { LocalBuild, ShooterRenderer } from './render';
import { drawMap, hex, type MapCache } from './hud/minimap';
import TeamPanel from './hud/TeamPanel.svelte';
import Vitals from './hud/Vitals.svelte';
import MatchResults from './hud/MatchResults.svelte';
import PauseMenu from './hud/PauseMenu.svelte';
import Knocked from './hud/Knocked.svelte';
import { announce } from '../../../../lib/announce.svelte';
;

  interface Props {
    setup: ShooterSetup | null; selfSlot: number; fighter: FighterId; link?: ShooterLink;
    names?: (slot: number) => string; paused?: boolean;
    /** Host: one snapshot. Loot and structures are never both included (relay size cap). */
    onframe?: (state: ShooterState, events: ShooterEvent[], withLoot: boolean, withBuilds: boolean) => void;
    oninput?: (input: ShooterInput, fighter: FighterId) => void;
    onfinish?: (state: ShooterState) => void;
    onexit: () => void; onrematch: () => void; rematchLabel?: string; rematchDisabled?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { setup, selfSlot, fighter, link, names = () => 'Player', paused = false, onframe, oninput, onfinish, onexit, onrematch, rematchLabel = 'Play again', rematchDisabled = false }: Props = $props();

  let stage: HTMLDivElement, root: HTMLDivElement;
  let minimap = $state<HTMLCanvasElement>();
  let bigmap = $state<HTMLCanvasElement>();
  let overlay = $state<HTMLCanvasElement>();
  let loading = $state(true), error = $state(''), menu = $state(false), muted = $state(false), fullscreen = $state(false), locked = $state(false), focused = $state(false);
  let hud = $state.raw<ShooterState | null>(null);
  let feed = $state<{ id: number; text: string; mine: boolean }[]>([]);
  let toast = $state(''), prompt = $state(''), hitMark = $state(0), hurtDir = $state<number | null>(null);
  let aiming = $state(false), spectate = $state(-1);
  /**
   * Gunplay HUD: hitmarker (time, kill), the class to respawn with (arena
   * modes, remembered), and the death recap (who, with what, from how far,
   * how much health they had left, damage both ways).
   */
  let hitKill = $state(false), chosenClass = $state(0);
  let recap = $state<{ killer: string; how: string; distance: number; left: number; dealt: number; taken: number; head: boolean } | null>(null);
  /** Damage dealt to / taken from each opponent this life (death recap). */
  const dealt = new Map<number, number>(), taken = new Map<number, number>();
  let lastHeadFrom = -1;
  /** Build mode, the selected piece (wall / floor / ramp) and material; the full-screen map. */
  let building = $state(false), piece = $state(0), material = $state(0), showMap = $state(false), ghostOk = $state(false);
  const audio = new GameAudio(['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket', 'explosion', 'hit', 'shieldhit', 'headshot', 'reload', 'pickup', 'chest', 'storm', 'eliminated', 'land', 'voice-win', 'voice-over', 'voice-hurry', 'voice-go', 'select', 'step', 'shield', 'bump', 'tick', 'drop', 'voice-1', 'voice-2', 'voice-3', 'item']);
  let renderer: ShooterRenderer | undefined, input: InputState | undefined;
  let stop: (() => void) | undefined;
  const isGuest = $derived(link?.role === 'guest');
  const cam = { yaw: 0, pitch: -0.3, aiming: false, zoom: 70 };
  let slotWanted = 0;
  const numbers: { x: number; y: number; z: number; text: string; color: string; t: number }[] = [];
  const GUN_ICON: Record<string, string> = { pistol: 'gun-pistol', smg: 'gun-smg', rifle: 'gun-rifle', shotgun: 'gun-shotgun', sniper: 'gun-sniper', rocket: 'gun-rocket' };
  const KILL_HOW = ['Sidearm', 'Buzz SMG', 'Assault Blaster', 'Pump Scatter', 'Longshot Rail', 'Boom Tube', 'the storm', 'Pickaxe', 'bled out', 'Frag', 'Molotov', 'Airstrike', 'Attack Drone', 'the infected', 'Acid'];
  /** Double-tapped Shift: tactical sprint while it stays held. */
  let tacOn = false, lastShift = 0;
  /** Build intent for the preview ghost, predicted locally (reused every frame). */
  const local: LocalBuild = { building: false, piece: 0, mat: 0, yaw: 0, pitch: 0, x: 0, y: 0, z: 0 };
  /** E2E hook: `window.__ottoGameTest = { stormScale }` shortens the storm so a match ends in about a minute. */
  const testHook = () => (globalThis as { __ottoGameTest?: { stormScale?: number } }).__ottoGameTest;

  const selfOf = (s: ShooterState) => Math.max(0, s.players.findIndex(p => p.human && p.slot === selfSlot));

  function readInput(state: ShooterState, me: number): ShooterInput {
    const i = input;
    const p = state.players[me];
    if (!i || menu || paused || !p) return idleShooterInput(cam.yaw, cam.pitch);
    const [dx, dy] = i.consumeLook();
    const sens = (cam.aiming ? 0.0012 : 0.0026) * (cam.zoom / 70);
    cam.yaw = wrapAngle(cam.yaw - dx * sens);
    cam.pitch = clamp(cam.pitch - dy * sens, -1.25, 1.2);
    if (i.hit('KeyM')) showMap = !showMap;
    // Arena modes: 6–9 pick the class to respawn with.
    if (respawns(state.mode)) for (let k = 0; k < CLASSES.length; k++) if (i.hit(`Digit${6 + k}`)) chosenClass = k;
    const now = performance.now();
    if (i.hit('ShiftLeft', 'ShiftRight')) { tacOn = now - lastShift < 320; lastShift = now; }
    if (!i.down('ShiftLeft', 'ShiftRight')) tacOn = false;
    // Q / B toggle build mode; F draws the pickaxe (and leaves build mode).
    if (p.mode === 'ground' && i.hit('KeyQ', 'KeyB', 'Build')) building = !building;
    if (i.hit('KeyF', 'Pick')) { building = false; slotWanted = PICKAXE_KEY; }
    let edit = false;
    if (building) {
      // Build mode: 1-3 / wheel pick the piece, R cycles material, G edits the piece in the crosshair.
      if (i.wheel) piece = (piece + (i.wheel > 0 ? 1 : 2)) % 3;
      for (let k = 1; k <= 3; k++) if (i.hit(`Digit${k}`)) piece = k - 1;
      if (i.hit('KeyR', 'Reload')) material = (material + 1) % 3;
      if (p.mats[material] < BUILD_COST) { const has = [0, 1, 2].find(m => p.mats[m] >= BUILD_COST); if (has !== undefined) material = has; }
      edit = i.down('KeyG', 'Edit') || i.hit('KeyG', 'Edit');
      nades.lethal = nades.tactical = false;
    } else {
      // Combat: G frag / molotov, T smoke / flash (Q is the build toggle).
      nades.lethal = i.down('KeyG', 'Nade') || i.hit('KeyG', 'Nade');
      nades.tactical = i.down('KeyT') || i.hit('KeyT');
    }
    if (!building) {
      // Hotbar: 1-5, wheel over the slots and the pickaxe.
      if (i.wheel) slotWanted = ((p.current + (i.wheel > 0 ? 1 : -1)) % (SLOTS + 1) + SLOTS + 1) % (SLOTS + 1) + 1;
      for (let k = 1; k <= SLOTS; k++) if (i.hit(`Digit${k}`)) slotWanted = k;
    }
    const slot = slotWanted; slotWanted = 0;
    if (p.mode !== 'ground' || p.swimming) building = false;
    const held = heldItem(p);
    const aim = !building && (i.mouse.right || i.down('KeyZ'));
    cam.aiming = aim && p.mode === 'ground' && !!held && isWeapon(held.w);
    const def = held && isWeapon(held.w) ? WEAPONS[held.w] : null;
    // The view zooms in as the sights come up (a scope zooms further); a tactical sprint widens it.
    const zoom = def ? weaponStats(def, held!.att).zoom : 55;
    const ads = p.ads > 0 ? p.ads : cam.aiming ? 0.35 : 0;
    cam.zoom = p.mode === 'sky' || p.mode === 'glide' ? 78 : 70 + (zoom - 70) * ads + (p.tactical ? 6 : 0);
    aiming = cam.aiming && p.ads > 0.85 && !!def && (def.id === 'sniper' || (held!.att & 1) === 1);
    // Aim the muzzle at whatever is under the crosshair (the build ghost follows the camera).
    let yaw = cam.yaw, pitch = cam.pitch;
    if (renderer && p.mode === 'ground' && !building) {
      const t = renderer.aimPoint;
      const [ex, ey, ez] = renderer.eye(p);
      yaw = Math.atan2(t.x - ex, t.z - ez);
      pitch = Math.atan2(t.y - ey, Math.max(0.5, Math.hypot(t.x - ex, t.z - ez)));
      if (Math.abs(wrapAngle(yaw - cam.yaw - p.kickYaw)) > 0.6) { yaw = cam.yaw + p.kickYaw; pitch = cam.pitch + p.kick; }
      // The view climbs with the recoil (the host adds the kick to every shot): aim from under it.
      yaw -= p.kickYaw; pitch -= p.kick;
    }
    return {
      mx: (i.down('KeyD', 'ArrowRight', 'Right') ? 1 : 0) - (i.down('KeyA', 'ArrowLeft', 'Left') ? 1 : 0),
      mz: (i.down('KeyW', 'ArrowUp', 'Up') ? 1 : 0) - (i.down('KeyS', 'ArrowDown', 'Down') ? 1 : 0),
      yaw, pitch, fire: (i.mouse.left && locked) || i.down('Fire') || (i.mouse.left && focused && !locked && p.mode === 'ground'),
      aim, jump: i.down('Space', 'Jump') || i.hit('Space', 'Jump'), sprint: i.down('ShiftLeft', 'ShiftRight'),
      reload: !building && (i.down('KeyR', 'Reload') || i.hit('KeyR', 'Reload')), interact: i.down('KeyE', 'Use') || i.hit('KeyE', 'Use'), slot,
      build: building, piece, mat: material, edit,
      crouch: i.down('KeyC', 'Crouch') || i.hit('KeyC', 'Crouch'), tac: tacOn || i.down('Tac'),
      lethal: nades.lethal, tactical: nades.tactical, streak: i.down('KeyX', 'Streak') || i.hit('KeyX', 'Streak'), cls: chosenClass,
    };
  }
  const nades = { lethal: false, tactical: false };

  function say(text: string, ms = 1800) { toast = text; announce(text); const shown = text; setTimeout(() => { if (toast === shown) toast = ''; }, ms); }
  function post(id: number, text: string, mine: boolean) {
    feed = [...feed.slice(-4), { id, text, mine }];
    setTimeout(() => { feed = feed.filter(f => f.id !== id); }, 6000);
  }
  function nameOf(state: ShooterState, id: number): string { if (id === -2) return state.mode === 'apoc' ? 'the toxic cloud' : 'the storm'; if (id === -3) return 'the infected'; if (id < 0) return 'a fall'; return state.players[id]?.name ?? 'someone'; }
  function hitmark(now: number, kill: boolean) { hitMark = now; hitKill = kill; }
  const SHOT_SOUNDS = ['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket'];

  function onEvent(state: ShooterState, e: ShooterEvent, me: number, now: number) {
    renderer?.effect(e, state);
    const p = state.players[me];
    const dist = p ? Math.hypot(e.x - p.x, e.z - p.z) : 0;
    switch (e.type) {
      case 'shot': audio.play(SHOT_SOUNDS[e.value] ?? 'pistol', { distance: e.player === me ? 0 : dist, volume: e.player === me ? 0.55 : 0.8, throttle: 0.02 }); break;
      case 'zhit':
        if (e.player === me) {
          audio.play(e.flag & 2 ? 'headshot' : 'hit', { volume: 0.45, throttle: 0.04 });
          hitmark(now, false);
          numbers.push({ x: e.x, y: e.y + 0.4, z: e.z, text: String(e.value), color: e.flag & 2 ? '#ffd23f' : '#c8f08a', t: now });
          if (numbers.length > 40) numbers.shift();
        }
        break;
      case 'zkill': if (e.player === me) { audio.play('tick', { volume: 0.7, throttle: 0.05 }); hitmark(now, true); } break;
      case 'hit':
        if (e.player >= 0 && e.target === me) { taken.set(e.player, (taken.get(e.player) ?? 0) + e.value); if (e.flag & 2) lastHeadFrom = e.player; }
        if (e.player === me && e.target >= 0) dealt.set(e.target, (dealt.get(e.target) ?? 0) + e.value);
        if (e.player === me) {
          audio.play(e.flag & 2 ? 'headshot' : e.flag & 1 ? 'shieldhit' : 'hit', { volume: 0.6, throttle: 0.03 });
          hitmark(now, false);
          numbers.push({ x: e.x, y: e.y + 0.4, z: e.z, text: String(e.value), color: e.flag & 2 ? '#ffd23f' : e.flag & 1 ? '#7cc8ff' : '#ffffff', t: now });
          if (numbers.length > 40) numbers.shift();
        }
        if (e.target === me && p) {
          const src = e.player >= 0 ? state.players[e.player] : null;
          hurtDir = src ? Math.atan2(src.x - p.x, src.z - p.z) - cam.yaw : null;
          setTimeout(() => { hurtDir = null; }, 900);
        }
        break;
      case 'kill': {
        const killer = nameOf(state, e.player), victim = nameOf(state, e.target);
        const how = e.player === -2 ? 'was caught in the storm' : e.player < 0 ? 'fell' : `eliminated by ${killer}`;
        const weapon = KILL_HOW[e.value] ?? '';
        post(e.id, e.player >= 0 ? `${killer} ⟶ ${victim}${weapon ? ` · ${weapon}` : ''}` : e.target === me ? `You ${how.replace('was ', 'were ')}` : `${victim} ${how}`, e.player === me || e.target === me);
        if (e.player === me && e.target !== me) { audio.play('eliminated'); hitmark(now, true); say(isRoyale(state.mode) ? `You eliminated ${victim} · ${aliveCount(state)} left` : `You eliminated ${victim}`); }
        if (e.target === me) {
          audio.play('voice-over'); say(e.player >= 0 ? `Eliminated by ${killer}` : `You ${how.replace('was ', 'were ')}`, 3000);
          // Death recap: who, with what, from how far, how much they had left, damage both ways this life.
          const k = e.player >= 0 ? state.players[e.player] : null;
          recap = { killer: e.player >= 0 ? killer : e.player === -2 ? (state.mode === 'apoc' ? 'The toxic cloud' : 'The storm') : e.player === -3 ? 'The infected' : 'A fall', how: KILL_HOW[e.value] ?? '',
            distance: k ? Math.round(Math.hypot(k.x - e.x, k.z - e.z)) : 0, left: k ? Math.round(k.hp + k.shield) : 0,
            dealt: Math.round(dealt.get(e.player) ?? 0), taken: Math.round(taken.get(e.player) ?? 0), head: lastHeadFrom === e.player && e.player >= 0 };
          dealt.clear(); taken.clear(); lastHeadFrom = -1;
          // Team modes: watch a teammate who is still in it first.
          const mate = state.squad > 1 ? state.players.find(q => q.team === p?.team && q.id !== me && q.mode !== 'dead') : undefined;
          spectate = mate ? mate.id : e.player >= 0 ? e.player : -1;
        }
        break;
      }
      case 'knock': {
        const killer = nameOf(state, e.player), victim = nameOf(state, e.target);
        post(e.id, `${e.player >= 0 ? killer : e.player === -2 ? 'The storm' : 'A fall'} knocked ${victim}${KILL_HOW[e.value] && e.player >= 0 ? ` · ${KILL_HOW[e.value]}` : ''}`, e.player === me || e.target === me);
        if (e.target === me) { audio.play('voice-hurry'); say('Knocked! Crawl to a teammate — they can revive you', 3000); }
        else if (e.player === me) { audio.play('eliminated', { volume: 0.6 }); say(`You knocked ${victim}`); }
        else if (p && state.players[e.target]?.team === p.team) say(`${victim} is down — hold E next to them to revive`, 2600);
        break;
      }
      case 'revive':
        post(e.id, `${nameOf(state, e.player)} revived ${nameOf(state, e.target)}`, e.player === me || e.target === me);
        if (e.target === me || e.player === me) { audio.play('shield'); say(e.target === me ? `${nameOf(state, e.player)} got you back up` : `You revived ${nameOf(state, e.target)}`); }
        break;
      case 'supply':
        if (e.flag === 0) { audio.play('voice-go', { volume: 0.7 }); say('Supply drop incoming — look for the beam (M)', 2600); }
        else if (e.flag === 1) audio.play('land', { distance: dist, volume: 0.9 });
        else { audio.play('chest', { distance: dist }); if (e.player === me) say('Supply drop opened'); }
        break;
      case 'launch': audio.play('select', { distance: e.player === me ? 0 : dist, volume: 0.8 }); break;
      case 'nade': if (e.player === me) audio.play('select', { volume: 0.5 }); break;
      case 'fire': audio.play('explosion', { distance: dist, volume: 0.5, rate: 1.4 }); break;
      case 'smoke': audio.play('bump', { distance: dist, volume: 0.6 }); break;
      case 'flash':
        if (e.target < 0) audio.play('explosion', { distance: dist, volume: 0.8, rate: 1.8 });
        else if (e.target === me) say('Flashed!', 900);
        break;
      case 'streak': {
        const st = STREAKS[e.value];
        if (!st) break;
        if (e.flag === 0 && e.player === me) { audio.play('voice-go', { volume: 0.8 }); say(`${st.name} ready — press X`, 2400); }
        else if (e.flag === 1) {
          const mine = e.player === me || (p && state.players[e.player]?.team === p.team && state.squad > 1);
          audio.play('item', { volume: 0.8 });
          post(e.id, `${mine ? 'Friendly' : 'Enemy'} ${st.name} ${st.id === 'airstrike' ? 'inbound' : 'up'}${e.player >= 0 ? ` · ${nameOf(state, e.player)}` : ''}`, !!mine);
        }
        break;
      }
      case 'capture': audio.play('chest', { volume: 0.8 }); say(`${POINT_NAMES[e.value] ?? '?'} captured by ${p && e.flag === p.team ? 'your team' : 'the enemy'}`, 1800); break;
      case 'round': audio.play('voice-hurry'); say(`Round ${e.value}`, 2400); break;
      case 'extract':
        audio.play(e.flag === 3 ? 'voice-over' : 'voice-go');
        say(e.flag === 0 ? 'Helicopter inbound — survive, then get to the landing zone (M)' : e.flag === 1 ? 'The helicopter has landed — get aboard!' : e.flag === 2 ? `Extracted! ${e.value} aboard` : `Overrun on round ${e.value}`, 3600);
        break;
      case 'buy':
        if (e.player === me) { if (e.flag === 1) { audio.play('pickup'); say(`Bought ${WEAPONS[buyWallsFor(mapFor(state.map))[e.value]?.w ?? 'pistol'].name}`); } else say('Not enough points', 1200); }
        break;
      case 'spawn': if (e.player === me) { audio.play('select'); recap = null; } break;
      case 'repair': if (e.player === me) audio.play('tick', { volume: 0.5, throttle: 0.06 }); break;
      case 'fell': audio.play('bump', { distance: dist, volume: 0.9, throttle: 0.05 }); break;
      case 'pickup': if (e.player === me) audio.play('pickup', { volume: 0.5 }); break;
      case 'reload': if (e.player === me) audio.play('reload', { volume: 0.6 }); break;
      case 'chest': audio.play('chest', { distance: dist }); break;
      case 'explode': audio.play('explosion', { distance: dist, volume: 1.2 }); break;
      case 'land': if (e.player === me) audio.play('land'); break;
      case 'storm': audio.play('storm', { volume: 0.7 }); if (e.flag === 1) { audio.play('voice-hurry'); say(`Storm phase ${e.value + 1} of ${STORM.length} — the storm is closing in!`); } else say('Storm eye revealed — check the map (M)', 2200); break;
      case 'win': if (e.player === me || (p && state.squad > 1 && e.value === p.team)) { audio.play('voice-win'); say(isRoyale(state.mode) ? 'Victory Royale!' : state.mode === 'zombies' ? 'Extracted!' : 'Victory!', 4000); } else if (!isRoyale(state.mode) && state.mode !== 'zombies') say('Defeat', 3000); break;
      case 'jump': if (e.player === me) audio.play('select'); break;
      case 'build': audio.play('drop', { distance: e.player === me ? 0 : dist, volume: 0.5, throttle: 0.05 }); break;
      case 'break': audio.play('bump', { distance: dist, volume: 0.8, throttle: 0.05 }); break;
      case 'harvest': if (e.player === me) audio.play('tick', { volume: 0.6, throttle: 0.05 }); break;
      case 'swing': if (e.value > 0) audio.play('bump', { distance: e.player === me ? 0 : dist, volume: 0.4, throttle: 0.05 }); break;
      case 'use': if (e.player === me) audio.play('shield', { volume: 0.6 }); break;
      case 'edit': if (e.player === me) audio.play('select', { volume: 0.5 }); break;
    }
  }

  const mapCache: MapCache = { image: null };
  function drawOverlay(now: number) {
    const c = overlay; if (!c || !renderer) return;
    const g = c.getContext('2d'); if (!g) return;
    if (c.width !== c.clientWidth || c.height !== c.clientHeight) { c.width = c.clientWidth; c.height = c.clientHeight; }
    g.clearRect(0, 0, c.width, c.height);
    g.font = '700 20px system-ui, sans-serif'; g.textAlign = 'center';
    for (let i = numbers.length - 1; i >= 0; i--) {
      const n = numbers[i], age = (now - n.t) / 1000;
      if (age > 0.9) { numbers.splice(i, 1); continue; }
      const pt = renderer.project(n.x, n.y + age * 1.2, n.z);
      if (!pt) continue;
      g.globalAlpha = 1 - age / 0.9;
      g.lineWidth = 4; g.strokeStyle = 'rgba(0,0,0,0.6)'; g.strokeText(n.text, pt.x, pt.y);
      g.fillStyle = n.color; g.fillText(n.text, pt.x, pt.y);
    }
    g.globalAlpha = 1;
  }

  /** Guest: interpolate fighters between buffered snapshots. */
  let view: ShooterState | null = null;
  function interpolated(now: number): ShooterState | null {
    const buf = link?.snapshots;
    if (!buf?.length) return null;
    const t = now - 100;
    let a = buf[0], b = buf[buf.length - 1];
    for (let i = buf.length - 1; i > 0; i--) if (buf[i - 1].at <= t) { a = buf[i - 1]; b = buf[i]; break; }
    const k = b.at === a.at ? 1 : clamp((t - a.at) / (b.at - a.at), 0, 1);
    if (!view || view.players.length !== b.state.players.length) view = structuredClone(b.state);
    Object.assign(view, b.state, { players: view.players });
    b.state.players.forEach((pb, i) => {
      const pa = a.state.players[i] ?? pb, out = view!.players[i];
      Object.assign(out, pb);
      if (Math.hypot(pb.x - pa.x, pb.z - pa.z) < 15) { out.x = lerp(pa.x, pb.x, k); out.y = lerp(pa.y, pb.y, k); out.z = lerp(pa.z, pb.z, k); out.yaw = pa.yaw + wrapAngle(pb.yaw - pa.yaw) * k; }
    });
    return view;
  }

  function start() {
    stop?.();
    const abort = new AbortController();
    let disposed = false, frame = 0, last = performance.now(), lastHud = 0, lastSend = 0, lastEvent = 0, bootStarted = false, finished = false, lastLootRev = -1, lastBuildRev = -1, preferBuilds = false, sentAt = 0;
    const hook = testHook();
    let state: ShooterState | null = setup ? createMatch(hook?.stormScale ? { ...setup, stormScale: hook.stormScale } : setup) : null;
    const latch = { jump: false, interact: false, reload: false, fire: false, edit: false, crouch: false, lethal: false, tactical: false, streak: false, slot: 0 };
    /** The camera as drawn: the player's aim plus the recoil kick (reused every frame). */
    const view = { yaw: 0, pitch: 0, aiming: false, zoom: 70 };
    const bootAt = performance.now();
    let pending: ShooterEvent[] = [];
    /** Context-loss remounts left (lib/three/stage.ts: twice a minute, then Retry). */
    const losses = lossBudget();
    loading = true; error = ''; menu = false; feed = []; spectate = -1; building = false; showMap = false; recap = null;
    dealt.clear(); taken.clear(); lastHeadFrom = -1;
    const fail = (e: unknown) => { if (!disposed) { loading = false; error = e instanceof Error ? e.message : 'The match couldn’t start.'; } };
    const boot = async (first: ShooterState) => {
      const { ShooterRenderer } = await import('./render');
      if (disposed) return;
      const me = selfOf(first);
      cam.yaw = first.players[me]?.yaw ?? 0;
      const created = new ShooterRenderer(stage, first.map, first.players.map(p => ({ fighter: p.fighter, human: p.human, name: p.name, team: p.team })), me, abort.signal);
      try { await created.load(); } catch (e) { created.dispose(); throw e; }
      if (disposed) { created.dispose(); return; }
      renderer = created;
      input = new InputState(created.view.canvas, { pointerLock: true, onLockChange: v => { locked = v; } });
      created.view.canvas.addEventListener('focus', () => { focused = true; });
      created.view.canvas.addEventListener('blur', () => { focused = false; });
      // Esc first releases the pointer lock (the browser's own); the next Esc opens the menu.
      created.view.canvas.addEventListener('keydown', e => { if (e.key === 'Escape' && document.pointerLockElement !== created.view.canvas && !menu && !finished) { e.preventDefault(); menu = true; } });
      created.view.canvas.focus();
      // A reclaimed GPU context remounts the renderer on the live match state
      // instead of leaving a black canvas; a third loss within a minute asks for Retry.
      created.view.onContextLost(() => {
        if (disposed || renderer !== created) return;
        input?.dispose(); input = undefined;
        renderer = undefined; created.dispose();
        if (!losses.allow()) { fail(new Error('The match lost its graphics context (too many 3D views open at once).')); return; }
        loading = true;
        setTimeout(() => { const s = state; if (!disposed && s) boot(s).catch(fail); }, 600);
      });
      (stage as HTMLDivElement & { __ottoGame?: unknown }).__ottoGame = {
        snapshot: () => structuredClone(state ?? view), stats: () => created.stats(),
        look: (yaw: number, pitch: number) => { cam.yaw = yaw; cam.pitch = pitch; },
      };
      loading = false;
    };
    const tick = (now: number) => {
      if (disposed) return;
      frame = requestAnimationFrame(tick);
      const dt = Math.min(0.1, Math.max(0, (now - last) / 1000));
      last = now;
      if (isGuest) {
        const v = interpolated(now);
        if (v && !bootStarted) { bootStarted = true; boot(v).catch(fail); }
        state = v;
      }
      if (!state) return;
      const me = selfOf(state);
      const mine = readInput(state, me);
      if (!isGuest) {
        const inputs = state.players.map(p => (!p.human ? undefined : p.slot === selfSlot ? mine : link?.inputs?.get(p.slot)?.input));
        if (state.phase === 'bus') for (const p of state.players) if (p.human && p.slot !== selfSlot) { const f = link?.inputs?.get(p.slot)?.fighter; if (f && p.mode === 'bus') p.fighter = f; }
        // Hold the drop until every guest has reported in (their survivor pick
        // arrives with their first input), for at most twelve seconds.
        const waiting = state.phase === 'bus' && state.bus.t === 0 && now - bootAt < 12_000 && state.players.some(p => p.human && p.slot !== selfSlot && !link?.inputs?.has(p.slot));
        if (!paused && !(menu && !link) && !waiting) stepMatch(state, inputs, dt);
        pending.push(...state.events.filter(e => e.id > lastEvent));
        if (onframe && now - sentAt > 50) {
          // Loot and structures ride when they change (or every two seconds, offset by one),
          // never in the same packet: when both are due they take turns.
          const lootDue = state.lootRev !== lastLootRev || Math.floor(now / 2000) !== Math.floor(sentAt / 2000);
          const buildDue = state.buildRev !== lastBuildRev || Math.floor((now + 1000) / 2000) !== Math.floor((sentAt + 1000) / 2000);
          const withBuilds = buildDue && (!lootDue || preferBuilds);
          const withLoot = lootDue && !withBuilds;
          onframe(state, pending, withLoot, withBuilds);
          if (withLoot) { lastLootRev = state.lootRev; preferBuilds = true; }
          if (withBuilds) { lastBuildRev = state.buildRev; preferBuilds = false; }
          pending = []; sentAt = now;
        }
      } else {
        // Latch one-shot buttons so a tap between two sends still reaches the host.
        latch.jump ||= mine.jump; latch.interact ||= mine.interact; latch.reload ||= mine.reload; latch.fire ||= mine.fire; latch.edit ||= mine.edit;
        latch.crouch ||= mine.crouch; latch.lethal ||= mine.lethal; latch.tactical ||= mine.tactical; latch.streak ||= mine.streak;
        if (mine.slot) latch.slot = mine.slot;
        if (now - lastSend > 33) {
          oninput?.({ ...mine, jump: latch.jump, interact: latch.interact, reload: latch.reload, fire: latch.fire, edit: latch.edit, crouch: latch.crouch, lethal: latch.lethal, tactical: latch.tactical, streak: latch.streak, slot: latch.slot || mine.slot }, fighter);
          latch.jump = latch.interact = latch.reload = latch.fire = latch.edit = latch.crouch = latch.lethal = latch.tactical = latch.streak = false; latch.slot = 0;
          lastSend = now;
        }
      }
      for (const e of state.events) if (e.id > lastEvent) { onEvent(state, e, me, now); lastEvent = e.id; }
      const p = state.players[me];
      if (p && p.mode !== 'dead') spectate = -1;
      let watch = me;
      if (p?.mode === 'dead') {
        building = false;
        if (spectate < 0 || state.players[spectate]?.mode === 'dead') {
          const mate = state.squad > 1 ? state.players.findIndex(q => q.mode !== 'dead' && q.team === p.team) : -1;
          spectate = mate >= 0 ? mate : state.players.findIndex(q => q.mode !== 'dead');
        }
        watch = spectate >= 0 ? spectate : me;
        if (watch !== me) { const w = state.players[watch]; cam.yaw = cam.yaw + wrapAngle(w.yaw - cam.yaw) * Math.min(1, dt * 3); }
      }
      if (renderer && p) {
        // The ghost follows this player's own intent at once; guests place it from the newest snapshot, extrapolated.
        local.building = building && p.mode === 'ground'; local.piece = piece; local.mat = material; local.yaw = cam.yaw; local.pitch = cam.pitch;
        const fresh = isGuest ? link?.snapshots?.at(-1) : undefined, src = fresh?.state.players[me] ?? p;
        const age = fresh ? clamp((now - fresh.at) / 1000, 0, 0.15) : 0;
        local.x = src.x + src.vx * age; local.y = src.y; local.z = src.z + src.vz * age;
      }
      // The drawn view climbs with this player's recoil kick (the host adds it to every shot).
      const kicked = p && watch === me && p.mode === 'ground' && !building;
      view.yaw = cam.yaw + (kicked ? p.kickYaw : 0); view.pitch = cam.pitch + (kicked ? p.kick : 0); view.aiming = cam.aiming; view.zoom = cam.zoom;
      if (renderer) renderer.frame(state, me, view, dt, watch, p ? local : undefined);
      input?.endFrame();
      drawOverlay(now);
      if (now - lastHud > 80) {
        hud = {
          ...state, storm: { ...state.storm }, bus: { ...state.bus }, chests: [...state.chests], players: state.players.map(q => ({ ...q, weapons: q.weapons.map(w => w && { ...w }), ammo: { ...q.ammo }, mats: [...q.mats] as Fighter['mats'] })),
          // The mode state is mutated in place: copy it so the HUD sees each change.
          obj: { ...state.obj, score: [...state.obj.score] as [number, number], points: state.obj.points.map(pt => ({ ...pt })) }, uav: [...state.uav],
        };
        ghostOk = renderer?.ghostValid ?? false;
        if (p && p.mode === 'ground') {
          const mate = nearestDowned(state, p, REVIVE_RANGE);
          const chest = nearestClosedChest(state, p);
          const drop = nearestDrop(state, p);
          const w = nearestWeaponLoot(state, p);
          const wall = nearestWall(state, p), bw = wall >= 0 ? buyWallsFor(mapFor(state.map))[wall] : null;
          const owns = !!bw && p.weapons.some(h => h?.w === bw.w);
          prompt = building ? '' : mate >= 0 ? (p.interact > 0 ? `Reviving ${state.players[mate].name}… ${Math.round((p.interact / REVIVE_TIME) * 100)}%` : `Hold E — revive ${state.players[mate].name}`)
            : bw ? `E — ${owns ? `${WEAPONS[bw.w].name} ammo` : `buy ${WEAPONS[bw.w].name}`} · ${owns ? Math.round(bw.price / 2) : bw.price} points${p.points < (owns ? bw.price / 2 : bw.price) ? ' (not enough)' : ''}`
            : chest >= 0 ? (p.interact > 0 ? 'Opening…' : 'Hold E — open chest')
              : drop >= 0 ? (p.interact > 0 ? 'Opening supply drop…' : 'Hold E — open supply drop')
                : w?.weapon ? `E — pick up ${RARITY_NAMES[w.rarity]} ${itemName(w.weapon)}${isConsumable(w.weapon) ? ` ×${w.amount}` : ''}` : p.swimming ? 'Swimming — head for the shore' : '';
        } else prompt = p?.mode === 'bus' ? 'Press Space to drop' : '';
        drawMap(minimap, state, me, false, cam.yaw, mapCache);
        if (showMap) drawMap(bigmap, state, me, true, cam.yaw, mapCache);
        lastHud = now;
      }
      if (state.phase === 'done' && !finished) { finished = true; input?.release(); onfinish?.(state); }
    };
    if (state) { bootStarted = true; boot(state).catch(fail); }
    frame = requestAnimationFrame(tick);
    stop = () => {
      disposed = true; abort.abort(); cancelAnimationFrame(frame);
      input?.dispose(); input = undefined;
      renderer?.dispose(); renderer = undefined;
      mapCache.image = null;
      delete (stage as HTMLDivElement & { __ottoGame?: unknown }).__ottoGame;
    };
  }

  onMount(() => {
    const change = () => { fullscreen = document.fullscreenElement === root; };
    document.addEventListener('fullscreenchange', change);
    start();
    return () => { document.removeEventListener('fullscreenchange', change); stop?.(); audio.dispose(); };
  });
  $effect(() => { if (paused || menu) input?.release(); });
  function resume() { menu = false; renderer?.view.canvas.focus(); }
  async function toggleFullscreen() { try { if (document.fullscreenElement) await document.exitFullscreen(); else await root.requestFullscreen(); } catch { /* embedding may block fullscreen */ } }
  function wake() { void audio.start().catch(() => {}); }
  function touchKey(e: PointerEvent, code: string, down: boolean) { e.preventDefault(); if (down) { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); wake(); } input?.touch(code, down); }
  let lookFrom: { x: number; y: number } | null = null;
  function look(e: PointerEvent) { if (!lookFrom) return; input?.look((e.clientX - lookFrom.x) * 1.6, (e.clientY - lookFrom.y) * 1.6); lookFrom = { x: e.clientX, y: e.clientY }; }

  /** Hotbar labels: a weapon shows mag / reserve, a consumable its count. */
  const slotAmmo = (w: Held) => (isWeapon(w.w) ? `${w.mag}` : `×${w.mag}`);
  const slotReserve = (p: Fighter, w: Held) => (isWeapon(w.w) ? `/${p.ammo[WEAPONS[w.w].ammo]}` : '');
  const consumableKind = (w: Held) => (isConsumable(w.w) ? (CONSUMABLES[w.w].pad ? 'pad' : CONSUMABLES[w.w].heal ? 'heal' : 'shield') : '');

  const me = $derived(hud ? hud.players[selfOf(hud)] : null);
  const held = $derived(me ? heldItem(me) : null);
  const heldWeapon = $derived(held && isWeapon(held.w) ? WEAPONS[held.w] : null);
  const heldUse = $derived(held && isConsumable(held.w) ? CONSUMABLES[held.w] : null);
  const alive = $derived(hud ? aliveCount(hud) : 0);
  const squad = $derived(hud?.squad ?? 1);
  const teams = $derived(hud && squad > 1 ? teamsAlive(hud) : 0);
  const mode = $derived(hud?.mode ?? 'royale');
  const arena = $derived(respawns(mode));
  const obj = $derived(hud?.obj);
  // The team panel lists duo / squad / Zombies teammates (the arena teams are too big for it).
  const mates = $derived(hud && me && squad > 1 && !arena ? hud.players.filter(q => q.team === me.team && q.id !== me.id) : []);
  const modeName = $derived(arena || mode === 'zombies' ? modeTitle(mode) : TEAM_MODES.find(m => m.size === squad)?.name ?? 'Solo');
  const heldStats = $derived(heldWeapon && held ? { ...weaponStats(heldWeapon, held.att) } : null);
  /** Our score and theirs (arena modes). */
  const ours = $derived(obj && me ? obj.score[me.team] ?? 0 : 0);
  const theirs = $derived(obj && me ? obj.score[1 - me.team] ?? 0 : 0);
  const infectedLeft = $derived(hud ? hud.infected.length + Math.max(0, hud.obj.left) : 0);
  const uavUp = $derived(!!me && (hud?.uav[me.team] ?? 0) > 0);
  const mapLabel = $derived(hud ? SHOOTER_MAPS.find(m => m.id === hud!.map)?.name ?? hud.map : '');
  const attachNames = (att: number) => ATTACHMENTS.filter(a => att & a.bit).map(a => a.name).join(', ');
  const storm = $derived(hud?.storm);
  const outside = $derived(!!(me && storm && me.mode !== 'dead' && Math.hypot(me.x - storm.cx, me.z - storm.cz) > storm.r));
  const results = $derived(hud && hud.phase === 'done' ? shooterResults(hud) : []);
  const mm = (s: number) => `${Math.floor(Math.max(0, s) / 60)}:${String(Math.floor(Math.max(0, s) % 60)).padStart(2, '0')}`;
  const watched = $derived(hud && spectate >= 0 ? hud.players[spectate] : null);
  /** The one always-mounted screen-reader status: edges only (round start, helicopter,
   *  respawning, the result headline). Ticking timers stay visual. */
  let srStatus = $state('');
  const objectiveEdge = $derived(obj ? `${obj.wave}:${obj.round}` : '');
  $effect(() => {
    if (!objectiveEdge) return;
    untrack(() => {
      if (!obj) return;
      const say = obj.wave === WAVE_PRE ? `Round ${obj.round + 1} starts in ${Math.ceil(obj.timer)} seconds` : obj.wave === WAVE_HELI ? 'Helicopter inbound' : obj.wave === WAVE_BOARD ? 'Board the helicopter' : obj.wave === WAVE_ON && obj.round >= obj.lastRound ? 'Final round' : '';
      if (say) srStatus = say;
    });
  });
  const respawning = $derived(!!(arena && me && me.mode === 'dead' && hud?.phase !== 'done'));
  $effect(() => { if (respawning) srStatus = 'Respawning. Pick a class with 6 to 9.'; });
  const knocked = $derived(me?.mode === 'down');
  $effect(() => { if (knocked) srStatus = 'Knocked. Crawl to a teammate; they can revive you.'; });
  $effect(() => { if (loading && !error) srStatus = isGuest && !hud ? 'Waiting for the host…' : 'Preparing the island…'; });
  $effect(() => { if (paused && hud && hud.phase !== 'done') srStatus = 'Match paused while someone reconnects.'; });
  const headline = $derived.by(() => {
    if (!hud || hud.phase !== 'done' || !me) return '';
    if (arena && obj) return hud.winTeam < 0 ? 'Draw' : hud.winTeam === me.team ? 'Victory!' : 'Defeat';
    if (mode === 'zombies' && obj) return obj.aboard > 0 ? (me.place === 1 ? 'Extracted!' : 'Left behind') : `Overrun on round ${obj.round}`;
    return me.place === 1 ? 'Victory Royale!' : squad > 1 ? `Your team placed ${ordinal(me.place)}` : `You placed ${ordinal(me.place)}`;
  });
  $effect(() => { if (headline) srStatus = headline; });
  const buildHint = $derived(!me ? '' : me.mats[material] < BUILD_COST ? `Need ${BUILD_COST} ${MATERIAL_NAMES[material].toLowerCase()} — harvest with the pickaxe (F)` : ghostOk ? 'Click to place' : 'Nothing to build on here');
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {              "class":`br-play game-play`,...__sveltets_2_empty({"data-game":`shooter`}),...__sveltets_2_empty({"data-map":setup?.map ?? hud?.map}),...__sveltets_2_empty({"data-phase":hud?.phase ?? 'loading'}),...__sveltets_2_empty({"data-building":building ? 'on' : 'off'}),"onpointerdown":wake,"role":`presentation`,});root = $$_div0;
   { const $$_div1 = svelteHTML.createElement("div", {  "class":`br-stage`,});stage = $$_div1; }
   { const $$_canvas1 = svelteHTML.createElement("canvas", {    "class":`br-overlay`,"aria-hidden":`true`,});overlay = $$_canvas1; }
   { svelteHTML.createElement("p", {   "class":`sr-only`,"role":`status`,});srStatus; }
  if(loading && !error){ { svelteHTML.createElement("div", { "class":`br-cover`,}); { svelteHTML.createElement("h2", {});isGuest && !hud ? 'Waiting for the host…' : 'Preparing the island…'; } { svelteHTML.createElement("p", {});     } }}
  if(error){ { svelteHTML.createElement("div", { "class":`br-cover`,}); { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`match`,error,"empty":true,"onretry":start,}});} { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});   } }}
  if(hud && me && !loading && !error){
    if(outside){ { svelteHTML.createElement("div", {   "class":`br-storm-tint`,"aria-hidden":`true`,}); }}
    if(aiming){ { svelteHTML.createElement("div", {   "class":`br-scope`,"aria-hidden":`true`,}); { svelteHTML.createElement("i", {}); } }}
    if(me.blind > 0){ { svelteHTML.createElement("div", {    "class":`br-flash`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, Math.min(1, me.blind / 1.2)); }}
    if(me.hurtT > 0){ { svelteHTML.createElement("div", {    "class":`br-hurt`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, Math.min(0.7, me.hurtT * 1.6)); }}
    if(hurtDir !== null){ { svelteHTML.createElement("div", {    "class":`br-hurt-dir`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, `translate(-50%, -50%) rotate(${-hurtDir}rad)`); { svelteHTML.createElement("i", {}); } }}
     { svelteHTML.createElement("div", { "class":`br-top`,});
       { svelteHTML.createElement("div", {   "class":`br-stats`,"aria-label":`Match status`,});
        if(arena && obj){
           { svelteHTML.createElement("span", {     "class":`br-score`,"title":`First to ${obj.limit}`,});__sveltets_2_ensureType(String, Number, hex(teamColour(me.team)));__sveltets_2_ensureType(String, Number, hex(teamColour(1 - me.team))); { svelteHTML.createElement("b", { "class":`us`,});ours; } { svelteHTML.createElement("small", {});  } { svelteHTML.createElement("b", { "class":`them`,});theirs; } { svelteHTML.createElement("small", {}); obj.limit; } }
           { svelteHTML.createElement("span", { "title":`Time left`,}); { svelteHTML.createElement("b", {});mm(obj.time); } }
           { svelteHTML.createElement("span", { "title":`Your eliminations and deaths`,}); { svelteHTML.createElement("b", {});me.kills; } me.deaths;  { svelteHTML.createElement("small", {});  } }
        } else if (mode === 'zombies' && obj){
           { svelteHTML.createElement("span", { "title":`Round`,}); { svelteHTML.createElement("small", {});  }  { svelteHTML.createElement("b", {});obj.round || 1; }if(obj.lastRound){ { svelteHTML.createElement("small", {});  obj.lastRound; }} }
           { svelteHTML.createElement("span", { "title":`Infected left this round`,}); { svelteHTML.createElement("b", {});infectedLeft; }  }
           { svelteHTML.createElement("span", {   "class":`br-points`,"title":`Points: buy from walls (E), refill ammo`,}); { svelteHTML.createElement("b", {});me.points.toLocaleString(); }  }
           { svelteHTML.createElement("span", { "title":`Infected killed`,}); { svelteHTML.createElement("b", {});me.kills; }  }
        }else{
           { svelteHTML.createElement("span", { "title":`Survivors left`,}); { svelteHTML.createElement("b", {});alive; }  }
          if(squad > 1){ { svelteHTML.createElement("span", { "title":`${modeName}: teams still in the match`,}); { svelteHTML.createElement("b", {});teams; } teams === 1 ? 'team' : 'teams'; }}
           { svelteHTML.createElement("span", { "title":`Your eliminations`,}); { svelteHTML.createElement("b", {});me.kills; } me.kills === 1 ? 'elim' : 'elims'; }
          if(storm && hud.phase === 'play'){ { svelteHTML.createElement("span", {  "class":`br-storm`,});storm.shrinking;mode === 'apoc' ? (storm.stage < 0 ? 'Toxic cloud forms in' : storm.shrinking ? 'Toxic cloud closing' : 'Toxic cloud moves in') : storm.stage < 0 ? 'Storm forms in' : storm.shrinking ? 'Storm shrinking' : storm.stage >= STORM.length - 1 ? 'Final circle' : 'Storm moves in';  { svelteHTML.createElement("b", {});mm(storm.timer); }if(storm.stage >= 0){ { svelteHTML.createElement("small", {});   Math.min(STORM.length, storm.stage + 1); STORM.length; }} }}
          if(hud.phase === 'bus'){ { svelteHTML.createElement("span", {});    { svelteHTML.createElement("b", {});mm(hud.bus.duration - hud.bus.t); } }}
        }
       }
       { svelteHTML.createElement("div", { "class":`br-tools`,});
         { svelteHTML.createElement("button", {       "class":`btn small`,"aria-label":muted ? 'Unmute game' : 'Mute game',"title":muted ? 'Unmute game' : 'Mute game',"onclick":() => { muted = !muted; audio.mute(muted); },});muted ? 'Sound off' : 'Sound on'; }
         { svelteHTML.createElement("button", {     "class":`btn small`,"aria-pressed":showMap,"onclick":() => { showMap = !showMap; },});  }
        if(typeof document !== 'undefined' && document.fullscreenEnabled){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":toggleFullscreen,});fullscreen ? 'Exit full screen' : 'Full screen'; }}
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { menu = !menu; },});  }
       }
     }
    if(obj && mode === 'dom' && obj.points.length){
       { svelteHTML.createElement("ol", {   "class":`br-objective`,"aria-label":`Capture points`,});
            for(let pt of __sveltets_2_ensureArray(obj.points)){let i = 1;i;
           { svelteHTML.createElement("li", {      "title":`${POINT_NAMES[i]}: ${pt.own === me.team ? 'held by your team' : pt.own >= 0 ? 'held by the enemy' : 'neutral'}`,});pt.own === me.team;pt.own >= 0 && pt.own !== me.team;__sveltets_2_ensureType(String, Number, pt.own >= 0 ? hex(teamColour(pt.own)) : 'transparent');__sveltets_2_ensureType(String, Number, pt.by >= 0 ? hex(teamColour(pt.by)) : 'transparent');
             { svelteHTML.createElement("b", {});POINT_NAMES[i]; }if(pt.own < 0 && pt.cap > 0){ { svelteHTML.createElement("i", { });__sveltets_2_ensureType(String, Number, `${pt.cap * 100}%`); }}
           }
        }
       }
    } else if (obj && mode === 'zombies' && hud.phase === 'play'){
       { svelteHTML.createElement("p", {   "class":`br-objective-note`,"aria-hidden":`true`,});obj.wave === WAVE_PRE ? `Round ${obj.round + 1} in ${Math.ceil(obj.timer)}s — barricade (Q) and repair with the pickaxe (F)` : obj.wave === WAVE_HELI ? `Helicopter inbound ${mm(obj.timer)} — hold out, then get to the landing zone (M)` : obj.wave === WAVE_BOARD ? `Board the helicopter! ${mm(obj.timer)}` : obj.wave === WAVE_ON && obj.round >= obj.lastRound ? 'Final round — the helicopter comes after this one' : ''; }
    }
     { svelteHTML.createElement("div", { "class":`br-mapbox`,}); { const $$_canvas2 = svelteHTML.createElement("canvas", {        "class":`br-minimap`,"width":`200`,"height":`200`,"aria-label":`Island map`,});minimap = $$_canvas2; }
       { svelteHTML.createElement("ol", {   "class":`br-feed`,"aria-live":`polite`,});   for(let f of __sveltets_2_ensureArray(feed)){f.id; { svelteHTML.createElement("li", { });f.mine;f.text; }} }
     }
    if(mates.length){
       { const $$_lenaPmaeT1C = __sveltets_2_ensureComponent(TeamPanel); new $$_lenaPmaeT1C({ target: __sveltets_2_any(), props: {    mates,"team":me.team,modeName,}});}
    }
    if(showMap && hud.phase !== 'done'){
       { svelteHTML.createElement("div", { "class":`br-bigmap`,}); { const $$_canvas2 = svelteHTML.createElement("canvas", {       "width":`640`,"height":`640`,"aria-label":`Full island map`,});bigmap = $$_canvas2; } { svelteHTML.createElement("p", {});mapFor(hud.map).pois.length;               } }
    }
    if(me.mode === 'ground' && !aiming){
      
       { svelteHTML.createElement("div", {       "class":`br-cross`,"aria-hidden":`true`,});performance.now() - hitMark < 160;building;me.ads > 0.6 && !building;__sveltets_2_ensureType(String, Number, `${8 + (heldWeapon && heldStats && !building ? (heldWeapon.spread + (heldStats.adsSpread - heldWeapon.spread) * me.ads) * (me.crouch ? 0.75 : 1) * 260 : 0)}px`); { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("b", {}); } }
    }
    if(performance.now() - hitMark < 220){ { svelteHTML.createElement("div", {    "class":`br-hitmark`,"aria-hidden":`true`,});hitKill; { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } }}
    if(prompt){ { svelteHTML.createElement("div", { "class":`br-prompt`,});prompt; }}
    if(toast){ { svelteHTML.createElement("div", { "class":`br-toast`,});toast; }}
    if(me.reload > 0 && heldWeapon){ { svelteHTML.createElement("div", {   "class":`br-reload`,"aria-label":`Reloading`,}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${(1 - me.reload / heldWeapon.reload) * 100}%`); } { svelteHTML.createElement("em", {});  } }}
    if(me.useT > 0 && heldUse){ { svelteHTML.createElement("div", {   "class":`br-reload use`,"aria-label":`Using ${heldUse.name}`,}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${Math.min(1, me.useT / heldUse.time) * 100}%`); } { svelteHTML.createElement("em", {}); heldUse.name; } }}
    if(me.mode === 'down'){ { const $$_dekconK1C = __sveltets_2_ensureComponent(Knocked); new $$_dekconK1C({ target: __sveltets_2_any(), props: { me,}});}}
    if(me.mode !== 'dead'){
       { const $$_slatiV1C = __sveltets_2_ensureComponent(Vitals); new $$_slatiV1C({ target: __sveltets_2_any(), props: {     me,mode,uavUp,building,material,}});}
      if(building){
         { svelteHTML.createElement("div", {   "class":`br-buildbar`,"aria-label":`Build mode`,});
              for(let name of __sveltets_2_ensureArray(BUILD_KIND_NAMES)){let k = 1;name; { svelteHTML.createElement("button", {      "class":`btn small`,"aria-pressed":k === piece,"onclick":() => { piece = k; renderer?.view.canvas.focus(); },});k === piece; { svelteHTML.createElement("small", {});k + 1; }name; }}
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => { material = (material + 1) % 3; renderer?.view.canvas.focus(); },"title":`Change material (R)`,});MATERIAL_NAMES[material];  BUILD_COST; }
           { svelteHTML.createElement("p", { });!ghostOk;buildHint;         }
         }
      }
       { svelteHTML.createElement("div", {   "class":`br-inventory`,"aria-label":`Hotbar`,});
         { svelteHTML.createElement("div", {    "class":`br-slot br-tool`,"title":`Pickaxe (F)`,});me.current === PICKAXE_SLOT && !building; { svelteHTML.createElement("small", {});  } { svelteHTML.createElement("svg", {   "viewBox":`0 0 24 24`,"aria-hidden":`true`,}); { svelteHTML.createElement("path", {  "d":`M4 20 14 10M9 5c3-2 8-2 11 1-3-1-6-1-8 1zm3 2 5 5`,});} } { svelteHTML.createElement("span", { "class":`br-empty`,});  } }
            for(let w of __sveltets_2_ensureArray(me.weapons)){let i = 1;i;
           { svelteHTML.createElement("div", {   "class":`br-slot`,});i === me.current && !building;__sveltets_2_ensureType(String, Number, w ? hex(RARITY_COLOURS[w.rarity]) : undefined);
             { svelteHTML.createElement("small", {});i + 1; }
            if(w && isWeapon(w.w)){ { svelteHTML.createElement("img", {    "src":`/room-games/portraits/${GUN_ICON[w.w]}.webp`,"alt":itemName(w.w),});} { svelteHTML.createElement("span", {});slotAmmo(w); { svelteHTML.createElement("em", {});slotReserve(me, w); } }
            } else if (w){ { svelteHTML.createElement("svg", {       "class":`br-item ${consumableKind(w)}`,"viewBox":`0 0 24 24`,"role":`img`,"aria-label":itemName(w.w),});if(consumableKind(w) === 'heal'){ { svelteHTML.createElement("path", {  "d":`M10 3h4v7h7v4h-7v7h-4v-7H3v-4h7z`,});}} else if (consumableKind(w) === 'pad'){ { svelteHTML.createElement("path", {  "d":`M4 18h16v3H4zM7 15h10l-2-3h3l-6-9-6 9h3z`,});}}else{ { svelteHTML.createElement("path", {  "d":`M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5z`,});}} } { svelteHTML.createElement("span", {});slotAmmo(w); }
            }else{ { svelteHTML.createElement("span", { "class":`br-empty`,});  }}
           }
        }
        if(building){ { svelteHTML.createElement("p", { "class":`br-held`,});  BUILD_KIND_NAMES[piece]; }
        } else if (held){ { svelteHTML.createElement("p", { "class":`br-held`,});RARITY_NAMES[held.rarity]; itemName(held.w);if(held.att){ attachNames(held.att);}if(heldUse?.pad){         } else if (heldUse && !canUse(heldUse.id, me.hp, me.shield)){ heldUse.heal ? 'health' : 'shield'; } }
        } else if (me.current === PICKAXE_SLOT){ { svelteHTML.createElement("p", { "class":`br-held`,});          }}
         { svelteHTML.createElement("p", {   "class":`br-ammo`,"aria-label":`Ammo reserve`,});   for(let k of __sveltets_2_ensureArray(AMMO_KINDS)){k; { svelteHTML.createElement("span", { });me.ammo[k] > 0;k;  { svelteHTML.createElement("b", {});me.ammo[k]; } }} }
       }
    } else if (hud.phase !== 'done'){
      if(recap){
         { svelteHTML.createElement("section", {   "class":`br-recap`,"aria-label":`Death recap`,});
           { svelteHTML.createElement("h3", {});recap.killer; }
           { svelteHTML.createElement("p", {});recap.how;recap.distance ? ` · ${recap.distance} m` : '';recap.head ? ' · headshot' : '';recap.left ? ` · they had ${recap.left} health left` : ''; }
          if(recap.dealt || recap.taken){ { svelteHTML.createElement("p", { "class":`br-recap-dmg`,}); { svelteHTML.createElement("span", {});   { svelteHTML.createElement("b", {});recap.dealt; } } { svelteHTML.createElement("span", {});   { svelteHTML.createElement("b", {});recap.taken; } } }}
         }
      }
      if(arena){
         { svelteHTML.createElement("div", { "class":`br-respawn`,});
           { svelteHTML.createElement("p", {}); { svelteHTML.createElement("span", { "aria-hidden":`true`,});me.respawn > 0 ? `Respawning in ${Math.ceil(me.respawn)}…` : 'Respawning…'; }  { svelteHTML.createElement("small", {});    } }
           { svelteHTML.createElement("div", {     "class":`br-classes`,"role":`group`,"aria-label":`Class`,});
                for(let c of __sveltets_2_ensureArray(CLASSES)){let k = 1;c.id; { svelteHTML.createElement("button", {       "class":`btn small`,"aria-pressed":chosenClass === k,"title":c.blurb,"onclick":() => { chosenClass = k; renderer?.view.canvas.focus(); },}); { svelteHTML.createElement("small", {});k + 6; }c.name; }}
           }
           { svelteHTML.createElement("p", { "class":`br-class-note`,});CLASSES[chosenClass].blurb; }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`br-spectate`,}); { svelteHTML.createElement("p", {});mode === 'zombies' ? 'Down for this round — you are back when the next one starts · ' : me.place ? `#${me.place} · ` : squad > 1 ? 'Your team is still in it · ' : '';me.kills; mode === 'zombies' ? 'kills' : 'elims';  watched ? `Spectating ${watched.name || fighterName(watched.fighter)} (${watched.kills} ${mode === 'zombies' ? 'kills' : 'elims'})` : 'Match in progress'; }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { if (!hud) return; const list = hud.players.filter(q => q.mode !== 'dead'); const idx = list.findIndex(q => q.id === spectate); spectate = list[(idx + 1) % list.length]?.id ?? -1; },});  }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onexit,});  } }
      }
    }
    if(!focused && !locked && hud.phase !== 'done' && !menu && !paused){ { svelteHTML.createElement("div", { "class":`br-hint`,});                                                                          mode === 'zombies' ? ' / buy' : '';       }}
    if(paused && hud.phase !== 'done'){ { svelteHTML.createElement("div", { "class":`br-cover`,}); { svelteHTML.createElement("h2", {});   } { svelteHTML.createElement("p", {});       } { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});  } }
    } else if (hud.phase === 'done'){
       { const $$_stluseRhctaM1C = __sveltets_2_ensureComponent(MatchResults); new $$_stluseRhctaM1C({ target: __sveltets_2_any(), props: {            hud,me,results,headline,mapLabel,modeName,ours,theirs,rematchLabel,rematchDisabled,onrematch,onexit,}});}
    } else if (menu){
       { const $$_uneMesuaP1C = __sveltets_2_ensureComponent(PauseMenu); new $$_uneMesuaP1C({ target: __sveltets_2_any(), props: {     "networked":!!link,"onresume":resume,onexit,}});}
    }
     { svelteHTML.createElement("div", {   "class":`br-touch`,"aria-label":`Touch controls`,});
       { svelteHTML.createElement("div", { "class":`br-touch-move`,});   for(let [k, label] of __sveltets_2_ensureArray(([['Up', '▲'], ['Left', '◀'], ['Down', '▼'], ['Right', '▶']]))){k; { svelteHTML.createElement("button", {       "aria-label":`Move ${k.toLowerCase()}`,"onpointerdown":e => touchKey(e, k, true),"onpointerup":e => touchKey(e, k, false),"onpointercancel":e => touchKey(e, k, false),});label; }} }
       { svelteHTML.createElement("div", {             "class":`br-look`,"role":`application`,"aria-label":`Drag to aim`,"onpointerdown":e => { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); lookFrom = { x: e.clientX, y: e.clientY }; },"onpointermove":look,"onpointerup":() => (lookFrom = null),"onpointercancel":() => (lookFrom = null),});  }
       { svelteHTML.createElement("div", { "class":`br-touch-act`,});   for(let [k, label] of __sveltets_2_ensureArray(([['Fire', building ? 'Place' : 'Fire'], ['Jump', 'Jump'], ['Use', 'Use'], ['Reload', building ? 'Material' : 'Reload'], ['Build', building ? 'Combat' : 'Build'], ['Pick', 'Pickaxe'], ['Edit', 'Edit'], ['Nade', 'Grenade'], ['Crouch', 'Crouch'], ['Streak', 'Streak']]))){k; { svelteHTML.createElement("button", {     "onpointerdown":e => touchKey(e, k, true),"onpointerup":e => touchKey(e, k, false),"onpointercancel":e => touchKey(e, k, false),});label; }} }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ShooterGame__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ShooterGame__SvelteComponent_ = ReturnType<typeof ShooterGame__SvelteComponent_>;
/*Ωignore_endΩ*/export default ShooterGame__SvelteComponent_;