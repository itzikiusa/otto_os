<script lang="ts">
  import { onMount } from 'svelte';
  import LoadState from '../../../../lib/components/LoadState.svelte';
  import { aliveCount, createMatch, idleShooterInput, nearestClosedChest, nearestWeaponLoot, shooterResults, stepMatch, STORM, type ShooterEvent, type ShooterInput, type ShooterLink, type ShooterSetup, type ShooterState } from './sim';
  import { mapFor, HALF } from './map';
  import { AMMO_KINDS, RARITY_COLOURS, RARITY_NAMES, WEAPONS } from './weapons';
  import { fighterName, type FighterId } from './catalog';
  import { InputState } from '../common/input';
  import { GameAudio } from '../common/audio';
  import { clamp, lerp, ordinal, wrapAngle } from '../common/math';
  import type { ShooterRenderer } from './render';

  interface Props {
    setup: ShooterSetup | null; selfSlot: number; fighter: FighterId; link?: ShooterLink;
    names?: (slot: number) => string; paused?: boolean;
    onframe?: (state: ShooterState, events: ShooterEvent[], withLoot: boolean) => void;
    oninput?: (input: ShooterInput, fighter: FighterId) => void;
    onfinish?: (state: ShooterState) => void;
    onexit: () => void; onrematch: () => void; rematchLabel?: string; rematchDisabled?: boolean;
  }
  let { setup, selfSlot, fighter, link, names = () => 'Player', paused = false, onframe, oninput, onfinish, onexit, onrematch, rematchLabel = 'Play again', rematchDisabled = false }: Props = $props();

  let stage: HTMLDivElement, root: HTMLDivElement;
  let minimap = $state<HTMLCanvasElement>();
  let overlay = $state<HTMLCanvasElement>();
  let loading = $state(true), error = $state(''), menu = $state(false), muted = $state(false), fullscreen = $state(false), locked = $state(false), focused = $state(false);
  let hud = $state.raw<ShooterState | null>(null);
  let feed = $state<{ id: number; text: string; mine: boolean }[]>([]);
  let toast = $state(''), prompt = $state(''), hitMark = $state(0), hurtDir = $state<number | null>(null);
  let aiming = $state(false), spectate = $state(-1);
  const audio = new GameAudio(['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket', 'explosion', 'hit', 'shieldhit', 'headshot', 'reload', 'pickup', 'chest', 'storm', 'eliminated', 'land', 'voice-win', 'voice-over', 'voice-hurry', 'voice-go', 'select', 'step']);
  let renderer: ShooterRenderer | undefined, input: InputState | undefined;
  let stop: (() => void) | undefined;
  const isGuest = $derived(link?.role === 'guest');
  const cam = { yaw: 0, pitch: -0.15, aiming: false, zoom: 70 };
  let slotWanted = 0;
  const numbers: { x: number; y: number; z: number; text: string; color: string; t: number }[] = [];
  const GUN_ICON: Record<string, string> = { pistol: 'gun-pistol', smg: 'gun-smg', rifle: 'gun-rifle', shotgun: 'gun-shotgun', sniper: 'gun-sniper', rocket: 'gun-rocket' };
  const hex = (n: number) => `#${n.toString(16).padStart(6, '0')}`;

  const selfOf = (s: ShooterState) => Math.max(0, s.players.findIndex(p => p.human && p.slot === selfSlot));

  function readInput(state: ShooterState, me: number): ShooterInput {
    const i = input;
    const p = state.players[me];
    if (!i || menu || paused || !p) return idleShooterInput(cam.yaw, cam.pitch);
    const [dx, dy] = i.consumeLook();
    const sens = (cam.aiming ? 0.0012 : 0.0026) * (cam.zoom / 70);
    cam.yaw = wrapAngle(cam.yaw - dx * sens);
    cam.pitch = clamp(cam.pitch - dy * sens, -1.25, 1.2);
    if (i.wheel) slotWanted = ((p.current + (i.wheel > 0 ? 1 : -1)) % 3 + 3) % 3 + 1;
    for (let k = 1; k <= 3; k++) if (i.hit(`Digit${k}`)) slotWanted = k;
    const slot = slotWanted; slotWanted = 0;
    const aim = i.mouse.right || i.down('KeyZ');
    cam.aiming = aim && p.mode === 'ground' && !!p.weapons[p.current];
    const def = p.weapons[p.current] ? WEAPONS[p.weapons[p.current]!.w] : null;
    cam.zoom = cam.aiming ? def?.zoom ?? 55 : p.mode === 'sky' || p.mode === 'glide' ? 78 : 70;
    aiming = cam.aiming && def?.id === 'sniper';
    // Aim the muzzle at whatever is under the crosshair.
    let yaw = cam.yaw, pitch = cam.pitch;
    if (renderer && p.mode === 'ground') {
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
      aim, jump: i.down('Space', 'Jump') || i.hit('Space', 'Jump'), sprint: i.down('ShiftLeft', 'ShiftRight'), reload: i.down('KeyR', 'Reload') || i.hit('KeyR', 'Reload'), interact: i.down('KeyE', 'KeyF', 'Use') || i.hit('KeyE', 'KeyF', 'Use'), slot,
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
        feed = [...feed.slice(-4), { id: e.id, text: e.player >= 0 ? `${killer} ⟶ ${victim}` : `${victim} ${how}`, mine: e.player === me || e.target === me }];
        setTimeout(() => { feed = feed.filter(f => f.id !== e.id); }, 6000);
        if (e.player === me && e.target !== me) { audio.play('eliminated'); say(`You eliminated ${victim}`); }
        if (e.target === me) { audio.play('voice-over'); say(e.player >= 0 ? `Eliminated by ${killer}` : `You ${how.replace('was ', 'were ')}`, 3000); spectate = e.player >= 0 ? e.player : -1; }
        break;
      }
      case 'pickup': if (e.player === me) audio.play('pickup', { volume: 0.5 }); break;
      case 'reload': if (e.player === me) audio.play('reload', { volume: 0.6 }); break;
      case 'chest': audio.play('chest', { distance: dist }); break;
      case 'explode': audio.play('explosion', { distance: dist, volume: 1.2 }); break;
      case 'land': if (e.player === me) audio.play('land'); break;
      case 'storm': audio.play('storm', { volume: 0.7 }); if (e.flag === 1) { audio.play('voice-hurry'); say('The storm is closing in!'); } else say('Storm eye revealed — check the map', 2200); break;
      case 'win': if (e.player === me) { audio.play('voice-win'); say('Victory Royale!', 4000); } break;
      case 'jump': if (e.player === me) audio.play('select'); break;
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
    for (let i = 0; i < map.size * map.size; i++) {
      const h = map.heights[i];
      let r: number, gg: number, b: number;
      if (map.theme === 'isle') { if (h < 0) { r = 46; gg = 128; b = 196; } else if (h < 1.6) { r = 232; gg = 211; b = 154; } else { const k = Math.min(1, h / 16); r = 110 - k * 30; gg = 185 - k * 40; b = 80 - k * 20; } }
      else { const k = Math.min(1, h / 30); r = 215 - k * 40; gg = 150 - k * 50; b = 92 - k * 30; }
      img.data[i * 4] = r; img.data[i * 4 + 1] = gg; img.data[i * 4 + 2] = b; img.data[i * 4 + 3] = 255;
    }
    g.putImageData(img, 0, 0);
    g.fillStyle = 'rgba(60,60,70,0.85)';
    for (const b of map.boxes) { const s = map.size / (HALF * 2); g.fillRect((b.minX + HALF) * s, (b.minZ + HALF) * s, Math.max(1, (b.maxX - b.minX) * s), Math.max(1, (b.maxZ - b.minZ) * s)); }
    mapImage = c;
    return c;
  }
  function drawMinimap(state: ShooterState, me: number) {
    const c = minimap; if (!c) return;
    const g = c.getContext('2d'); if (!g) return;
    const w = c.width, s = w / (HALF * 2);
    // Mirror X so the map reads like the third-person view (+X is to the left facing +Z).
    const mx = (x: number) => w - (x + HALF) * s, mz = (z: number) => (z + HALF) * s;
    g.save();
    g.clearRect(0, 0, w, w);
    g.translate(w, 0); g.scale(-1, 1); g.drawImage(islandImage(state), 0, 0, w, w); g.restore();
    const st = state.storm;
    g.fillStyle = 'rgba(120,60,200,0.35)';
    g.beginPath(); g.rect(0, 0, w, w); g.arc(mx(st.cx), mz(st.cz), Math.max(0, st.r) * s, 0, Math.PI * 2, true); g.fill();
    g.strokeStyle = '#ffffff'; g.lineWidth = 2; g.setLineDash([5, 4]);
    g.beginPath(); g.arc(mx(st.toX), mz(st.toZ), Math.max(0, st.toR) * s, 0, Math.PI * 2); g.stroke(); g.setLineDash([]);
    if (state.phase === 'bus') { g.strokeStyle = 'rgba(255,255,255,0.8)'; g.lineWidth = 3; g.beginPath(); g.moveTo(mx(state.bus.ax), mz(state.bus.az)); g.lineTo(mx(state.bus.bx), mz(state.bus.bz)); g.stroke(); }
    g.font = '600 10px system-ui, sans-serif'; g.fillStyle = 'rgba(255,255,255,0.9)'; g.textAlign = 'center';
    for (const poi of mapFor(state.map).pois) g.fillText(poi.name, mx(poi.x), mz(poi.z));
    for (const p of state.players) {
      if (p.mode === 'dead' || p.id === me || !p.human) continue;
      g.fillStyle = '#ffd23f'; g.beginPath(); g.arc(mx(p.x), mz(p.z), 4, 0, Math.PI * 2); g.fill();
    }
    const p = state.players[me];
    if (p && p.mode !== 'dead') {
      g.save(); g.translate(mx(p.x), mz(p.z)); g.rotate(cam.yaw);
      g.fillStyle = '#ffffff'; g.strokeStyle = '#000'; g.lineWidth = 1.5;
      g.beginPath(); g.moveTo(0, 7); g.lineTo(5, -5); g.lineTo(0, -2); g.lineTo(-5, -5); g.closePath(); g.fill(); g.stroke();
      g.restore();
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
    let disposed = false, frame = 0, last = performance.now(), lastHud = 0, lastSend = 0, lastEvent = 0, bootStarted = false, finished = false, lastLootRev = -1, sentAt = 0;
    let state: ShooterState | null = setup ? createMatch(setup) : null;
    const latch = { jump: false, interact: false, reload: false, fire: false, slot: 0 };
    const bootAt = performance.now();
    let pending: ShooterEvent[] = [];
    loading = true; error = ''; menu = false; feed = []; spectate = -1;
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
          const withLoot = state.lootRev !== lastLootRev || Math.floor(now / 2000) !== Math.floor(sentAt / 2000);
          onframe(state, pending, withLoot);
          if (withLoot) lastLootRev = state.lootRev;
          pending = []; sentAt = now;
        }
      } else {
        // Latch one-shot buttons so a tap between two sends still reaches the host.
        latch.jump ||= mine.jump; latch.interact ||= mine.interact; latch.reload ||= mine.reload; latch.fire ||= mine.fire;
        if (mine.slot) latch.slot = mine.slot;
        if (now - lastSend > 33) {
          oninput?.({ ...mine, jump: latch.jump, interact: latch.interact, reload: latch.reload, fire: latch.fire, slot: latch.slot || mine.slot }, fighter);
          latch.jump = latch.interact = latch.reload = latch.fire = false; latch.slot = 0;
          lastSend = now;
        }
      }
      for (const e of state.events) if (e.id > lastEvent) { onEvent(state, e, me, now); lastEvent = e.id; }
      const p = state.players[me];
      if (p && p.mode !== 'dead') spectate = -1;
      let watch = me;
      if (p?.mode === 'dead') {
        if (spectate < 0 || state.players[spectate]?.mode === 'dead') spectate = state.players.findIndex(q => q.mode !== 'dead');
        watch = spectate >= 0 ? spectate : me;
        if (watch !== me) { const w = state.players[watch]; cam.yaw = cam.yaw + wrapAngle(w.yaw - cam.yaw) * Math.min(1, dt * 3); }
      }
      if (renderer) renderer.frame(state, me, cam, dt, watch);
      input?.endFrame();
      drawOverlay(now);
      if (now - lastHud > 80) {
        hud = { ...state, players: state.players.map(q => ({ ...q, weapons: [...q.weapons] })) };
        if (p && p.mode === 'ground') {
          const chest = nearestClosedChest(state, p);
          const w = nearestWeaponLoot(state, p);
          prompt = chest >= 0 ? (p.interact > 0 ? 'Opening…' : 'Hold E — open chest') : w?.weapon ? `E — pick up ${RARITY_NAMES[w.rarity]} ${WEAPONS[w.weapon].name}` : '';
        } else prompt = p?.mode === 'bus' ? 'Press Space to drop' : '';
        drawMinimap(state, me);
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

  const me = $derived(hud ? hud.players[selfOf(hud)] : null);
  const held = $derived(me ? me.weapons[me.current] : null);
  const alive = $derived(hud ? aliveCount(hud) : 0);
  const storm = $derived(hud?.storm);
  const outside = $derived(!!(me && storm && me.mode !== 'dead' && Math.hypot(me.x - storm.cx, me.z - storm.cz) > storm.r));
  const results = $derived(hud && hud.phase === 'done' ? shooterResults(hud) : []);
  const mm = (s: number) => `${Math.floor(Math.max(0, s) / 60)}:${String(Math.floor(Math.max(0, s) % 60)).padStart(2, '0')}`;
  const watched = $derived(hud && spectate >= 0 ? hud.players[spectate] : null);
</script>

<div class="br-play game-play" bind:this={root} data-game="shooter" data-map={setup?.map ?? hud?.map} data-phase={hud?.phase ?? 'loading'} onpointerdown={wake} role="presentation">
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
        {#if storm && hud.phase === 'play'}<span class="br-storm" class:shrinking={storm.shrinking}>{storm.stage < 0 ? 'Storm forms in' : storm.shrinking ? 'Storm shrinking' : storm.stage >= STORM.length - 1 ? 'Final circle' : 'Storm moves in'} <b>{mm(storm.timer)}</b></span>{/if}
        {#if hud.phase === 'bus'}<span>Drop ship · <b>{mm(hud.bus.duration - hud.bus.t)}</b></span>{/if}
      </div>
      <div class="br-tools">
        <button class="btn small" aria-label={muted ? 'Unmute game' : 'Mute game'} title={muted ? 'Unmute game' : 'Mute game'} onclick={() => { muted = !muted; audio.mute(muted); }}>{muted ? 'Sound off' : 'Sound on'}</button>
        {#if typeof document !== 'undefined' && document.fullscreenEnabled}<button class="btn small" onclick={toggleFullscreen}>{fullscreen ? 'Exit full screen' : 'Full screen'}</button>{/if}
        <button class="btn small" onclick={() => { menu = !menu; }}>Menu</button>
      </div>
    </div>
    <div class="br-mapbox"><canvas class="br-minimap" bind:this={minimap} width="200" height="200" aria-label="Island map"></canvas>
      <ol class="br-feed" aria-live="polite">{#each feed as f (f.id)}<li class:mine={f.mine}>{f.text}</li>{/each}</ol>
    </div>
    {#if me.mode === 'ground' && !aiming}
      <div class="br-cross" class:hit={performance.now() - hitMark < 160} style:--spread={`${8 + (held ? WEAPONS[held.w].spread * (me.aiming ? 0.4 : 1) * 260 : 0)}px`} aria-hidden="true"><i></i><i></i><i></i><i></i></div>
    {/if}
    {#if prompt}<div class="br-prompt">{prompt}</div>{/if}
    {#if toast}<div class="br-toast" aria-live="polite">{toast}</div>{/if}
    {#if me.reload > 0 && held}<div class="br-reload" aria-label="Reloading"><span style:width={`${(1 - me.reload / WEAPONS[held.w].reload) * 100}%`}></span><em>Reloading</em></div>{/if}
    {#if me.mode !== 'dead'}
      <div class="br-vitals" aria-label="Health and shield">
        <div class="br-bar shield"><span style:width={`${me.shield}%`}></span><b>{Math.round(me.shield)}</b></div>
        <div class="br-bar health"><span style:width={`${me.hp}%`}></span><b>{Math.round(me.hp)}</b></div>
      </div>
      <div class="br-inventory" aria-label="Weapons">
        {#each me.weapons as w, i (i)}
          <div class="br-slot" class:active={i === me.current} style:border-color={w ? hex(RARITY_COLOURS[w.rarity]) : undefined}>
            <small>{i + 1}</small>
            {#if w}<img src={`/room-games/portraits/${GUN_ICON[w.w]}.webp`} alt={WEAPONS[w.w].name} /><span>{w.mag}<em>/{me.ammo[WEAPONS[w.w].ammo]}</em></span>{:else}<span class="br-empty">Empty</span>{/if}
          </div>
        {/each}
        {#if held}<p class="br-held">{RARITY_NAMES[held.rarity]} {WEAPONS[held.w].name}</p>{/if}
        <p class="br-ammo">{#each AMMO_KINDS as k (k)}<span title={k}>{k} {me.ammo[k]}</span>{/each}</p>
      </div>
    {:else if hud.phase !== 'done'}
      <div class="br-spectate"><p>#{me.place} · {me.kills} elims · {watched ? `Spectating ${watched.name || fighterName(watched.fighter)}` : 'Match in progress'}</p>
        <button class="btn small" onclick={() => { if (!hud) return; const list = hud.players.filter(q => q.mode !== 'dead'); const idx = list.findIndex(q => q.id === spectate); spectate = list[(idx + 1) % list.length]?.id ?? -1; }}>Next player</button>
        <button class="btn small" onclick={onexit}>Leave</button></div>
    {/if}
    {#if !focused && !locked && hud.phase !== 'done' && !menu && !paused}<div class="br-hint">Click the game to aim · WASD move · Mouse aim · Click fire · Right-click zoom · 1–3 weapons · E pick up / open · R reload · Space jump / drop · Shift sprint</div>{/if}
    {#if paused && hud.phase !== 'done'}<div class="br-cover" role="status"><h2>Waiting for players</h2><p>The match is paused while someone reconnects.</p><button class="btn" onclick={onexit}>Leave match</button></div>
    {:else if hud.phase === 'done'}
      <div class="br-cover br-results" aria-live="polite">
        <p class="br-kicker">{mapFor(hud.map).id === 'isle' ? 'Palm Isle' : 'Red Mesa'} · Match results</p>
        <h2>{me.place === 1 ? 'Victory Royale!' : `You placed ${ordinal(me.place)}`}</h2>
        <p>{me.kills} eliminations · {Math.round(me.damage)} damage · survived {mm(me.aliveTime)}</p>
        <table>
          <thead><tr><th scope="col">Place</th><th scope="col">Survivor</th><th scope="col">Elims</th><th scope="col">Damage</th></tr></thead>
          <tbody>{#each results.slice(0, 8) as r (r.id)}<tr class:me={r.id === me.id}><td>#{r.place}</td><td><img src={`/room-games/portraits/fighter-${r.fighter}.webp`} alt="" /><span>{r.name}{r.human ? '' : ' (bot)'}</span></td><td>{r.kills}</td><td>{r.damage}</td></tr>{/each}</tbody>
        </table>
        <div class="br-actions"><button class="btn primary" disabled={rematchDisabled} onclick={onrematch}>{rematchLabel}</button><button class="btn" onclick={onexit}>Back to games</button></div>
      </div>
    {:else if menu}
      <div class="br-cover"><h2>Paused</h2><p>{link ? 'The match continues for everyone else while this menu is open.' : 'The island waits for you.'}</p>
        <p class="br-keys">WASD move · Mouse aim · Left click fire · Right click / Z aim down sights · 1–3 or wheel switch weapons · E pick up / hold to open chests · R reload · Space jump (and drop from the balloon) · Shift sprint · Esc release the mouse</p>
        <button class="btn primary" onclick={() => { menu = false; renderer?.view.canvas.focus(); }}>Resume</button><button class="btn" onclick={onexit}>Leave match</button></div>
    {/if}
    <div class="br-touch" aria-label="Touch controls">
      <div class="br-touch-move">{#each [['Up', '▲'], ['Left', '◀'], ['Down', '▼'], ['Right', '▶']] as [k, label] (k)}<button aria-label={`Move ${k.toLowerCase()}`} onpointerdown={e => touchKey(e, k, true)} onpointerup={e => touchKey(e, k, false)} onpointercancel={e => touchKey(e, k, false)}>{label}</button>{/each}</div>
      <div class="br-look" role="application" aria-label="Drag to aim" onpointerdown={e => { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); lookFrom = { x: e.clientX, y: e.clientY }; }} onpointermove={look} onpointerup={() => (lookFrom = null)} onpointercancel={() => (lookFrom = null)}>Aim</div>
      <div class="br-touch-act">{#each [['Fire', 'Fire'], ['Jump', 'Jump'], ['Use', 'Use'], ['Reload', 'Reload']] as [k, label] (k)}<button onpointerdown={e => touchKey(e, k, true)} onpointerup={e => touchKey(e, k, false)} onpointercancel={e => touchKey(e, k, false)}>{label}</button>{/each}</div>
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
  .br-cover p { margin: 0; max-width: 600px; color: var(--text-dim); line-height: 1.6; }
  .br-top { position: absolute; inset-block-start: 14px; inset-inline: 14px; display: flex; justify-content: space-between; gap: 12px; align-items: flex-start; pointer-events: none; }
  .br-stats, .br-tools, .br-minimap, .br-vitals, .br-slot, .br-spectate, .br-prompt { background: var(--scrim-media); color: var(--on-scrim); border: 1px solid color-mix(in srgb, var(--on-scrim) 18%, transparent); border-radius: var(--radius-m); }
  .br-stats { display: flex; gap: 16px; padding: 8px 14px; font-size: var(--fs-s); align-items: baseline; }
  .br-stats b { font-size: var(--fs-l); font-variant-numeric: tabular-nums; }
  .br-storm.shrinking { color: var(--warning); }
  .br-tools { display: flex; gap: 6px; padding: 6px; pointer-events: auto; margin-inline-end: 216px; }
  .br-tools :global(.btn) { background: color-mix(in srgb, var(--on-scrim) 14%, transparent); color: var(--on-scrim); border-color: color-mix(in srgb, var(--on-scrim) 25%, transparent); }
  .br-mapbox { position: absolute; inset-block-start: 14px; inset-inline-end: 14px; width: 200px; pointer-events: none; }
  .br-minimap { width: 200px; height: 200px; display: block; padding: 0; }
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
  .br-prompt { position: absolute; inset-block-start: 60%; inset-inline-start: 50%; transform: translateX(-50%); padding: 6px 14px; font-size: var(--fs-m); white-space: nowrap; pointer-events: none; }
  .br-toast { position: absolute; inset-block-start: 24%; inset-inline: 12px; text-align: center; font-size: clamp(22px, 3.6vw, 40px); font-weight: 600; color: var(--on-scrim); text-shadow: 0 3px 10px var(--scrim-media); pointer-events: none; }
  .br-reload { position: absolute; inset-block-start: 56%; inset-inline-start: 50%; transform: translateX(-50%); width: 140px; height: 6px; border-radius: var(--radius-s); background: var(--scrim-media); pointer-events: none; }
  .br-reload span { display: block; height: 100%; border-radius: var(--radius-s); background: var(--on-scrim); }
  .br-reload em { position: absolute; inset-block-start: 8px; inset-inline: 0; text-align: center; font-size: var(--fs-s); font-style: normal; color: var(--on-scrim); }
  .br-vitals { position: absolute; inset-block-end: 18px; inset-inline-start: 18px; padding: 10px; width: min(300px, 40vw); display: grid; gap: 6px; pointer-events: none; }
  .br-bar { position: relative; height: 18px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 15%, transparent); overflow: hidden; }
  .br-bar span { position: absolute; inset-block: 0; inset-inline-start: 0; }
  .br-bar.health span { background: var(--success); }
  .br-bar.shield span { background: var(--info); }
  .br-bar b { position: relative; padding-inline-start: 8px; font-size: var(--fs-s); line-height: 18px; font-variant-numeric: tabular-nums; }
  .br-inventory { position: absolute; inset-block-end: 18px; inset-inline-end: 18px; display: grid; grid-template-columns: repeat(3, 88px); gap: 6px; pointer-events: none; }
  .br-slot { height: 78px; position: relative; display: grid; place-items: center; border-width: 2px; opacity: .8; }
  .br-slot.active { opacity: 1; transform: translateY(-6px); box-shadow: 0 0 0 2px var(--on-scrim); }
  .br-slot img { width: 64px; height: 40px; object-fit: contain; }
  .br-slot small { position: absolute; inset-block-start: 2px; inset-inline-start: 6px; font-size: var(--fs-xs); opacity: .8; }
  .br-slot span { font-size: var(--fs-m); font-weight: 600; font-variant-numeric: tabular-nums; }
  .br-slot span em { font-style: normal; font-weight: 400; opacity: .7; font-size: var(--fs-s); }
  .br-empty { font-size: var(--fs-s) !important; font-weight: 400 !important; opacity: .6; }
  .br-held, .br-ammo { grid-column: 1 / -1; margin: 0; text-align: end; color: var(--on-scrim); text-shadow: 0 1px 3px var(--scrim-media); font-size: var(--fs-s); }
  .br-ammo { display: flex; gap: 8px; justify-content: flex-end; text-transform: capitalize; opacity: .9; }
  .br-spectate { position: absolute; inset-block-end: 18px; inset-inline-start: 50%; transform: translateX(-50%); display: flex; gap: 10px; align-items: center; padding: 8px 12px; }
  .br-spectate p { margin: 0; font-size: var(--fs-m); }
  .br-spectate :global(.btn) { background: color-mix(in srgb, var(--on-scrim) 14%, transparent); color: var(--on-scrim); }
  .br-hint { position: absolute; inset-block-end: 130px; inset-inline: 20px; margin-inline: auto; max-width: 700px; text-align: center; background: var(--scrim-media); color: var(--on-scrim); border-radius: var(--radius-s); padding: 8px; font-size: var(--fs-s); pointer-events: none; }
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
  @media (max-width: 640px) { .br-mapbox { width: 130px; } .br-minimap { width: 130px; height: 130px; } .br-tools { margin-inline-end: 144px; flex-wrap: wrap; } .br-inventory { grid-template-columns: repeat(3, 64px); } .br-slot { height: 60px; } .br-slot img { width: 46px; } .br-stats { flex-wrap: wrap; gap: 8px; } }
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
