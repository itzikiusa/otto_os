<script lang="ts">
  import { onMount } from 'svelte';
  import LoadState from '../../../../lib/components/LoadState.svelte';
  import {
    aliveCount, createMatch, heldItem, idleShooterInput, nearestClosedChest, nearestWeaponLoot, PICKAXE_KEY, PICKAXE_SLOT, shooterResults, SLOTS, stepMatch, STORM,
    type Fighter, type Held, type ShooterEvent, type ShooterInput, type ShooterLink, type ShooterSetup, type ShooterState,
  } from './sim';
  import { mapFor, HALF, waterLevel } from './map';
  import { AMMO_KINDS, BUILD_COST, CONSUMABLES, canUse, isConsumable, isWeapon, itemName, MATERIAL_COLOURS, MATERIAL_NAMES, RARITY_COLOURS, RARITY_NAMES, WEAPONS } from './weapons';
  import { BUILD_KIND_NAMES } from './builds';
  import { fighterName, type FighterId } from './catalog';
  import { InputState } from '../common/input';
  import { GameAudio } from '../common/audio';
  import { clamp, lerp, ordinal, wrapAngle } from '../common/math';
  import type { ShooterRenderer } from './render';

  interface Props {
    setup: ShooterSetup | null; selfSlot: number; fighter: FighterId; link?: ShooterLink;
    names?: (slot: number) => string; paused?: boolean;
    /** Host: one snapshot. Loot and structures are never both included (relay size cap). */
    onframe?: (state: ShooterState, events: ShooterEvent[], withLoot: boolean, withBuilds: boolean) => void;
    oninput?: (input: ShooterInput, fighter: FighterId) => void;
    onfinish?: (state: ShooterState) => void;
    onexit: () => void; onrematch: () => void; rematchLabel?: string; rematchDisabled?: boolean;
  }
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
  /** Build mode, the selected piece (wall / floor / ramp) and material; the full-screen map. */
  let building = $state(false), piece = $state(0), material = $state(0), showMap = $state(false), ghostOk = $state(false);
  const audio = new GameAudio(['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket', 'explosion', 'hit', 'shieldhit', 'headshot', 'reload', 'pickup', 'chest', 'storm', 'eliminated', 'land', 'voice-win', 'voice-over', 'voice-hurry', 'voice-go', 'select', 'step', 'shield', 'bump', 'tick', 'drop']);
  let renderer: ShooterRenderer | undefined, input: InputState | undefined;
  let stop: (() => void) | undefined;
  const isGuest = $derived(link?.role === 'guest');
  const cam = { yaw: 0, pitch: -0.3, aiming: false, zoom: 70 };
  let slotWanted = 0;
  const numbers: { x: number; y: number; z: number; text: string; color: string; t: number }[] = [];
  const GUN_ICON: Record<string, string> = { pistol: 'gun-pistol', smg: 'gun-smg', rifle: 'gun-rifle', shotgun: 'gun-shotgun', sniper: 'gun-sniper', rocket: 'gun-rocket' };
  const hex = (n: number) => `#${n.toString(16).padStart(6, '0')}`;
  const KILL_HOW = ['Sidearm', 'Buzz SMG', 'Assault Blaster', 'Pump Scatter', 'Longshot Rail', 'Boom Tube', 'the storm', 'Pickaxe'];

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
    } else {
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
    cam.zoom = cam.aiming ? def?.zoom ?? 55 : p.mode === 'sky' || p.mode === 'glide' ? 78 : 70;
    aiming = cam.aiming && def?.id === 'sniper';
    // Aim the muzzle at whatever is under the crosshair (the build ghost follows the camera).
    let yaw = cam.yaw, pitch = cam.pitch;
    if (renderer && p.mode === 'ground' && !building) {
      const t = renderer.aimPoint;
      const [ex, ey, ez] = renderer.eye(p);
      yaw = Math.atan2(t.x - ex, t.z - ez);
      pitch = Math.atan2(t.y - ey, Math.max(0.5, Math.hypot(t.x - ex, t.z - ez)));
      if (Math.abs(wrapAngle(yaw - cam.yaw)) > 0.6) { yaw = cam.yaw; pitch = cam.pitch; }
    }
    return {
      mx: (i.down('KeyD', 'ArrowRight', 'Right') ? 1 : 0) - (i.down('KeyA', 'ArrowLeft', 'Left') ? 1 : 0),
      mz: (i.down('KeyW', 'ArrowUp', 'Up') ? 1 : 0) - (i.down('KeyS', 'ArrowDown', 'Down') ? 1 : 0),
      yaw, pitch, fire: (i.mouse.left && locked) || i.down('Fire') || (i.mouse.left && focused && !locked && p.mode === 'ground'),
      aim, jump: i.down('Space', 'Jump') || i.hit('Space', 'Jump'), sprint: i.down('ShiftLeft', 'ShiftRight'),
      reload: !building && (i.down('KeyR', 'Reload') || i.hit('KeyR', 'Reload')), interact: i.down('KeyE', 'Use') || i.hit('KeyE', 'Use'), slot,
      build: building, piece, mat: material, edit,
    };
  }

  function say(text: string, ms = 1800) { toast = text; const shown = text; setTimeout(() => { if (toast === shown) toast = ''; }, ms); }
  function nameOf(state: ShooterState, id: number): string { if (id === -2) return 'the storm'; if (id < 0) return 'a fall'; return state.players[id]?.name ?? 'someone'; }
  const SHOT_SOUNDS = ['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket'];

  function onEvent(state: ShooterState, e: ShooterEvent, me: number, now: number) {
    renderer?.effect(e, state);
    const p = state.players[me];
    const dist = p ? Math.hypot(e.x - p.x, e.z - p.z) : 0;
    switch (e.type) {
      case 'shot': audio.play(SHOT_SOUNDS[e.value] ?? 'pistol', { distance: e.player === me ? 0 : dist, volume: e.player === me ? 0.55 : 0.8, throttle: 0.02 }); break;
      case 'hit':
        if (e.player === me) {
          audio.play(e.flag & 2 ? 'headshot' : e.flag & 1 ? 'shieldhit' : 'hit', { volume: 0.6, throttle: 0.03 });
          hitMark = now;
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
        feed = [...feed.slice(-4), { id: e.id, text: e.player >= 0 ? `${killer} ⟶ ${victim}${weapon ? ` · ${weapon}` : ''}` : `${victim} ${how}`, mine: e.player === me || e.target === me }];
        setTimeout(() => { feed = feed.filter(f => f.id !== e.id); }, 6000);
        if (e.player === me && e.target !== me) { audio.play('eliminated'); say(`You eliminated ${victim} · ${aliveCount(state)} left`); }
        if (e.target === me) { audio.play('voice-over'); say(e.player >= 0 ? `Eliminated by ${killer}` : `You ${how.replace('was ', 'were ')}`, 3000); spectate = e.player >= 0 ? e.player : -1; }
        break;
      }
      case 'pickup': if (e.player === me) audio.play('pickup', { volume: 0.5 }); break;
      case 'reload': if (e.player === me) audio.play('reload', { volume: 0.6 }); break;
      case 'chest': audio.play('chest', { distance: dist }); break;
      case 'explode': audio.play('explosion', { distance: dist, volume: 1.2 }); break;
      case 'land': if (e.player === me) audio.play('land'); break;
      case 'storm': audio.play('storm', { volume: 0.7 }); if (e.flag === 1) { audio.play('voice-hurry'); say(`Storm phase ${e.value + 1} of ${STORM.length} — the storm is closing in!`); } else say('Storm eye revealed — check the map (M)', 2200); break;
      case 'win': if (e.player === me) { audio.play('voice-win'); say('Victory Royale!', 4000); } break;
      case 'jump': if (e.player === me) audio.play('select'); break;
      case 'build': audio.play('drop', { distance: e.player === me ? 0 : dist, volume: 0.5, throttle: 0.05 }); break;
      case 'break': audio.play('bump', { distance: dist, volume: 0.8, throttle: 0.05 }); break;
      case 'harvest': if (e.player === me) audio.play('tick', { volume: 0.6, throttle: 0.05 }); break;
      case 'swing': if (e.value > 0) audio.play('bump', { distance: e.player === me ? 0 : dist, volume: 0.4, throttle: 0.05 }); break;
      case 'use': if (e.player === me) audio.play('shield', { volume: 0.6 }); break;
      case 'edit': if (e.player === me) audio.play('select', { volume: 0.5 }); break;
    }
  }

  let mapImage: HTMLCanvasElement | null = null;
  function islandImage(state: ShooterState): HTMLCanvasElement {
    if (mapImage) return mapImage;
    const map = mapFor(state.map);
    const c = document.createElement('canvas');
    c.width = c.height = map.size;
    const g = c.getContext('2d')!;
    const img = g.createImageData(map.size, map.size);
    const step = (HALF * 2) / (map.size - 1);
    for (let i = 0; i < map.size * map.size; i++) {
      const h = map.heights[i], x = -HALF + (i % map.size) * step, z = -HALF + Math.floor(i / map.size) * step;
      let r: number, gg: number, b: number;
      if (h < waterLevel(map, x, z)) { r = 46; gg = 128; b = 196; }
      else if (map.theme === 'isle') { if (h < 1.6) { r = 232; gg = 211; b = 154; } else { const k = Math.min(1, h / 18); r = 110 - k * 30; gg = 185 - k * 40; b = 80 - k * 20; } }
      else { const k = Math.min(1, h / 32); r = 215 - k * 40; gg = 150 - k * 50; b = 92 - k * 30; }
      img.data[i * 4] = r; img.data[i * 4 + 1] = gg; img.data[i * 4 + 2] = b; img.data[i * 4 + 3] = 255;
    }
    g.putImageData(img, 0, 0);
    g.fillStyle = 'rgba(60,60,70,0.85)';
    for (const b of map.boxes) { const s = map.size / (HALF * 2); g.fillRect((b.minX + HALF) * s, (b.minZ + HALF) * s, Math.max(1, (b.maxX - b.minX) * s), Math.max(1, (b.maxZ - b.minZ) * s)); }
    mapImage = c;
    return c;
  }
  /** Island map with the storm, flight path, POIs, structures and teammates; `big` is the full-screen view. */
  function drawMap(c: HTMLCanvasElement | undefined, state: ShooterState, me: number, big: boolean) {
    if (!c) return;
    const g = c.getContext('2d'); if (!g) return;
    const w = c.width, s = w / (HALF * 2);
    // Mirror X so the map reads like the third-person view (+X is to the left facing +Z).
    const mx = (x: number) => w - (x + HALF) * s, mz = (z: number) => (z + HALF) * s;
    g.save();
    g.clearRect(0, 0, w, w);
    g.translate(w, 0); g.scale(-1, 1); g.drawImage(islandImage(state), 0, 0, w, w); g.restore();
    g.fillStyle = 'rgba(255,170,60,0.9)';
    for (const b of state.builds) { const a = b.aabb; g.fillRect(mx(a.maxX), mz(a.minZ), Math.max(1.5, (a.maxX - a.minX) * s), Math.max(1.5, (a.maxZ - a.minZ) * s)); }
    const st = state.storm;
    g.fillStyle = 'rgba(120,60,200,0.35)';
    g.beginPath(); g.rect(0, 0, w, w); g.arc(mx(st.cx), mz(st.cz), Math.max(0, st.r) * s, 0, Math.PI * 2, true); g.fill();
    g.strokeStyle = '#ffffff'; g.lineWidth = big ? 3 : 2; g.setLineDash([5, 4]);
    g.beginPath(); g.arc(mx(st.toX), mz(st.toZ), Math.max(0, st.toR) * s, 0, Math.PI * 2); g.stroke(); g.setLineDash([]);
    if (state.phase === 'bus') { g.strokeStyle = 'rgba(255,255,255,0.8)'; g.lineWidth = 3; g.beginPath(); g.moveTo(mx(state.bus.ax), mz(state.bus.az)); g.lineTo(mx(state.bus.bx), mz(state.bus.bz)); g.stroke(); }
    g.font = big ? '700 15px system-ui, sans-serif' : '600 10px system-ui, sans-serif'; g.textAlign = 'center';
    for (const poi of mapFor(state.map).pois) {
      g.lineWidth = 3; g.strokeStyle = 'rgba(0,0,0,0.55)'; g.strokeText(poi.name, mx(poi.x), mz(poi.z));
      g.fillStyle = 'rgba(255,255,255,0.95)'; g.fillText(poi.name, mx(poi.x), mz(poi.z));
    }
    for (const p of state.players) {
      if (p.mode === 'dead' || p.id === me || !p.human) continue;
      g.fillStyle = '#ffd23f'; g.beginPath(); g.arc(mx(p.x), mz(p.z), big ? 6 : 4, 0, Math.PI * 2); g.fill();
    }
    const p = state.players[me];
    if (p && p.mode !== 'dead') {
      const k = big ? 1.6 : 1;
      g.save(); g.translate(mx(p.x), mz(p.z)); g.rotate(cam.yaw); g.scale(k, k);
      g.fillStyle = '#ffffff'; g.strokeStyle = '#000'; g.lineWidth = 1.5;
      g.beginPath(); g.moveTo(0, 7); g.lineTo(5, -5); g.lineTo(0, -2); g.lineTo(-5, -5); g.closePath(); g.fill(); g.stroke();
      g.restore();
      // Outside the next circle: a line to safety.
      if (Math.hypot(p.x - st.toX, p.z - st.toZ) > st.toR) { g.strokeStyle = 'rgba(255,255,255,0.7)'; g.lineWidth = 2; g.beginPath(); g.moveTo(mx(p.x), mz(p.z)); g.lineTo(mx(st.toX), mz(st.toZ)); g.stroke(); }
    }
  }
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
    let state: ShooterState | null = setup ? createMatch(setup) : null;
    const latch = { jump: false, interact: false, reload: false, fire: false, edit: false, slot: 0 };
    const bootAt = performance.now();
    let pending: ShooterEvent[] = [];
    loading = true; error = ''; menu = false; feed = []; spectate = -1; building = false; showMap = false;
    const fail = (e: unknown) => { if (!disposed) { loading = false; error = e instanceof Error ? e.message : 'The match could not start.'; } };
    const boot = async (first: ShooterState) => {
      const { ShooterRenderer } = await import('./render');
      if (disposed) return;
      const me = selfOf(first);
      cam.yaw = first.players[me]?.yaw ?? 0;
      const created = new ShooterRenderer(stage, first.map, first.players.map(p => ({ fighter: p.fighter, human: p.human, name: p.name })), me, abort.signal);
      try { await created.load(); } catch (e) { created.dispose(); throw e; }
      if (disposed) { created.dispose(); return; }
      renderer = created;
      input = new InputState(created.view.canvas, { pointerLock: true, onLockChange: v => { locked = v; } });
      created.view.canvas.addEventListener('focus', () => { focused = true; });
      created.view.canvas.addEventListener('blur', () => { focused = false; });
      created.view.canvas.focus();
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
        if (mine.slot) latch.slot = mine.slot;
        if (now - lastSend > 33) {
          oninput?.({ ...mine, jump: latch.jump, interact: latch.interact, reload: latch.reload, fire: latch.fire, edit: latch.edit, slot: latch.slot || mine.slot }, fighter);
          latch.jump = latch.interact = latch.reload = latch.fire = latch.edit = false; latch.slot = 0;
          lastSend = now;
        }
      }
      for (const e of state.events) if (e.id > lastEvent) { onEvent(state, e, me, now); lastEvent = e.id; }
      const p = state.players[me];
      if (p && p.mode !== 'dead') spectate = -1;
      let watch = me;
      if (p?.mode === 'dead') {
        building = false;
        if (spectate < 0 || state.players[spectate]?.mode === 'dead') spectate = state.players.findIndex(q => q.mode !== 'dead');
        watch = spectate >= 0 ? spectate : me;
        if (watch !== me) { const w = state.players[watch]; cam.yaw = cam.yaw + wrapAngle(w.yaw - cam.yaw) * Math.min(1, dt * 3); }
      }
      if (renderer) renderer.frame(state, me, cam, dt, watch);
      input?.endFrame();
      drawOverlay(now);
      if (now - lastHud > 80) {
        hud = { ...state, storm: { ...state.storm }, bus: { ...state.bus }, chests: [...state.chests], players: state.players.map(q => ({ ...q, weapons: q.weapons.map(w => w && { ...w }), ammo: { ...q.ammo }, mats: [...q.mats] as Fighter['mats'] })) };
        ghostOk = renderer?.ghostValid ?? false;
        if (p && p.mode === 'ground') {
          const chest = nearestClosedChest(state, p);
          const w = nearestWeaponLoot(state, p);
          prompt = building ? '' : chest >= 0 ? (p.interact > 0 ? 'Opening…' : 'Hold E — open chest') : w?.weapon ? `E — pick up ${RARITY_NAMES[w.rarity]} ${itemName(w.weapon)}${isConsumable(w.weapon) ? ` ×${w.amount}` : ''}` : p.swimming ? 'Swimming — head for the shore' : '';
        } else prompt = p?.mode === 'bus' ? 'Press Space to drop' : '';
        drawMap(minimap, state, me, false);
        if (showMap) drawMap(bigmap, state, me, true);
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
      mapImage = null;
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
  async function toggleFullscreen() { try { if (document.fullscreenElement) await document.exitFullscreen(); else await root.requestFullscreen(); } catch { /* embedding may block fullscreen */ } }
  function wake() { void audio.start().catch(() => {}); }
  function touchKey(e: PointerEvent, code: string, down: boolean) { e.preventDefault(); if (down) { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); wake(); } input?.touch(code, down); }
  let lookFrom: { x: number; y: number } | null = null;
  function look(e: PointerEvent) { if (!lookFrom) return; input?.look((e.clientX - lookFrom.x) * 1.6, (e.clientY - lookFrom.y) * 1.6); lookFrom = { x: e.clientX, y: e.clientY }; }

  /** Hotbar labels: a weapon shows mag / reserve, a consumable its count. */
  const slotAmmo = (w: Held) => (isWeapon(w.w) ? `${w.mag}` : `×${w.mag}`);
  const slotReserve = (p: Fighter, w: Held) => (isWeapon(w.w) ? `/${p.ammo[WEAPONS[w.w].ammo]}` : '');
  const consumableKind = (w: Held) => (isConsumable(w.w) ? (CONSUMABLES[w.w].heal ? 'heal' : 'shield') : '');

  const me = $derived(hud ? hud.players[selfOf(hud)] : null);
  const held = $derived(me ? heldItem(me) : null);
  const heldWeapon = $derived(held && isWeapon(held.w) ? WEAPONS[held.w] : null);
  const heldUse = $derived(held && isConsumable(held.w) ? CONSUMABLES[held.w] : null);
  const alive = $derived(hud ? aliveCount(hud) : 0);
  const storm = $derived(hud?.storm);
  const outside = $derived(!!(me && storm && me.mode !== 'dead' && Math.hypot(me.x - storm.cx, me.z - storm.cz) > storm.r));
  const results = $derived(hud && hud.phase === 'done' ? shooterResults(hud) : []);
  const mm = (s: number) => `${Math.floor(Math.max(0, s) / 60)}:${String(Math.floor(Math.max(0, s) % 60)).padStart(2, '0')}`;
  const watched = $derived(hud && spectate >= 0 ? hud.players[spectate] : null);
  const buildHint = $derived(!me ? '' : me.mats[material] < BUILD_COST ? `Need ${BUILD_COST} ${MATERIAL_NAMES[material].toLowerCase()} — harvest with the pickaxe (F)` : ghostOk ? 'Click to place' : 'Nothing to build on here');
</script>

<div class="br-play game-play" bind:this={root} data-game="shooter" data-map={setup?.map ?? hud?.map} data-phase={hud?.phase ?? 'loading'} data-building={building ? 'on' : 'off'} onpointerdown={wake} role="presentation">
  <div class="br-stage" bind:this={stage}></div>
  <canvas class="br-overlay" bind:this={overlay} aria-hidden="true"></canvas>
  {#if loading && !error}<div class="br-cover" role="status"><h2>{isGuest && !hud ? 'Waiting for the host…' : 'Preparing the island…'}</h2><p>Loading survivors, weapons and scenery</p></div>{/if}
  {#if error}<div class="br-cover"><LoadState what="match" {error} empty onretry={start} /><button class="btn" onclick={onexit}>Back to games</button></div>{/if}
  {#if hud && me && !loading && !error}
    {#if outside}<div class="br-storm-tint" aria-hidden="true"></div>{/if}
    {#if aiming}<div class="br-scope" aria-hidden="true"><i></i></div>{/if}
    {#if me.hurtT > 0}<div class="br-hurt" style:opacity={Math.min(0.7, me.hurtT * 1.6)} aria-hidden="true"></div>{/if}
    {#if hurtDir !== null}<div class="br-hurt-dir" style:transform={`translate(-50%, -50%) rotate(${-hurtDir}rad)`} aria-hidden="true"><i></i></div>{/if}
    <div class="br-top">
      <div class="br-stats" aria-label="Match status">
        <span title="Survivors left"><b>{alive}</b> alive</span>
        <span title="Your eliminations"><b>{me.kills}</b> {me.kills === 1 ? 'elim' : 'elims'}</span>
        {#if storm && hud.phase === 'play'}<span class="br-storm" class:shrinking={storm.shrinking}>{storm.stage < 0 ? 'Storm forms in' : storm.shrinking ? 'Storm shrinking' : storm.stage >= STORM.length - 1 ? 'Final circle' : 'Storm moves in'} <b>{mm(storm.timer)}</b>{#if storm.stage >= 0}<small> · phase {Math.min(STORM.length, storm.stage + 1)}/{STORM.length}</small>{/if}</span>{/if}
        {#if hud.phase === 'bus'}<span>Drop ship · <b>{mm(hud.bus.duration - hud.bus.t)}</b></span>{/if}
      </div>
      <div class="br-tools">
        <button class="btn small" aria-label={muted ? 'Unmute game' : 'Mute game'} title={muted ? 'Unmute game' : 'Mute game'} onclick={() => { muted = !muted; audio.mute(muted); }}>{muted ? 'Sound off' : 'Sound on'}</button>
        <button class="btn small" aria-pressed={showMap} onclick={() => { showMap = !showMap; }}>Map</button>
        {#if typeof document !== 'undefined' && document.fullscreenEnabled}<button class="btn small" onclick={toggleFullscreen}>{fullscreen ? 'Exit full screen' : 'Full screen'}</button>{/if}
        <button class="btn small" onclick={() => { menu = !menu; }}>Menu</button>
      </div>
    </div>
    <div class="br-mapbox"><canvas class="br-minimap" bind:this={minimap} width="200" height="200" aria-label="Island map"></canvas>
      <ol class="br-feed" aria-live="polite">{#each feed as f (f.id)}<li class:mine={f.mine}>{f.text}</li>{/each}</ol>
    </div>
    {#if showMap && hud.phase !== 'done'}
      <div class="br-bigmap"><canvas bind:this={bigmap} width="640" height="640" aria-label="Full island map"></canvas><p>{mapFor(hud.map).pois.length} landmarks · white ring: next safe zone · orange: structures · M to close</p></div>
    {/if}
    {#if me.mode === 'ground' && !aiming}
      <div class="br-cross" class:hit={performance.now() - hitMark < 160} class:build={building} style:--spread={`${8 + (heldWeapon && !building ? heldWeapon.spread * (me.aiming ? 0.4 : 1) * 260 : 0)}px`} aria-hidden="true"><i></i><i></i><i></i><i></i></div>
    {/if}
    {#if prompt}<div class="br-prompt">{prompt}</div>{/if}
    {#if toast}<div class="br-toast" aria-live="polite">{toast}</div>{/if}
    {#if me.reload > 0 && heldWeapon}<div class="br-reload" aria-label="Reloading"><span style:width={`${(1 - me.reload / heldWeapon.reload) * 100}%`}></span><em>Reloading</em></div>{/if}
    {#if me.useT > 0 && heldUse}<div class="br-reload use" aria-label={`Using ${heldUse.name}`}><span style:width={`${Math.min(1, me.useT / heldUse.time) * 100}%`}></span><em>Using {heldUse.name}</em></div>{/if}
    {#if me.mode !== 'dead'}
      <div class="br-vitals" aria-label="Health and shield">
        <div class="br-bar shield" role="meter" aria-label="Shield" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(me.shield)}><span style:width={`${me.shield}%`}></span><b><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5z" /></svg>Shield {Math.round(me.shield)}</b></div>
        <div class="br-bar health" role="meter" aria-label="Health" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(me.hp)}><span style:width={`${me.hp}%`}></span><b><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10 3h4v7h7v4h-7v7h-4v-7H3v-4h7z" /></svg>Health {Math.round(me.hp)}</b></div>
        <div class="br-mats" aria-label="Building materials">
          {#each MATERIAL_NAMES as name, m (name)}<span class:active={building && m === material} style:--mat={hex(MATERIAL_COLOURS[m])} title={name}><i aria-hidden="true"></i>{name} <b>{me.mats[m]}</b></span>{/each}
        </div>
      </div>
      {#if building}
        <div class="br-buildbar" aria-label="Build mode">
          {#each BUILD_KIND_NAMES as name, k (name)}<button class="btn small" class:active={k === piece} aria-pressed={k === piece} onclick={() => { piece = k; renderer?.view.canvas.focus(); }}><small>{k + 1}</small>{name}</button>{/each}
          <button class="btn small" onclick={() => { material = (material + 1) % 3; renderer?.view.canvas.focus(); }} title="Change material (R)">{MATERIAL_NAMES[material]} · {BUILD_COST}</button>
          <p class:bad={!ghostOk}>{buildHint} · G edit · Q leave build mode</p>
        </div>
      {/if}
      <div class="br-inventory" aria-label="Hotbar">
        <div class="br-slot br-tool" class:active={me.current === PICKAXE_SLOT && !building} title="Pickaxe (F)"><small>F</small><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 20 14 10M9 5c3-2 8-2 11 1-3-1-6-1-8 1zm3 2 5 5" /></svg><span class="br-empty">Pickaxe</span></div>
        {#each me.weapons as w, i (i)}
          <div class="br-slot" class:active={i === me.current && !building} style:border-color={w ? hex(RARITY_COLOURS[w.rarity]) : undefined}>
            <small>{i + 1}</small>
            {#if w && isWeapon(w.w)}<img src={`/room-games/portraits/${GUN_ICON[w.w]}.webp`} alt={itemName(w.w)} /><span>{slotAmmo(w)}<em>{slotReserve(me, w)}</em></span>
            {:else if w}<svg class={`br-item ${consumableKind(w)}`} viewBox="0 0 24 24" role="img" aria-label={itemName(w.w)}>{#if consumableKind(w) === 'heal'}<path d="M10 3h4v7h7v4h-7v7h-4v-7H3v-4h7z" />{:else}<path d="M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5z" />{/if}</svg><span>{slotAmmo(w)}</span>
            {:else}<span class="br-empty">Empty</span>{/if}
          </div>
        {/each}
        {#if building}<p class="br-held">Building · {BUILD_KIND_NAMES[piece]}</p>
        {:else if held}<p class="br-held">{RARITY_NAMES[held.rarity]} {itemName(held.w)}{#if heldUse && !canUse(heldUse.id, me.hp, me.shield)} · {heldUse.heal ? 'health' : 'shield'} full{/if}</p>
        {:else if me.current === PICKAXE_SLOT}<p class="br-held">Pickaxe — swing at trees, rocks and cars for materials</p>{/if}
        <p class="br-ammo" aria-label="Ammo reserve">{#each AMMO_KINDS as k (k)}<span class:has={me.ammo[k] > 0}>{k} <b>{me.ammo[k]}</b></span>{/each}</p>
      </div>
    {:else if hud.phase !== 'done'}
      <div class="br-spectate"><p>#{me.place} · {me.kills} elims · {watched ? `Spectating ${watched.name || fighterName(watched.fighter)} (${watched.kills} elims)` : 'Match in progress'}</p>
        <button class="btn small" onclick={() => { if (!hud) return; const list = hud.players.filter(q => q.mode !== 'dead'); const idx = list.findIndex(q => q.id === spectate); spectate = list[(idx + 1) % list.length]?.id ?? -1; }}>Next player</button>
        <button class="btn small" onclick={onexit}>Leave</button></div>
    {/if}
    {#if !focused && !locked && hud.phase !== 'done' && !menu && !paused}<div class="br-hint">Click the game to aim · WASD move · Mouse aim · Click fire / use · Right-click zoom · 1–5 hotbar · F pickaxe · Q build (1 wall, 2 floor, 3 ramp, R material, G edit) · E pick up / open · R reload · Space jump / drop · Shift sprint · M map</div>{/if}
    {#if paused && hud.phase !== 'done'}<div class="br-cover" role="status"><h2>Waiting for players</h2><p>The match is paused while someone reconnects.</p><button class="btn" onclick={onexit}>Leave match</button></div>
    {:else if hud.phase === 'done'}
      <div class="br-cover br-results" aria-live="polite">
        <p class="br-kicker">{mapFor(hud.map).id === 'isle' ? 'Palm Isle' : 'Red Mesa'} · Match results · {mm(hud.clock)}</p>
        <h2 class:victory={me.place === 1}>{me.place === 1 ? 'Victory Royale!' : `You placed ${ordinal(me.place)}`}</h2>
        <p>{me.kills} eliminations · {Math.round(me.damage)} damage · survived {mm(me.aliveTime)}</p>
        <table>
          <thead><tr><th scope="col">Place</th><th scope="col">Survivor</th><th scope="col">Elims</th><th scope="col">Damage</th></tr></thead>
          <tbody>{#each results.slice(0, 8) as r (r.id)}<tr class:me={r.id === me.id}><td>#{r.place}</td><td><img src={`/room-games/portraits/fighter-${r.fighter}.webp`} alt="" /><span>{r.name}{r.human ? '' : ' (bot)'}</span></td><td>{r.kills}</td><td>{r.damage}</td></tr>{/each}</tbody>
        </table>
        <div class="br-actions"><button class="btn primary" disabled={rematchDisabled} onclick={onrematch}>{rematchLabel}</button><button class="btn" onclick={onexit}>Back to games</button></div>
      </div>
    {:else if menu}
      <div class="br-cover"><h2>Paused</h2><p>{link ? 'The match continues for everyone else while this menu is open.' : 'The island waits for you.'}</p>
        <p class="br-keys">WASD move · Mouse aim · Left click fire (hold to use heals and shields) · Right click / Z aim down sights · 1–5 or wheel switch hotbar slots · F pickaxe (harvest materials) · Q build mode: 1 wall, 2 floor, 3 ramp, R material, G edit your piece (door / window, flip ramp), click to place · E pick up / hold to open chests · R reload · Space jump (and drop from the balloon) · Shift sprint · M full map · Esc release the mouse</p>
        <button class="btn primary" onclick={() => { menu = false; renderer?.view.canvas.focus(); }}>Resume</button><button class="btn" onclick={onexit}>Leave match</button></div>
    {/if}
    <div class="br-touch" aria-label="Touch controls">
      <div class="br-touch-move">{#each [['Up', '▲'], ['Left', '◀'], ['Down', '▼'], ['Right', '▶']] as [k, label] (k)}<button aria-label={`Move ${k.toLowerCase()}`} onpointerdown={e => touchKey(e, k, true)} onpointerup={e => touchKey(e, k, false)} onpointercancel={e => touchKey(e, k, false)}>{label}</button>{/each}</div>
      <div class="br-look" role="application" aria-label="Drag to aim" onpointerdown={e => { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); lookFrom = { x: e.clientX, y: e.clientY }; }} onpointermove={look} onpointerup={() => (lookFrom = null)} onpointercancel={() => (lookFrom = null)}>Aim</div>
      <div class="br-touch-act">{#each [['Fire', building ? 'Place' : 'Fire'], ['Jump', 'Jump'], ['Use', 'Use'], ['Reload', building ? 'Material' : 'Reload'], ['Build', building ? 'Combat' : 'Build'], ['Pick', 'Pickaxe'], ['Edit', 'Edit']] as [k, label] (k)}<button onpointerdown={e => touchKey(e, k, true)} onpointerup={e => touchKey(e, k, false)} onpointercancel={e => touchKey(e, k, false)}>{label}</button>{/each}</div>
    </div>
  {/if}
</div>

<style>
  .br-play { position: relative; flex: 1; min-height: 420px; overflow: hidden; background: var(--surface); isolation: isolate; user-select: none; --spread: 10px; }
  .br-play:fullscreen { width: 100vw; height: 100vh; }
  .br-stage { position: absolute; inset: 0; }
  .br-stage :global(canvas) { display: block; width: 100%; height: 100%; }
  .br-stage :global(canvas:focus-visible) { outline: 2px solid var(--accent-line-strong); outline-offset: -2px; }
  .br-overlay { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
  .br-cover { position: absolute; inset: 0; z-index: 6; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 24px; text-align: center; background: color-mix(in srgb, var(--surface) 90%, transparent); }
  .br-cover h2 { margin: 0; font-size: var(--fs-2xl); }
  .br-cover h2.victory { color: var(--warning); font-size: clamp(28px, 5vw, 52px); letter-spacing: -0.01em; }
  .br-cover p { margin: 0; max-width: 600px; color: var(--text-dim); line-height: 1.6; }
  .br-top { position: absolute; inset-block-start: 14px; inset-inline: 14px; display: flex; justify-content: space-between; gap: 12px; align-items: flex-start; pointer-events: none; }
  .br-stats, .br-tools, .br-minimap, .br-vitals, .br-slot, .br-spectate, .br-prompt, .br-buildbar { background: var(--scrim-media); color: var(--on-scrim); border: 1px solid color-mix(in srgb, var(--on-scrim) 18%, transparent); border-radius: var(--radius-m); }
  .br-stats { display: flex; gap: 16px; padding: 8px 14px; font-size: var(--fs-s); align-items: baseline; }
  .br-stats b { font-size: var(--fs-l); font-variant-numeric: tabular-nums; }
  .br-stats small { font-size: var(--fs-xs); opacity: .8; }
  .br-storm.shrinking { color: var(--warning); }
  .br-tools { display: flex; gap: 6px; padding: 6px; pointer-events: auto; margin-inline-end: 216px; }
  .br-tools :global(.btn) { background: color-mix(in srgb, var(--on-scrim) 14%, transparent); color: var(--on-scrim); border-color: color-mix(in srgb, var(--on-scrim) 25%, transparent); }
  .br-mapbox { position: absolute; inset-block-start: 14px; inset-inline-end: 14px; width: 200px; pointer-events: none; }
  .br-minimap { width: 200px; height: 200px; display: block; padding: 0; }
  .br-bigmap { position: absolute; inset: 0; z-index: 5; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; background: var(--scrim-media); pointer-events: none; }
  .br-bigmap canvas { width: min(82vmin, 640px); height: min(82vmin, 640px); border-radius: var(--radius-m); border: 1px solid color-mix(in srgb, var(--on-scrim) 30%, transparent); }
  .br-bigmap p { margin: 0; color: var(--on-scrim); font-size: var(--fs-s); }
  .br-feed { list-style: none; margin: 8px 0 0; padding: 0; display: grid; gap: 4px; font-size: var(--fs-s); }
  .br-feed li { background: var(--scrim-media); color: var(--on-scrim); padding: 4px 8px; border-radius: var(--radius-s); text-align: end; }
  .br-feed li.mine { background: color-mix(in srgb, var(--accent-solid) 70%, transparent); }
  .br-cross { position: absolute; inset-block-start: 50%; inset-inline-start: 50%; width: 0; height: 0; pointer-events: none; }
  .br-cross i { position: absolute; background: var(--on-scrim); box-shadow: 0 0 2px var(--scrim-media); }
  .br-cross i:nth-child(1), .br-cross i:nth-child(2) { width: 2px; height: 9px; inset-inline-start: -1px; }
  .br-cross i:nth-child(1) { inset-block-end: var(--spread); } .br-cross i:nth-child(2) { inset-block-start: var(--spread); }
  .br-cross i:nth-child(3), .br-cross i:nth-child(4) { height: 2px; width: 9px; inset-block-start: -1px; }
  .br-cross i:nth-child(3) { inset-inline-end: var(--spread); } .br-cross i:nth-child(4) { inset-inline-start: var(--spread); }
  .br-cross.hit i { background: var(--danger); }
  .br-cross.build i { background: var(--info); }
  .br-prompt { position: absolute; inset-block-start: 60%; inset-inline-start: 50%; transform: translateX(-50%); padding: 6px 14px; font-size: var(--fs-m); white-space: nowrap; pointer-events: none; }
  .br-toast { position: absolute; inset-block-start: 24%; inset-inline: 12px; text-align: center; font-size: clamp(22px, 3.6vw, 40px); font-weight: 600; color: var(--on-scrim); text-shadow: 0 3px 10px var(--scrim-media); pointer-events: none; }
  .br-reload { position: absolute; inset-block-start: 56%; inset-inline-start: 50%; transform: translateX(-50%); width: 140px; height: 6px; border-radius: var(--radius-s); background: var(--scrim-media); pointer-events: none; }
  .br-reload span { display: block; height: 100%; border-radius: var(--radius-s); background: var(--on-scrim); }
  .br-reload.use span { background: var(--success); }
  .br-reload em { position: absolute; inset-block-start: 8px; inset-inline: 0; text-align: center; font-size: var(--fs-s); font-style: normal; color: var(--on-scrim); white-space: nowrap; }
  .br-vitals { position: absolute; inset-block-end: 18px; inset-inline-start: 18px; padding: 10px; width: min(300px, 40vw); display: grid; gap: 6px; pointer-events: none; }
  .br-bar { position: relative; height: 22px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 15%, transparent); overflow: hidden; }
  .br-bar span { position: absolute; inset-block: 0; inset-inline-start: 0; }
  .br-bar.health span { background: var(--success); }
  .br-bar.shield span { background: var(--info); }
  .br-bar b { position: relative; display: inline-flex; align-items: center; gap: 6px; padding-inline-start: 8px; font-size: var(--fs-m); font-weight: 600; line-height: 22px; font-variant-numeric: tabular-nums; text-shadow: 0 1px 2px var(--scrim-media); }
  .br-bar svg { width: 14px; height: 14px; fill: var(--on-scrim); }
  .br-mats { display: flex; flex-wrap: wrap; gap: 4px; font-size: var(--fs-s); --mat: var(--on-scrim); }
  .br-mats span { display: inline-flex; align-items: center; gap: 4px; padding: 2px 6px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 10%, transparent); font-variant-numeric: tabular-nums; }
  .br-mats span.active { box-shadow: inset 0 0 0 2px var(--mat); }
  .br-mats i { width: 10px; height: 10px; border-radius: 2px; background: var(--mat); }
  .br-buildbar { position: absolute; inset-block-end: 132px; inset-inline-start: 50%; transform: translateX(-50%); display: flex; flex-wrap: wrap; justify-content: center; align-items: center; gap: 6px; padding: 8px 10px; max-width: min(680px, 92vw); pointer-events: auto; }
  .br-buildbar :global(.btn) { background: color-mix(in srgb, var(--on-scrim) 14%, transparent); color: var(--on-scrim); border-color: color-mix(in srgb, var(--on-scrim) 25%, transparent); gap: 6px; }
  .br-buildbar :global(.btn.active) { box-shadow: inset 0 0 0 2px var(--info); }
  .br-buildbar small { opacity: .7; font-size: var(--fs-xs); }
  .br-buildbar p { flex-basis: 100%; margin: 0; text-align: center; font-size: var(--fs-s); }
  .br-buildbar p.bad { color: var(--warning); }
  .br-inventory { position: absolute; inset-block-end: 18px; inset-inline-end: 18px; display: grid; grid-template-columns: repeat(6, 72px); gap: 6px; pointer-events: none; }
  .br-slot { height: 72px; position: relative; display: grid; place-items: center; border-width: 2px; opacity: .8; }
  .br-slot.active { opacity: 1; transform: translateY(-6px); box-shadow: 0 0 0 2px var(--on-scrim); }
  .br-slot img { width: 56px; height: 34px; object-fit: contain; }
  .br-slot > svg { width: 28px; height: 28px; fill: none; stroke: var(--on-scrim); stroke-width: 2; stroke-linecap: round; }
  .br-slot .br-item { width: 30px; height: 30px; stroke: none; }
  .br-item.heal { fill: var(--success); }
  .br-item.shield { fill: var(--info); }
  .br-slot small { position: absolute; inset-block-start: 2px; inset-inline-start: 6px; font-size: var(--fs-xs); opacity: .8; }
  .br-slot span { font-size: var(--fs-m); font-weight: 600; font-variant-numeric: tabular-nums; }
  .br-slot span em { font-style: normal; font-weight: 400; opacity: .7; font-size: var(--fs-s); }
  .br-empty { font-size: var(--fs-s) !important; font-weight: 400 !important; opacity: .6; }
  .br-held, .br-ammo { grid-column: 1 / -1; margin: 0; text-align: end; color: var(--on-scrim); font-size: var(--fs-m); }
  .br-held { justify-self: end; background: var(--scrim-media); padding: 2px 8px; border-radius: var(--radius-s); font-weight: 600; }
  .br-ammo { display: flex; flex-wrap: wrap; gap: 4px; justify-content: flex-end; text-transform: capitalize; }
  .br-ammo span { background: var(--scrim-media); padding: 2px 8px; border-radius: var(--radius-s); font-variant-numeric: tabular-nums; }
  .br-ammo span.has { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--on-scrim) 40%, transparent); }
  .br-spectate { position: absolute; inset-block-end: 18px; inset-inline-start: 50%; transform: translateX(-50%); display: flex; gap: 10px; align-items: center; padding: 8px 12px; }
  .br-spectate p { margin: 0; font-size: var(--fs-m); }
  .br-spectate :global(.btn) { background: color-mix(in srgb, var(--on-scrim) 14%, transparent); color: var(--on-scrim); }
  .br-hint { position: absolute; inset-block-end: 130px; inset-inline: 20px; margin-inline: auto; max-width: 760px; text-align: center; background: var(--scrim-media); color: var(--on-scrim); border-radius: var(--radius-s); padding: 8px; font-size: var(--fs-s); pointer-events: none; }
  .br-storm-tint { position: absolute; inset: 0; pointer-events: none; background: color-mix(in srgb, var(--cat-5) 22%, transparent); box-shadow: inset 0 0 140px 40px color-mix(in srgb, var(--cat-5) 60%, transparent); }
  .br-hurt { position: absolute; inset: 0; pointer-events: none; box-shadow: inset 0 0 120px 30px var(--danger); }
  .br-hurt-dir { position: absolute; inset-block-start: 50%; inset-inline-start: 50%; width: 220px; height: 220px; pointer-events: none; }
  .br-hurt-dir i { position: absolute; inset-block-start: 0; inset-inline-start: 50%; width: 48px; height: 14px; margin-inline-start: -24px; border-radius: 50% 50% 0 0; background: var(--danger); opacity: .85; }
  .br-scope { position: absolute; inset: 0; pointer-events: none; background: radial-gradient(circle at center, transparent 0 32vmin, var(--scrim-media) 32.4vmin); }
  .br-scope i { position: absolute; inset-block-start: 50%; inset-inline: calc(50% - 32vmin); height: 1px; background: var(--on-scrim); }
  .br-scope::after { content: ''; position: absolute; inset-inline-start: 50%; inset-block: calc(50% - 32vmin); width: 1px; background: var(--on-scrim); }
  .br-results table { border-collapse: collapse; min-width: min(560px, 100%); font-size: var(--fs-m); font-variant-numeric: tabular-nums; }
  .br-results th { text-align: start; font-size: var(--fs-s); color: var(--text-dim); font-weight: 500; padding: 4px 10px; }
  .br-results td { padding: 4px 10px; border-block-start: 1px solid var(--border); text-align: start; }
  .br-results td:nth-child(2) { display: flex; align-items: center; gap: 8px; }
  .br-results td img { width: 32px; height: 32px; object-fit: contain; }
  .br-results tr.me td { background: var(--accent-soft); font-weight: 600; }
  .br-kicker { text-transform: uppercase; letter-spacing: .06em; font-size: var(--fs-s); }
  .br-actions { display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
  .br-keys { font-size: var(--fs-s); }
  .br-touch { display: none; }
  @media (max-width: 640px) { .br-mapbox { width: 130px; } .br-minimap { width: 130px; height: 130px; } .br-tools { margin-inline-end: 144px; flex-wrap: wrap; } .br-inventory { grid-template-columns: repeat(6, 50px); } .br-slot { height: 54px; } .br-slot img { width: 40px; } .br-stats { flex-wrap: wrap; gap: 8px; } .br-buildbar { inset-block-end: 210px; } }
  @media (pointer: coarse) {
    .br-touch { position: absolute; inset-inline: 10px; inset-block-end: 130px; display: flex; justify-content: space-between; align-items: flex-end; gap: 8px; }
    .br-touch-move { display: grid; grid-template-columns: repeat(3, 48px); grid-template-rows: repeat(2, 48px); gap: 4px; }
    .br-touch-move button:nth-child(1) { grid-column: 2; } .br-touch-move button:nth-child(2) { grid-row: 2; grid-column: 1; } .br-touch-move button:nth-child(3) { grid-row: 2; grid-column: 2; } .br-touch-move button:nth-child(4) { grid-row: 2; grid-column: 3; }
    .br-touch button, .br-look { touch-action: none; min-width: 48px; min-height: 48px; border-radius: var(--radius-m); border: 1px solid var(--border); background: color-mix(in srgb, var(--surface) 80%, transparent); color: var(--text); font-size: var(--fs-s); }
    .br-look { display: grid; place-items: center; width: 110px; height: 96px; }
    .br-touch-act { display: grid; grid-template-columns: repeat(2, 64px); gap: 6px; }
    .br-hint { display: none; }
  }
</style>
