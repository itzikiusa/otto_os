<script lang="ts" module>
  import type { KartInput, KartSeat, KartState, Difficulty } from './sim';
  import type { TrackId } from './tracks';
  import type { RacerId } from './roster';
  export interface KartSetup { track: TrackId; difficulty: Difficulty; seats: KartSeat[]; seed: number }
  /** Network glue supplied by GameMatch; absent for solo races. */
  export interface KartLink {
    role: 'host' | 'guest';
    /** Host: latest decoded input per member slot. */
    inputs?: Map<number, { input: KartInput; racer: RacerId }>;
    /** Guest: buffered snapshots (newest last). */
    snapshots?: { state: KartState; at: number }[];
  }
</script>
<script lang="ts">
  import { onMount } from 'svelte';
  import LoadState from '../../../../lib/components/LoadState.svelte';
  import { createRace, GOLDEN_SECONDS, idleInput, MAX_COINS, standings, stepRace, type KartEvent } from './sim';
  import { drawLift, F_GAP, pointAt, trackFor, type Path, type Track } from './tracks';
  import { racerDef } from './roster';
  import { ITEMS, type ItemKind } from './items';
  import { itemArt, noticeFor, placeTier, ROULETTE } from './hud';
  import { InputState } from '../common/input';
  import { GameAudio } from '../common/audio';
  import { clamp, ordinal, raceClock, lerp, wrapAngle } from '../common/math';
  import type { KartRenderer } from './render';

  interface Props {
    setup: KartSetup | null; selfSlot: number; racer: RacerId; link?: KartLink;
    names?: (slot: number) => string; paused?: boolean;
    onframe?: (state: KartState, events: KartEvent[]) => void;
    oninput?: (input: KartInput, racer: RacerId) => void;
    onfinish?: (state: KartState) => void;
    onexit: () => void; onrematch: () => void; rematchLabel?: string; rematchDisabled?: boolean;
  }
  let { setup, selfSlot, racer, link, names = () => 'Player', paused = false, onframe, oninput, onfinish, onexit, onrematch, rematchLabel = 'Race again', rematchDisabled = false }: Props = $props();

  let stage: HTMLDivElement, root: HTMLDivElement;
  let minimap = $state<HTMLCanvasElement>();
  let loading = $state(true), error = $state(''), menu = $state(false), muted = $state(false), fullscreen = $state(false), active = $state(false);
  let hud = $state.raw<KartState | null>(null);
  let notice = $state(''), noticeKind = $state('');
  let rouletteIcon = $state(0);
  /** Screen flash for lightning: bumped per flash so the overlay restarts its fade. */
  let flash = $state(0), flashKind = $state<'bolt' | 'zap'>('bolt');
  let waitingFor = $state<string[]>([]);
  let underwater = $state(false);
  const audio = new GameAudio(['voice-3', 'voice-2', 'voice-1', 'voice-go', 'item', 'tick', 'boost', 'turbo', 'spin', 'explode', 'bump', 'lap', 'finallap', 'drop', 'shield', 'land', 'pickup', 'voice-win', 'voice-congrats', 'voice-over', 'select']);
  let renderer: KartRenderer | undefined, input: InputState | undefined;
  let stop: (() => void) | undefined;
  const isGuest = $derived(link?.role === 'guest');
  const self = $derived(hud ? hud.racers.find(r => (r.human && r.slot === selfSlot)) ?? hud.racers[0] : null);
  const selfIndex = $derived(hud && self ? hud.racers.indexOf(self) : 0);

  function readInput(touchGas: boolean): KartInput {
    const i = input;
    if (!i || menu || paused) return idleInput();
    const up = i.down('KeyW', 'ArrowUp') || touchGas, down = i.down('KeyS', 'ArrowDown');
    const left = i.down('KeyA', 'ArrowLeft'), right = i.down('KeyD', 'ArrowRight');
    return {
      throttle: up && !down ? 1 : down && !up ? -1 : 0,
      steer: (right ? 1 : 0) - (left ? 1 : 0),
      drift: i.down('Space', 'ShiftLeft', 'ShiftRight', 'Drift') || i.hit('Space', 'Drift'),
      item: i.down('KeyE', 'KeyF', 'Enter', 'Item') || i.hit('KeyE', 'KeyF', 'Enter', 'Item'),
      respawn: i.down('KeyR'),
    };
  }

  function announce(text: string, kind = '', ms = 1400): void {
    notice = text; noticeKind = kind;
    const shown = text;
    setTimeout(() => { if (notice === shown) notice = ''; }, ms);
  }

  function onEvent(state: KartState, e: KartEvent, mine: number): void {
    renderer?.effect(e);
    const r = state.racers[e.racer];
    const own = e.racer === mine;
    const me = state.racers[mine];
    const dist = me ? Math.hypot(e.x - me.x, e.z - me.z) : 0;
    const vol = { distance: own ? 0 : dist };
    // The call-out (hud.noticeFor: lightning is good news for its user, bad only for whoever it strikes).
    const call = noticeFor(e, mine, state.racers);
    if (call) announce(call.text, call.kind, call.ms);
    switch (e.type) {
      case 'count': audio.play(`voice-${e.value}`, { volume: 1.1 }); break;
      case 'go': audio.play('voice-go', { volume: 1.1 }); break;
      case 'box': audio.play('pickup', { ...vol, volume: 0.6 }); break;
      case 'item': if (own) audio.play('item'); break;
      case 'boost': audio.play('boost', { ...vol, volume: 0.7 }); break;
      case 'turbo': audio.play('turbo', { ...vol, volume: 0.6 + (e.value ?? 1) * 0.1, rate: 0.9 + (e.value ?? 1) * 0.1 }); break;
      case 'draft': if (own) audio.play('boost', { volume: 0.5, rate: 1.2 }); break;
      case 'burnout': if (own) audio.play('bump', { volume: 0.6, rate: 0.6 }); break;
      case 'spin': audio.play('spin', vol); break;
      case 'explode': audio.play('explode', vol); break;
      case 'bump': audio.play('bump', { ...vol, volume: 0.5 }); break;
      case 'drop': audio.play('drop', vol); break;
      case 'throw': audio.play('boost', { ...vol, rate: 0.7 }); break;
      case 'star': audio.play('shield', vol); break;
      case 'bolt': audio.play('explode', { volume: 0.8, rate: 1.4 }); flashKind = 'bolt'; flash++; break;
      case 'zap': if (own) { audio.play('spin', { volume: 0.9, rate: 0.8 }); flashKind = 'zap'; flash++; } break;
      case 'trick': if (own) audio.play('select'); break;
      case 'land': if (own) audio.play('land', { volume: 0.6 }); break;
      case 'splash':
        // value 1: diving under on purpose; 0: fell in the water; 2: lava.
        if (e.value === 1) { if (own) audio.play('land', { volume: 0.5, rate: 0.7 }); break; }
        audio.play(e.value === 2 ? 'explode' : 'land', { ...vol, rate: e.value === 2 ? 0.6 : 0.5 });
        break;
      case 'glide': if (own) audio.play('boost', { rate: 0.8 }); break;
      case 'crush': audio.play('bump', { ...vol, volume: 0.9, rate: 0.7 }); break;
      case 'lap': if (own) audio.play('lap'); break;
      case 'final': if (own) audio.play('finallap'); break;
      case 'finish': if (own) audio.play(r && r.place <= 3 ? 'voice-congrats' : 'lap'); break;
    }
  }

  /** Minimap pixel density (the canvas is drawn at twice its CSS size for crisp lines). */
  const MAP_DPR = 2;
  /** The minimap's static layer (every route, forks, the start line) and its projection, per track. */
  let mapLayer: { track: string; canvas: HTMLCanvasElement; mx: (x: number) => number; mz: (z: number) => number } | null = null;
  function mapBase(track: Track, w: number, h: number): NonNullable<typeof mapLayer> {
    if (mapLayer?.track === track.def.id) return mapLayer;
    let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
    for (const p of track.paths) for (let i = 0; i < p.count; i++) { minX = Math.min(minX, p.px[i]); maxX = Math.max(maxX, p.px[i]); minZ = Math.min(minZ, p.pz[i]); maxZ = Math.max(maxZ, p.pz[i]); }
    const pad = 16 * MAP_DPR;
    const sc = Math.min((w - pad * 2) / (maxX - minX), (h - pad * 2) / (maxZ - minZ));
    // Mirror X so the map matches the chase view (+X is to the left when facing +Z).
    const mx = (x: number) => w - (pad + (x - minX) * sc + (w - pad * 2 - (maxX - minX) * sc) / 2);
    const mz = (z: number) => pad + (z - minZ) * sc + (h - pad * 2 - (maxZ - minZ) * sc) / 2;
    const canvas = document.createElement('canvas');
    canvas.width = w; canvas.height = h;
    const g = canvas.getContext('2d')!;
    /** Traces a route, lifting the pen over gaps (jumps and glides show as breaks). */
    const trace = (p: Path) => {
      g.beginPath();
      let pen = false;
      for (let i = 0; i <= (p.closed ? p.count : p.count - 1); i++) {
        const k = i % p.count;
        if (p.flags[k] & F_GAP) { pen = false; continue; }
        if (pen) g.lineTo(mx(p.px[k]), mz(p.pz[k])); else g.moveTo(mx(p.px[k]), mz(p.pz[k]));
        pen = true;
      }
    };
    g.lineJoin = 'round'; g.lineCap = 'round';
    // A soft drop shadow, a dark kerb outline, the light road, then a colour line down the middle of each
    // branch (shortcuts dashed yellow, boost-only cuts dashed orange, alternative routes light blue).
    const order = [...track.paths.slice(1), track];
    for (const p of order) { trace(p); g.setLineDash([]); g.strokeStyle = 'rgba(0,0,0,0.35)'; g.lineWidth = (p.index === 0 ? 14 : 11) * MAP_DPR; g.stroke(); }
    for (const p of order) { trace(p); g.strokeStyle = 'rgba(20,22,28,0.9)'; g.lineWidth = (p.index === 0 ? 10 : 8) * MAP_DPR; g.stroke(); }
    for (const p of order) { trace(p); g.strokeStyle = p.index === 0 ? 'rgba(236,238,242,0.95)' : 'rgba(200,204,212,0.92)'; g.lineWidth = (p.index === 0 ? 6.5 : 5) * MAP_DPR; g.stroke(); }
    for (const p of track.paths.slice(1)) {
      trace(p);
      g.setLineDash(p.shortcut ? [4 * MAP_DPR, 4 * MAP_DPR] : []);
      g.strokeStyle = p.boost ? '#ff9a2e' : p.shortcut ? '#ffc23a' : '#5ab8f0'; g.lineWidth = 2 * MAP_DPR; g.stroke();
    }
    g.setLineDash([]);
    // Forks: a marker where each route splits off.
    for (const p of track.paths.slice(1)) {
      g.beginPath(); g.arc(mx(p.px[0]), mz(p.pz[0]), 4 * MAP_DPR, 0, Math.PI * 2);
      g.fillStyle = p.boost ? '#ff9a2e' : p.shortcut ? '#ffd65a' : '#8cd7ff'; g.fill();
      g.lineWidth = 1.5 * MAP_DPR; g.strokeStyle = '#111'; g.stroke();
    }
    // The start line: a little chequered bar across the road.
    pointAt(track, 0, 0, _mp);
    g.save(); g.translate(mx(_mp[0]), mz(_mp[2])); g.rotate(-_mp[3]);
    for (let k = -3; k < 3; k++) for (let j = 0; j < 2; j++) { g.fillStyle = (k + j) % 2 ? '#111' : '#fff'; g.fillRect(k * 2 * MAP_DPR, (j - 1) * 2 * MAP_DPR, 2 * MAP_DPR, 2 * MAP_DPR); }
    g.restore();
    mapLayer = { track: track.def.id, canvas, mx, mz };
    return mapLayer;
  }

  const SHELL_DOT: Record<string, string> = { green: '#3fd04a', red: '#ff4a3a', blue: '#4f8fff', bomb: '#ff9a2e' };
  function drawMinimap(state: KartState, mine: number): void {
    const c = minimap;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const track = trackFor(state.track);
    const w = c.width, h = c.height;
    const { canvas, mx, mz } = mapBase(track, w, h);
    g.clearRect(0, 0, w, h);
    g.drawImage(canvas, 0, 0);
    // Drawbridges: a bar across the road, red while the leaves are up.
    for (const p of track.paths) for (const d of p.features.drawbridges) {
      pointAt(p, (d.from + d.to) / 2 * p.length, 0, _mp);
      const ax = Math.cos(_mp[3]) * 6 * MAP_DPR, az = Math.sin(_mp[3]) * 6 * MAP_DPR; // across the road (the map mirrors X)
      g.beginPath(); g.moveTo(mx(_mp[0]) - ax, mz(_mp[2]) - az); g.lineTo(mx(_mp[0]) + ax, mz(_mp[2]) + az);
      g.lineWidth = 3 * MAP_DPR; g.strokeStyle = drawLift(d, state.raceTime) > 0.05 ? '#ff4d4d' : '#5ad17a'; g.stroke();
    }
    for (const b of state.shells) { g.fillStyle = SHELL_DOT[b.kind] ?? SHELL_DOT.bomb; g.beginPath(); g.arc(mx(b.x), mz(b.z), 3 * MAP_DPR, 0, Math.PI * 2); g.fill(); g.lineWidth = MAP_DPR; g.strokeStyle = '#111'; g.stroke(); }
    // Rivals as dots in their colours (leaders drawn on top), the viewer as an arrow pointing the way it drives.
    const order = [...state.racers].sort((a, b) => b.place - a.place);
    for (const r of order) {
      if (r.id === mine) continue;
      g.beginPath(); g.arc(mx(r.x), mz(r.z), 4.5 * MAP_DPR, 0, Math.PI * 2);
      g.fillStyle = racerDef(r.racer).color; g.fill();
      g.lineWidth = 1.5 * MAP_DPR; g.strokeStyle = 'rgba(0,0,0,0.7)'; g.stroke();
    }
    const me = state.racers[mine];
    if (me) {
      // Heading (sin yaw, cos yaw) on the X-mirrored map.
      const dx = -Math.sin(me.yaw), dz = Math.cos(me.yaw), px = mx(me.x), pz = mz(me.z), s = 8 * MAP_DPR;
      g.beginPath(); g.moveTo(px + dx * s, pz + dz * s); g.lineTo(px - dx * s * 0.6 - dz * s * 0.65, pz - dz * s * 0.6 + dx * s * 0.65); g.lineTo(px - dx * s * 0.3, pz - dz * s * 0.3); g.lineTo(px - dx * s * 0.6 + dz * s * 0.65, pz - dz * s * 0.6 - dx * s * 0.65); g.closePath();
      g.fillStyle = racerDef(me.racer).color; g.fill();
      g.lineWidth = 2 * MAP_DPR; g.strokeStyle = '#fff'; g.stroke();
    }
  }

  const _mp = [0, 0, 0, 0];

  /** Guest view: interpolate between buffered snapshots ~100 ms behind real time. */
  let view: KartState | null = null;
  function interpolated(now: number): KartState | null {
    const buf = link?.snapshots;
    if (!buf?.length) return null;
    const t = now - 100;
    let a = buf[0], b = buf[buf.length - 1];
    for (let i = buf.length - 1; i > 0; i--) if (buf[i - 1].at <= t) { a = buf[i - 1]; b = buf[i]; break; }
    const k = b.at === a.at ? 1 : Math.max(0, Math.min(1, (t - a.at) / (b.at - a.at)));
    if (!view || view.racers.length !== b.state.racers.length) view = structuredClone(b.state);
    Object.assign(view, b.state, { racers: view.racers });
    b.state.racers.forEach((rb, i) => {
      const ra = a.state.racers[i] ?? rb, out = view!.racers[i];
      Object.assign(out, rb);
      const jump = Math.hypot(rb.x - ra.x, rb.z - ra.z) > 25;
      if (!jump) {
        out.x = lerp(ra.x, rb.x, k); out.y = lerp(ra.y, rb.y, k); out.z = lerp(ra.z, rb.z, k);
        out.yaw = ra.yaw + wrapAngle(rb.yaw - ra.yaw) * k;
      }
    });
    return view;
  }

  function start(): void {
    stop?.();
    const abort = new AbortController();
    let disposed = false, frame = 0, last = performance.now(), lastHud = 0, lastSend = 0, lastEvent = 0, rouletteAt = 0, coinsSeen = 0;
    const bootAt = performance.now();
    const latch = { item: false, drift: false, respawn: false };
    let state: KartState | null = setup ? createRace(setup.track, setup.seats, setup.difficulty, setup.seed) : null;
    let pending: KartEvent[] = [];
    let finished = false;
    loading = true; error = ''; menu = false;
    const touch = window.matchMedia?.('(pointer: coarse)').matches ?? false;
    const boot = async (first: KartState) => {
      const { KartRenderer } = await import('./render');
      if (disposed) return;
      const created = new KartRenderer(stage, trackFor(first.track), first.racers.map(r => ({ id: r.id, racer: r.racer, human: r.human, name: r.name })), Math.max(0, first.racers.findIndex(r => r.human && r.slot === selfSlot)), abort.signal);
      try { await created.load(); } catch (e) { created.dispose(); throw e; }
      if (disposed) { created.dispose(); return; }
      renderer = created;
      input = new InputState(created.view.canvas, { pointerLock: false });
      created.view.canvas.addEventListener('focus', () => { active = true; });
      created.view.canvas.addEventListener('blur', () => { active = false; });
      created.view.canvas.focus();
      // Diagnostics for the E2E suite: state, render stats, and a camera pin for fixed-view captures.
      (stage as HTMLDivElement & { __ottoGame?: unknown }).__ottoGame = { snapshot: () => structuredClone(state ?? view), stats: () => created.stats(), pin: (pose: number[] | null) => { created.pin = pose; } };
      loading = false;
    };
    const tick = (now: number) => {
      if (disposed) return;
      frame = requestAnimationFrame(tick);
      const dt = Math.min(0.1, Math.max(0, (now - last) / 1000));
      last = now;
      if (isGuest) {
        const v = interpolated(now);
        if (v && !renderer && loading && !bootStarted) { bootStarted = true; boot(v).catch(fail); }
        state = v;
      }
      if (!state) return;
      const mineIndex = Math.max(0, state.racers.findIndex(r => r.human && r.slot === selfSlot));
      const mine = readInput(touch);
      if (!isGuest) {
        const inputs = state.racers.map(r => {
          if (!r.human) return undefined;
          if (r.slot === selfSlot) return mine;
          return link?.inputs?.get(r.slot)?.input;
        });
        // Guests may change their racer until the countdown ends.
        if (state.phase === 'intro' || state.phase === 'countdown') {
          for (const r of state.racers) if (r.human && r.slot !== selfSlot) { const c = link?.inputs?.get(r.slot)?.racer; if (c) r.racer = c; }
        }
        // Hold the intro until every guest has reported in (their racer pick
        // arrives with their first input), for at most twelve seconds.
        const waiting = state.phase === 'intro' && now - bootAt < 12_000 && state.racers.some(r => r.human && r.slot !== selfSlot && !link?.inputs?.has(r.slot));
        waitingFor = waiting ? state.racers.filter(r => r.human && r.slot !== selfSlot && !link?.inputs?.has(r.slot)).map(r => r.name) : [];
        if (!paused && !(menu && !link) && !waiting) stepRace(state, inputs, dt);
        pending.push(...state.events.filter(e => e.id > lastEvent));
      } else {
        // Latch taps (item, hop) so a press between two sends still reaches the host.
        latch.item ||= mine.item; latch.drift ||= mine.drift; latch.respawn ||= mine.respawn;
        if (now - lastSend > 33) {
          oninput?.({ ...mine, item: latch.item, drift: latch.drift, respawn: latch.respawn }, racer);
          latch.item = latch.drift = latch.respawn = false;
          lastSend = now;
        }
      }
      for (const e of state.events) if (e.id > lastEvent) { onEvent(state, e, mineIndex); lastEvent = e.id; }
      if (!isGuest && onframe && now - lastSend > 50) { onframe(state, pending); pending = []; lastSend = now; }
      if (renderer) {
        renderer.frame(state, dt, input?.down('KeyQ', 'KeyC', 'Back') ?? false);
        const me = state.racers[mineIndex];
        // Coins have no event of their own: chime when our purse grows.
        if (me.coins > coinsSeen) audio.play('pickup', { volume: 0.35, rate: 1.6 });
        coinsSeen = me.coins;
        audio.engineLevel(Math.abs(me.speed) / 30, me.boost > 0, !paused && !menu && state.phase !== 'done' && me.respawn <= 0);
      }
      input?.endFrame();
      if (now - lastHud > 90) {
        hud = { ...state, boxes: state.boxes.map(b => ({ ...b })), racers: state.racers.map(r => ({ ...r, lapTimes: [...r.lapTimes] })) };
        if (state.racers[mineIndex]?.roll > 0 && now - rouletteAt > 80) { rouletteIcon = (rouletteIcon + 1) % ROULETTE.length; rouletteAt = now; audio.play('tick', { volume: 0.3 }); }
        drawMinimap(state, mineIndex);
        underwater = renderer?.isUnderwater ?? false;
        lastHud = now;
      }
      if (state.phase === 'done' && !finished) { finished = true; input?.release(); onfinish?.(state); audio.play(state.racers[mineIndex].place === 1 ? 'voice-win' : 'voice-over'); }
    };
    let bootStarted = false;
    const fail = (e: unknown) => { if (!disposed) { loading = false; error = e instanceof Error ? e.message : 'The race could not start.'; } };
    if (state) { bootStarted = true; boot(state).catch(fail); }
    frame = requestAnimationFrame(tick);
    stop = () => {
      disposed = true; abort.abort(); cancelAnimationFrame(frame);
      input?.dispose(); input = undefined;
      renderer?.dispose(); renderer = undefined;
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
  function touchKey(e: PointerEvent, code: string, down: boolean) {
    e.preventDefault();
    if (down) { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); wake(); }
    input?.touch(code, down);
  }
  const results = $derived(hud && hud.phase === 'done' ? standings(hud) : []);
  const lapNow = $derived(self && hud ? Math.min(hud.laps, self.lap + 1) : 1);
  const best = $derived(self?.lapTimes.length ? Math.min(...self.lapTimes) : null);
</script>

<div class="kart-play game-play" bind:this={root} data-game="kart" data-track={setup?.track ?? hud?.track} data-phase={hud?.phase ?? 'loading'} onpointerdown={wake} role="presentation">
  <div class="kart-stage" bind:this={stage}></div>
  {#if underwater}<div class="kart-underwater" aria-hidden="true"></div>{/if}
  {#if loading && !error}<div class="kart-overlay" role="status"><h2>{isGuest && !hud ? 'Waiting for the host…' : 'Building the circuit…'}</h2><p>Loading karts, racers and scenery</p></div>{/if}
  {#if error}<div class="kart-overlay"><LoadState what="race" {error} empty onretry={start} /><button class="btn" onclick={onexit}>Back to games</button></div>{/if}
  {#snippet art(kind: ItemKind, count: number)}
    {@const a = itemArt(kind)}
    {#if a.kind === 'bolt'}<svg class="kart-art-bolt" viewBox="0 0 24 24" aria-hidden="true"><path d="M13 2 4 14h6l-1 8 9-12h-6z" /></svg>
    {:else if a.kind === 'shell'}
      <svg class="kart-art-shell" data-color={a.color} viewBox="0 0 48 40" aria-hidden="true">
        <path class="dome" d="M5 28a19 19 0 0 1 38 0z" /><path class="plates" d="M13 27l5-10h12l5 10M18 17l6-8 6 8" />
        {#if a.color === 'blue'}<path class="spikes" d="M16 12l-3-7 7 4M32 12l3-7-7 4M24 9V1" />{/if}
        <rect class="rim" x="2" y="27" width="44" height="8" rx="4" />
      </svg>
    {:else}
      <span class="kart-art-stack" data-count={Math.min(3, count)}>{#each Array.from({ length: Math.min(3, count) }) as _, i (i)}<img src={a.src} alt="" class:gold={a.gold} />{/each}</span>
    {/if}
  {/snippet}
  {#if hud && self && !loading && !error}
    <div class="kart-top">
      {#if self.boost > 0 && hud.phase === 'race'}<div class="kart-boostfx" aria-hidden="true"></div>{/if}
      {#if flash}{#key flash}<div class="kart-flash" data-kind={flashKind} aria-hidden="true"></div>{/key}{/if}
      <div class="kart-itemcol">
        <div class="kart-item" class:rolling={self.roll > 0} class:golden={self.golden > 0} style:--golden-left={`${Math.round(clamp(self.golden / GOLDEN_SECONDS, 0, 1) * 360)}deg`} aria-label="Item">
          <div class="kart-item-well">
            {#if self.roll > 0}{#key rouletteIcon}<span class="kart-reel">{@render art(ROULETTE[rouletteIcon], ROULETTE[rouletteIcon] === 'triple' ? 3 : 1)}</span>{/key}
            {:else if self.item}<span class="kart-reel settled">{@render art(self.item, self.item === 'triple' ? self.itemCount : 1)}</span>{#if self.itemCount > 1 && self.item !== 'triple'}<b>×{self.itemCount}</b>{/if}
            {:else}<span class="kart-item-empty">Item</span>{/if}
          </div>
        </div>
        {#if self.item && self.roll <= 0}<span class="kart-item-name">{ITEMS[self.item].name} · E</span>{/if}
      </div>
      <div class="kart-tools">
        <button class="btn small" aria-label={muted ? 'Unmute game' : 'Mute game'} title={muted ? 'Unmute game' : 'Mute game'} onclick={() => { muted = !muted; audio.mute(muted); }}>{muted ? 'Sound off' : 'Sound on'}</button>
        {#if typeof document !== 'undefined' && document.fullscreenEnabled}<button class="btn small" onclick={toggleFullscreen}>{fullscreen ? 'Exit full screen' : 'Full screen'}</button>{/if}
        <button class="btn small" onclick={() => { menu = !menu; }}>Menu</button>
      </div>
      <div class="kart-lap" aria-label="Lap">
        <div class="kart-coins" class:full={self.coins >= MAX_COINS} aria-label={`${self.coins} of ${MAX_COINS} coins`}><i aria-hidden="true"></i><strong>{self.coins}</strong><small>/{MAX_COINS}</small></div>
        <div class="kart-lapcount"><span>Lap</span><strong>{lapNow}<small>/{hud.laps}</small></strong></div>
        <time>{raceClock(hud.raceTime)}</time>
        {#if self.seg > 0}<em class="kart-route">{trackFor(hud.track).paths[self.seg]?.name ?? ''}</em>{/if}
        {#if self.lapTimes.length}<ol class="kart-laptimes">{#each self.lapTimes as t, i (i)}<li class:best={t === best}>L{i + 1} {raceClock(t)}</li>{/each}</ol>{/if}
      </div>
    </div>
    <ol class="kart-ranking" aria-label="Race positions">
      {#each [...hud.racers].sort((a, b) => a.place - b.place) as r (r.id)}
        <li class:me={r.id === selfIndex}><span class="kart-dot" style:background={racerDef(r.racer).color}></span><b>{r.place}</b><span class="kart-rname">{r.name}</span>{#if r.finish !== null}<em>✓</em>{/if}</li>
      {/each}
    </ol>
    <div class="kart-place" data-tier={placeTier(self.place)} aria-live="polite">{#key self.place}<strong>{self.place}</strong>{/key}<span>{ordinal(self.place).replace(/^\d+/, '')}</span></div>
    <canvas class="kart-minimap" bind:this={minimap} width={220 * MAP_DPR} height={220 * MAP_DPR} aria-label="Track map"></canvas>
    {#if self.drift !== 0}<div class="kart-drift" data-level={self.driftLevel} aria-hidden="true"><i></i><i></i><i></i></div>{/if}
    {#if notice}<div class="kart-notice" data-kind={noticeKind} aria-live="polite">{notice}</div>{/if}
    {#if self.wrong > 1 && hud.phase === 'race'}<div class="kart-wrong" role="alert">Wrong way!</div>{/if}
    {#if hud.phase === 'intro'}<div class="kart-intro"><h2>{trackFor(hud.track).def.name}</h2><p>{waitingFor.length ? `Waiting for ${waitingFor.join(', ')}…` : `${hud.racers.length} racers · ${hud.laps} laps · ${hud.difficulty} CPUs`}</p></div>{/if}
    {#if !active && hud.phase !== 'done' && !menu && !paused}<div class="kart-hint">Click the race to drive · W/S gas &amp; brake · A/D steer · Space drift / trick · E item · Q look back · R rescue</div>{/if}
    {#if paused && hud.phase !== 'done'}<div class="kart-overlay" role="status"><h2>Waiting for players</h2><p>The race is paused while someone reconnects.</p><button class="btn" onclick={onexit}>Leave race</button></div>
    {:else if hud.phase === 'done'}
      <div class="kart-overlay kart-results" aria-live="polite">
        <p class="kart-results-kicker">{trackFor(hud.track).def.name} · Results</p>
        <h2>{self.place === 1 ? 'You win!' : `You finished ${ordinal(self.place)}`}</h2>
        <table>
          <thead><tr><th scope="col">Pos</th><th scope="col">Racer</th><th scope="col">Time</th><th scope="col">Best lap</th></tr></thead>
          <tbody>{#each results as row (row.id)}<tr class:me={row.id === selfIndex}><td>{ordinal(row.place)}</td><td><img src={`/room-games/portraits/racer-${row.racer}.webp`} alt="" /><span>{row.name}{row.human ? '' : ' (CPU)'}</span></td><td>{raceClock(row.time)}</td><td>{row.best ? raceClock(row.best) : '—'}</td></tr>{/each}</tbody>
        </table>
        <div class="kart-actions"><button class="btn primary" disabled={rematchDisabled} onclick={onrematch}>{rematchLabel}</button><button class="btn" onclick={onexit}>Back to games</button></div>
      </div>
    {:else if menu}
      <div class="kart-overlay"><h2>Paused</h2><p>{link ? 'The race continues for everyone else while this menu is open.' : 'Take a breather.'}</p>
        <p class="kart-keys">W / S — gas &amp; brake · A / D — steer · Space — hop, drift (release for a mini-turbo), trick in the air · E — use item · Q — look back · R — rescue when stuck</p>
        <button class="btn primary" onclick={() => { menu = false; renderer?.view.canvas.focus(); }}>Resume</button><button class="btn" onclick={onexit}>Leave race</button></div>
    {/if}
    <div class="kart-touch" aria-label="Touch controls">
      <div><button aria-label="Steer left" onpointerdown={e => touchKey(e, 'ArrowLeft', true)} onpointerup={e => touchKey(e, 'ArrowLeft', false)} onpointercancel={e => touchKey(e, 'ArrowLeft', false)}>◀</button><button aria-label="Steer right" onpointerdown={e => touchKey(e, 'ArrowRight', true)} onpointerup={e => touchKey(e, 'ArrowRight', false)} onpointercancel={e => touchKey(e, 'ArrowRight', false)}>▶</button></div>
      <div><button onpointerdown={e => touchKey(e, 'ArrowDown', true)} onpointerup={e => touchKey(e, 'ArrowDown', false)} onpointercancel={e => touchKey(e, 'ArrowDown', false)}>Brake</button><button onpointerdown={e => touchKey(e, 'Drift', true)} onpointerup={e => touchKey(e, 'Drift', false)} onpointercancel={e => touchKey(e, 'Drift', false)}>Drift</button><button onpointerdown={e => touchKey(e, 'Item', true)} onpointerup={e => touchKey(e, 'Item', false)} onpointercancel={e => touchKey(e, 'Item', false)}>Item</button></div>
    </div>
  {/if}
</div>

<style>
  .kart-play { position: relative; flex: 1; min-height: 420px; overflow: hidden; background: var(--surface); isolation: isolate; user-select: none; }
  .kart-play:fullscreen { width: 100vw; height: 100vh; }
  .kart-stage { position: absolute; inset: 0; }
  .kart-stage :global(canvas) { display: block; width: 100%; height: 100%; }
  .kart-stage :global(canvas:focus-visible) { outline: 2px solid var(--accent-line-strong); outline-offset: -2px; }
  .kart-underwater { position: absolute; inset: 0; pointer-events: none; background: radial-gradient(ellipse at center, color-mix(in srgb, var(--info) 8%, transparent), color-mix(in srgb, var(--info) 30%, transparent)); }
  .kart-overlay { position: absolute; inset: 0; z-index: 6; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 24px; text-align: center; background: color-mix(in srgb, var(--surface) 90%, transparent); }
  .kart-overlay h2 { margin: 0; font-size: var(--fs-2xl); }
  .kart-overlay p { margin: 0; max-width: 560px; color: var(--text-dim); line-height: 1.6; }
  /* HUD frame: the item box top-left, tools top-right, lap / coins / time above the minimap. */
  .kart-top { position: absolute; inset: 14px; pointer-events: none; }
  .kart-lap, .kart-tools, .kart-ranking, .kart-minimap { background: var(--scrim-media); color: var(--on-scrim); border: 1px solid color-mix(in srgb, var(--on-scrim) 18%, transparent); border-radius: var(--radius-l); }
  .kart-tools { position: absolute; inset-block-start: 0; inset-inline-end: 0; display: flex; gap: 6px; padding: 6px; pointer-events: auto; }
  .kart-tools :global(.btn) { background: color-mix(in srgb, var(--on-scrim) 14%, transparent); color: var(--on-scrim); border-color: color-mix(in srgb, var(--on-scrim) 25%, transparent); }
  /* Item box: a chunky double-ringed square; a gold timer ring runs down through a golden-mushroom window. */
  .kart-itemcol { position: absolute; inset-block-start: 0; inset-inline-start: 0; display: flex; flex-direction: column; align-items: center; gap: 6px; }
  .kart-item { --golden-left: 0deg; width: 112px; height: 112px; padding: 6px; border-radius: var(--radius-l); background: linear-gradient(160deg, color-mix(in srgb, var(--on-scrim) 92%, var(--info)), color-mix(in srgb, var(--on-scrim) 55%, var(--info))); box-shadow: 0 6px 18px rgba(0,0,0,0.35), inset 0 -4px 0 rgba(0,0,0,0.18); position: relative; }
  .kart-item.golden { background: conic-gradient(var(--warning) var(--golden-left), color-mix(in srgb, var(--warning) 25%, var(--scrim-media)) 0); }
  .kart-item-well { position: relative; width: 100%; height: 100%; border-radius: var(--radius-l); display: grid; place-items: center; overflow: hidden; background: radial-gradient(circle at 50% 40%, color-mix(in srgb, var(--info) 35%, var(--scrim-media)), var(--scrim-media)); box-shadow: inset 0 3px 8px rgba(0,0,0,0.45); }
  .kart-item.rolling .kart-item-well { background: radial-gradient(circle at 50% 40%, color-mix(in srgb, var(--cat-5) 40%, var(--scrim-media)), var(--scrim-media)); }
  .kart-reel { display: grid; place-items: center; width: 78px; height: 78px; animation: kart-reel 80ms linear; }
  .kart-reel.settled { animation: kart-settle 260ms cubic-bezier(.2, 1.6, .4, 1); }
  @keyframes kart-reel { from { transform: translateY(-60%); opacity: .4; } to { transform: none; opacity: 1; } } /* ui-guards: allow — in-game HUD motion */
  @keyframes kart-settle { from { transform: scale(1.35); } to { transform: none; } } /* ui-guards: allow — in-game HUD motion */
  .kart-item b { position: absolute; inset-block-end: 6px; inset-inline-end: 10px; font-size: var(--fs-xl); font-style: italic; color: var(--on-scrim); text-shadow: 0 2px 4px rgba(0,0,0,0.6); }
  .kart-item-name { white-space: nowrap; font-size: var(--fs-s); color: var(--on-scrim); background: var(--scrim-media); padding: 2px 10px; border-radius: var(--radius-s); }
  .kart-item-empty { font-size: var(--fs-s); color: var(--on-scrim); opacity: .6; text-transform: uppercase; letter-spacing: .06em; }
  .kart-art-stack { position: relative; display: grid; place-items: center; width: 100%; height: 100%; }
  .kart-art-stack img { width: 72px; height: 72px; object-fit: contain; filter: drop-shadow(0 3px 3px rgba(0,0,0,0.45)); }
  .kart-art-stack img.gold { filter: sepia(1) saturate(5) hue-rotate(5deg) brightness(1.15) drop-shadow(0 0 8px color-mix(in srgb, var(--warning) 80%, transparent)); }
  .kart-art-stack[data-count='2'] img, .kart-art-stack[data-count='3'] img { position: absolute; width: 46px; height: 46px; }
  .kart-art-stack[data-count='2'] img:nth-child(1) { transform: translate(-12px, -8px); }
  .kart-art-stack[data-count='2'] img:nth-child(2) { transform: translate(12px, 8px); }
  .kart-art-stack[data-count='3'] img:nth-child(1) { transform: translate(-14px, 10px); }
  .kart-art-stack[data-count='3'] img:nth-child(2) { transform: translate(14px, 10px); }
  .kart-art-stack[data-count='3'] img:nth-child(3) { transform: translate(0, -12px); }
  .kart-art-bolt { width: 70px; height: 70px; fill: var(--warning); stroke: var(--on-scrim); stroke-width: 1; filter: drop-shadow(0 0 6px color-mix(in srgb, var(--warning) 70%, transparent)); }
  .kart-art-shell { width: 74px; height: 62px; filter: drop-shadow(0 3px 3px rgba(0,0,0,0.45)); }
  .kart-art-shell .dome { fill: var(--success); stroke: rgba(0,0,0,0.5); stroke-width: 1.5; }
  .kart-art-shell[data-color='red'] .dome { fill: var(--danger); }
  .kart-art-shell[data-color='blue'] .dome { fill: var(--info); }
  .kart-art-shell .plates { fill: none; stroke: var(--on-scrim); stroke-width: 2; stroke-linejoin: round; opacity: .85; }
  .kart-art-shell .spikes { fill: none; stroke: var(--on-scrim); stroke-width: 3; stroke-linecap: round; }
  .kart-art-shell .rim { fill: var(--on-scrim); stroke: rgba(0,0,0,0.45); stroke-width: 1.5; }
  /* Lap, coins and race time: bottom-left, stacked over the minimap. */
  .kart-lap { position: absolute; inset-inline-start: 0; inset-block-end: 214px; padding: 8px 14px 10px; display: grid; grid-template-columns: auto auto; column-gap: 14px; row-gap: 2px; align-items: center; min-width: 200px; }
  .kart-coins { display: flex; align-items: baseline; gap: 4px; font-variant-numeric: tabular-nums; }
  .kart-coins i { align-self: center; width: 22px; height: 22px; border-radius: 50%; background: radial-gradient(circle at 35% 30%, color-mix(in srgb, var(--warning) 40%, var(--on-scrim)), var(--warning) 55%, color-mix(in srgb, var(--warning) 60%, var(--danger))); box-shadow: inset 0 0 0 3px color-mix(in srgb, var(--warning) 55%, var(--scrim-media)); }
  .kart-coins strong { font-size: var(--fs-2xl); font-style: italic; line-height: 1; }
  .kart-coins small { font-size: var(--fs-s); opacity: .75; }
  .kart-coins.full strong { color: var(--warning); }
  .kart-lapcount { display: flex; align-items: baseline; gap: 6px; justify-self: end; }
  .kart-lapcount span { font-size: var(--fs-s); text-transform: uppercase; letter-spacing: .06em; opacity: .85; font-weight: 600; }
  .kart-lapcount strong { font-size: calc(var(--fs-hero) * 1.05); font-style: italic; line-height: 1; }
  .kart-lapcount small { font-size: var(--fs-l); opacity: .8; }
  .kart-lap time { grid-column: 1 / -1; font-variant-numeric: tabular-nums; font-size: var(--fs-l); }
  .kart-route { grid-column: 1 / -1; font-style: normal; font-size: var(--fs-s); color: var(--warning); text-transform: uppercase; letter-spacing: .06em; }
  .kart-laptimes { grid-column: 1 / -1; list-style: none; margin: 2px 0 0; padding: 0; font-size: var(--fs-s); font-variant-numeric: tabular-nums; opacity: .9; }
  .kart-laptimes li.best { color: var(--success); font-weight: 600; }
  .kart-ranking { position: absolute; inset-inline-end: 14px; inset-block-start: 80px; list-style: none; margin: 0; padding: 8px; display: grid; gap: 2px; min-width: 170px; font-size: var(--fs-s); pointer-events: none; }
  .kart-ranking li { display: flex; align-items: center; gap: 8px; padding: 4px 6px; border-radius: var(--radius-s); }
  .kart-ranking li.me { background: color-mix(in srgb, var(--on-scrim) 22%, transparent); font-weight: 600; }
  .kart-ranking b { width: 14px; text-align: end; font-variant-numeric: tabular-nums; }
  .kart-rname { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 120px; }
  .kart-ranking em { font-style: normal; color: var(--success); }
  .kart-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; border: 1px solid color-mix(in srgb, var(--on-scrim) 50%, transparent); }
  /* Position badge: a huge outlined italic numeral in its medal colour, popping on every change of place. */
  .kart-place { --medal: var(--info); position: absolute; inset-inline-end: 22px; inset-block-end: 14px; display: flex; align-items: baseline; pointer-events: none; color: var(--medal); font-weight: 600; font-style: italic; -webkit-text-stroke: 5px rgba(0,0,0,0.7); paint-order: stroke fill; text-shadow: 0 6px 14px rgba(0,0,0,0.45); }
  .kart-place[data-tier='gold'] { --medal: var(--warning); }
  .kart-place[data-tier='silver'] { --medal: color-mix(in srgb, var(--on-scrim) 82%, var(--info)); }
  .kart-place[data-tier='bronze'] { --medal: color-mix(in srgb, var(--warning) 55%, var(--danger)); }
  .kart-place strong { font-size: calc(var(--fs-hero) * 3.2); line-height: .9; display: inline-block; animation: kart-pop 380ms cubic-bezier(.2, 1.7, .4, 1); }
  .kart-place span { font-size: calc(var(--fs-hero) * 1.1); margin-inline-start: 2px; }
  @keyframes kart-pop { from { transform: scale(1.6) rotate(-6deg); } to { transform: none; } } /* ui-guards: allow — in-game HUD motion */
  .kart-minimap { position: absolute; inset-inline-start: 14px; inset-block-end: 14px; width: 200px; height: 200px; padding: 0; pointer-events: none; }
  /* Boost: a warm rush at the screen edges. Lightning: a white (or, when struck, electric-blue) flash. */
  .kart-boostfx { position: absolute; inset: -14px; background: radial-gradient(ellipse at center, transparent 55%, color-mix(in srgb, var(--warning) 22%, transparent) 85%, color-mix(in srgb, var(--danger) 30%, transparent)); animation: kart-rush 600ms ease-out; }
  @keyframes kart-rush { from { opacity: 0; transform: scale(1.06); } to { opacity: 1; transform: none; } } /* ui-guards: allow — in-game HUD motion */
  .kart-flash { position: absolute; inset: -14px; background: var(--on-scrim); opacity: 0; animation: kart-flash 520ms ease-out; }
  .kart-flash[data-kind='zap'] { background: color-mix(in srgb, var(--info) 55%, var(--on-scrim)); }
  @keyframes kart-flash { 0% { opacity: .75; } 30% { opacity: .2; } 45% { opacity: .5; } 100% { opacity: 0; } } /* ui-guards: allow — in-game HUD motion */
  @media (prefers-reduced-motion: reduce) { .kart-reel, .kart-reel.settled, .kart-place strong, .kart-boostfx { animation: none; } .kart-flash { animation-duration: 1ms; } }
  .kart-drift { position: absolute; inset-block-end: 26px; inset-inline-start: 50%; transform: translateX(-50%); display: flex; gap: 6px; pointer-events: none; }
  .kart-drift i { width: 34px; height: 10px; border-radius: var(--radius-s); background: color-mix(in srgb, var(--on-scrim) 25%, transparent); }
  .kart-drift[data-level='1'] i:nth-child(1), .kart-drift[data-level='2'] i:nth-child(-n+2), .kart-drift[data-level='3'] i { background: var(--info); }
  .kart-drift[data-level='2'] i:nth-child(-n+2) { background: var(--warning); }
  .kart-drift[data-level='3'] i { background: var(--cat-5); }
  .kart-notice { position: absolute; inset-block-start: 28%; inset-inline: 12px; text-align: center; font-size: clamp(26px, 4.4vw, 48px); font-weight: 600; font-style: italic; color: var(--on-scrim); text-shadow: 0 3px 10px var(--scrim-media); pointer-events: none; }
  .kart-notice[data-kind='count'] { font-size: clamp(64px, 12vw, 140px); inset-block-start: 22%; }
  .kart-notice[data-kind='small'] { font-size: clamp(18px, 2.6vw, 28px); inset-block-start: 62%; }
  .kart-notice[data-kind='bad'] { color: var(--danger); }
  .kart-notice[data-kind='good'] { color: var(--warning); }
  .kart-notice[data-kind='info'] { font-size: clamp(16px, 2.2vw, 24px); inset-block-start: 18%; }
  .kart-wrong { position: absolute; inset-block-start: 40%; inset-inline: 0; text-align: center; font-size: var(--fs-2xl); font-weight: 600; color: var(--on-scrim); background: color-mix(in srgb, var(--danger) 70%, transparent); padding: 8px; pointer-events: none; }
  .kart-intro { position: absolute; inset-block-end: 22%; inset-inline: 0; text-align: center; color: var(--on-scrim); text-shadow: 0 2px 8px var(--scrim-media); pointer-events: none; }
  .kart-intro h2 { margin: 0; font-size: clamp(28px, 5vw, 56px); font-style: italic; }
  .kart-intro p { margin: 4px 0 0; font-size: var(--fs-l); text-transform: capitalize; }
  .kart-hint { position: absolute; inset-block-end: 236px; inset-inline: 20px; margin-inline: auto; max-width: 640px; text-align: center; background: var(--scrim-media); color: var(--on-scrim); border-radius: var(--radius-s); padding: 8px; font-size: var(--fs-s); pointer-events: none; }
  .kart-results table { border-collapse: collapse; min-width: min(560px, 100%); font-size: var(--fs-m); font-variant-numeric: tabular-nums; }
  .kart-results th { text-align: start; font-size: var(--fs-s); color: var(--text-dim); font-weight: 500; padding: 4px 10px; }
  .kart-results td { padding: 4px 10px; border-block-start: 1px solid var(--border); text-align: start; }
  .kart-results td:nth-child(2) { display: flex; align-items: center; gap: 8px; }
  .kart-results td img { width: 34px; height: 34px; object-fit: contain; }
  .kart-results tr.me td { background: var(--accent-soft); font-weight: 600; }
  .kart-results-kicker { text-transform: uppercase; letter-spacing: .06em; font-size: var(--fs-s); }
  .kart-actions { display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
  .kart-keys { font-size: var(--fs-s); }
  .kart-touch { display: none; }
  @media (max-width: 640px) {
    .kart-ranking { display: none; } .kart-minimap { width: 130px; height: 130px; } .kart-lap { min-width: 0; inset-block-end: 144px; } .kart-tools { flex-wrap: wrap; justify-content: flex-end; max-width: 60%; }
    .kart-item { width: 84px; height: 84px; } .kart-reel { width: 58px; height: 58px; } .kart-place strong { font-size: calc(var(--fs-hero) * 2); }
  }
  @media (pointer: coarse) {
    .kart-touch { position: absolute; inset-inline: 12px; inset-block-end: 150px; display: flex; justify-content: space-between; gap: 8px; }
    .kart-touch div { display: flex; gap: 8px; }
    .kart-touch button { touch-action: none; min-width: 64px; min-height: 56px; border-radius: var(--radius-m); border: 1px solid var(--border); background: color-mix(in srgb, var(--surface) 80%, transparent); color: var(--text); font-size: var(--fs-m); }
    .kart-hint { display: none; }
  }
</style>
