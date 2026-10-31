///<reference types="svelte" />
;
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
;;

import { onMount, tick } from 'svelte';
import LoadState from '../../../../lib/components/LoadState.svelte';
import { createRace, GOLDEN_SECONDS, idleInput, MAX_COINS, standings, stepRace, type KartEvent } from './sim';
import { drawLift, F_GAP, pointAt, trackFor, type Path, type Track } from './tracks';
import { racerDef } from './roster';
import { ITEMS, type ItemKind } from './items';
import { itemArt, noticeFor, placeTier, ROULETTE } from './hud';
import { InputState } from '../common/input';
import { lossBudget } from '../../../../lib/three/stage';
import { GameAudio } from '../common/audio';
import { clamp, ordinal, raceClock, lerp, wrapAngle } from '../common/math';
import type { KartRenderer } from './render';
;

  interface Props {
    setup: KartSetup | null; selfSlot: number; racer: RacerId; link?: KartLink;
    names?: (slot: number) => string; paused?: boolean;
    onframe?: (state: KartState, events: KartEvent[]) => void;
    oninput?: (input: KartInput, racer: RacerId) => void;
    onfinish?: (state: KartState) => void;
    onexit: () => void; onrematch: () => void; rematchLabel?: string; rematchDisabled?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { setup, selfSlot, racer, link, names = () => 'Player', paused = false, onframe, oninput, onfinish, onexit, onrematch, rematchLabel = 'Race again', rematchDisabled = false }: Props = $props();

  let stage: HTMLDivElement, root: HTMLDivElement;
  let minimap = $state<HTMLCanvasElement>();
  let loading = $state(true), error = $state(''), menu = $state(false), muted = $state(false), fullscreen = $state(false), active = $state(false);
  let hud = $state.raw<KartState | null>(null);
  let notice = $state(''), noticeKind = $state('');
  /** The one always-mounted screen-reader status: edges only (round start, go, call-outs,
   *  waiting, the result headline). Countdown digits and the visual notice stay aria-hidden. */
  let srStatus = $state('');
  let resumeButton = $state<HTMLButtonElement>();
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
    // Digits would be re-read every second; the countdown's first beat and GO are the edges.
    if (kind !== 'count') srStatus = text;
    else if (text === '3') srStatus = 'Race starts in 3 seconds';
    else if (text === 'GO!') srStatus = 'Go!';
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
    /** Context-loss remounts left (lib/three/stage.ts: twice a minute, then Retry). */
    const losses = lossBudget();
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
      // Escape on the race opens the menu (its Resume button takes focus).
      created.view.canvas.addEventListener('keydown', e => { if (e.key === 'Escape' && !menu && !finished) { e.preventDefault(); menu = true; } });
      created.view.canvas.focus();
      // A reclaimed GPU context remounts the renderer on the live race state
      // instead of leaving a black canvas; a third loss within a minute asks for Retry.
      created.view.onContextLost(() => {
        if (disposed || renderer !== created) return;
        input?.dispose(); input = undefined;
        renderer = undefined; created.dispose();
        if (!losses.allow()) { fail(new Error('The race lost its graphics context (too many 3D views open at once).')); return; }
        loading = true;
        setTimeout(() => { const s = state; if (!disposed && s) boot(s).catch(fail); }, 600);
      });
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
    const fail = (e: unknown) => { if (!disposed) { loading = false; error = e instanceof Error ? e.message : 'The race couldn’t start.'; } };
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
  // The Paused overlay covers the Menu button: move focus onto its Resume button.
  $effect(() => { const b = resumeButton; if (b) void tick().then(() => b.focus()); });
  function resume() { menu = false; renderer?.view.canvas.focus(); }
  const headline = $derived(hud?.phase === 'done' && self ? (self.place === 1 ? 'You win!' : `You finished ${ordinal(self.place)}`) : '');
  $effect(() => { if (headline) srStatus = headline; });
  $effect(() => { if (paused && hud && hud.phase !== 'done') srStatus = 'Race paused while someone reconnects.'; });
  // The overlays and the wrong-way banner are plain visuals; the always-mounted
  // `srStatus` region speaks their edges (a region inserted with its text is
  // silent in VoiceOver).
  const loadingLine = $derived(loading && !error ? (isGuest && !hud ? 'Waiting for the host…' : 'Building the circuit…') : '');
  $effect(() => { if (loadingLine) srStatus = loadingLine; });
  const wrongWay = $derived(!!self && self.wrong > 1 && hud?.phase === 'race');
  $effect(() => { if (wrongWay) srStatus = 'Wrong way!'; });

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
;
async () => {


 { const $$_div0 = svelteHTML.createElement("div", {            "class":`kart-play game-play`,...__sveltets_2_empty({"data-game":`kart`}),...__sveltets_2_empty({"data-track":setup?.track ?? hud?.track}),...__sveltets_2_empty({"data-phase":hud?.phase ?? 'loading'}),"onpointerdown":wake,"role":`presentation`,});root = $$_div0;
   const art/*Ωignore_positionΩ*/ = (kind: ItemKind, count: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
    const a = itemArt(kind);
    if(a.kind === 'bolt'){ { svelteHTML.createElement("svg", {     "class":`kart-art-bolt`,"viewBox":`0 0 24 24`,"aria-hidden":`true`,}); { svelteHTML.createElement("path", {  "d":`M13 2 4 14h6l-1 8 9-12h-6z`,});} }
    } else if (a.kind === 'shell'){
       { svelteHTML.createElement("svg", {       "class":`kart-art-shell`,...__sveltets_2_empty({"data-color":a.color}),"viewBox":`0 0 48 40`,"aria-hidden":`true`,});
         { svelteHTML.createElement("path", {    "class":`dome`,"d":`M5 28a19 19 0 0 1 38 0z`,});} { svelteHTML.createElement("path", {    "class":`plates`,"d":`M13 27l5-10h12l5 10M18 17l6-8 6 8`,});}
        if(a.color === 'blue'){ { svelteHTML.createElement("path", {    "class":`spikes`,"d":`M16 12l-3-7 7 4M32 12l3-7-7 4M24 9V1`,});}}
         { svelteHTML.createElement("rect", {            "class":`rim`,"x":`2`,"y":`27`,"width":`44`,"height":`8`,"rx":`4`,});}
       }
    }else{
       { svelteHTML.createElement("span", {   "class":`kart-art-stack`,...__sveltets_2_empty({"data-count":Math.min(3, count)}),});    for(let _ of __sveltets_2_ensureArray((Array.from({ length: Math.min(3, count) })))){let i = 1;i; { svelteHTML.createElement("img", {    "src":a.src,"alt":"",});a.gold;}} }
    }
  };return __sveltets_2_any(0)}; { const $$_div1 = svelteHTML.createElement("div", {  "class":`kart-stage`,});stage = $$_div1; }
   { svelteHTML.createElement("p", {   "class":`sr-only`,"role":`status`,});srStatus; }
  if(underwater){ { svelteHTML.createElement("div", {   "class":`kart-underwater`,"aria-hidden":`true`,}); }}
  if(loadingLine){ { svelteHTML.createElement("div", { "class":`kart-overlay`,}); { svelteHTML.createElement("h2", {});loadingLine; } { svelteHTML.createElement("p", {});     } }}
  if(error){ { svelteHTML.createElement("div", { "class":`kart-overlay`,}); { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`race`,error,"empty":true,"onretry":start,}});} { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});   } }}
  
  if(hud && self && !loading && !error){
     { svelteHTML.createElement("div", { "class":`kart-top`,});
      if(self.boost > 0 && hud.phase === 'race'){ { svelteHTML.createElement("div", {   "class":`kart-boostfx`,"aria-hidden":`true`,}); }}
      if(flash){flash; { { svelteHTML.createElement("div", {     "class":`kart-flash`,...__sveltets_2_empty({"data-kind":flashKind}),"aria-hidden":`true`,}); }}}
       { svelteHTML.createElement("div", { "class":`kart-itemcol`,});
         { svelteHTML.createElement("div", {      "class":`kart-item`,"aria-label":`Item`,});self.roll > 0;self.golden > 0;__sveltets_2_ensureType(String, Number, `${Math.round(clamp(self.golden / GOLDEN_SECONDS, 0, 1) * 360)}deg`);
           { svelteHTML.createElement("div", { "class":`kart-item-well`,});
            if(self.roll > 0){rouletteIcon; { { svelteHTML.createElement("span", { "class":`kart-reel`,});;__sveltets_2_ensureSnippet(art(ROULETTE[rouletteIcon], ROULETTE[rouletteIcon] === 'triple' ? 3 : 1)); }}
            } else if (self.item){ { svelteHTML.createElement("span", { "class":`kart-reel settled`,});;__sveltets_2_ensureSnippet(art(self.item, self.item === 'triple' ? self.itemCount : 1)); }if(self.itemCount > 1 && self.item !== 'triple'){ { svelteHTML.createElement("b", {}); self.itemCount; }}
            }else{ { svelteHTML.createElement("span", { "class":`kart-item-empty`,});  }}
           }
         }
        if(self.item && self.roll <= 0){ { svelteHTML.createElement("span", { "class":`kart-item-name`,});ITEMS[self.item].name;   }}
       }
       { svelteHTML.createElement("div", { "class":`kart-tools`,});
         { svelteHTML.createElement("button", {       "class":`btn small`,"aria-label":muted ? 'Unmute game' : 'Mute game',"title":muted ? 'Unmute game' : 'Mute game',"onclick":() => { muted = !muted; audio.mute(muted); },});muted ? 'Sound off' : 'Sound on'; }
        if(typeof document !== 'undefined' && document.fullscreenEnabled){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":toggleFullscreen,});fullscreen ? 'Exit full screen' : 'Full screen'; }}
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { menu = !menu; },});  }
       }
       { svelteHTML.createElement("div", {   "class":`kart-lap`,"aria-label":`Lap`,});
         { svelteHTML.createElement("div", {    "class":`kart-coins`,"aria-label":`${self.coins} of ${MAX_COINS} coins`,});self.coins >= MAX_COINS; { svelteHTML.createElement("i", { "aria-hidden":`true`,}); } { svelteHTML.createElement("strong", {});self.coins; } { svelteHTML.createElement("small", {}); MAX_COINS; } }
         { svelteHTML.createElement("div", { "class":`kart-lapcount`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("strong", {});lapNow; { svelteHTML.createElement("small", {}); hud.laps; } } }
         { svelteHTML.createElement("time", {});raceClock(hud.raceTime); }
        if(self.seg > 0){ { svelteHTML.createElement("em", { "class":`kart-route`,});trackFor(hud.track).paths[self.seg]?.name ?? ''; }}
        if(self.lapTimes.length){ { svelteHTML.createElement("ol", { "class":`kart-laptimes`,});    for(let t of __sveltets_2_ensureArray(self.lapTimes)){let i = 1;i; { svelteHTML.createElement("li", { });t === best; i + 1; raceClock(t); }} }}
       }
     }
     { svelteHTML.createElement("ol", {   "class":`kart-ranking`,"aria-label":`Race positions`,});
         for(let r of __sveltets_2_ensureArray(([...hud.racers].sort((a, b) => a.place - b.place)))){r.id;
         { svelteHTML.createElement("li", { });r.id === selfIndex; { svelteHTML.createElement("span", {  "class":`kart-dot`,});__sveltets_2_ensureType(String, Number, racerDef(r.racer).color); } { svelteHTML.createElement("b", {});r.place; } { svelteHTML.createElement("span", { "class":`kart-rname`,});r.name; }if(r.finish !== null){ { svelteHTML.createElement("em", {});  }} }
      }
     }
     { svelteHTML.createElement("div", {     "class":`kart-place`,...__sveltets_2_empty({"data-tier":placeTier(self.place)}),"aria-live":`polite`,});self.place; { { svelteHTML.createElement("strong", {});self.place; }} { svelteHTML.createElement("span", {});ordinal(self.place).replace(/^\d+/, ''); } }
     { const $$_canvas1 = svelteHTML.createElement("canvas", {        "class":`kart-minimap`,"width":220 * MAP_DPR,"height":220 * MAP_DPR,"aria-label":`Track map`,});minimap = $$_canvas1; }
    if(self.drift !== 0){ { svelteHTML.createElement("div", {     "class":`kart-drift`,...__sveltets_2_empty({"data-level":self.driftLevel}),"aria-hidden":`true`,}); { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } }}
    if(notice){ { svelteHTML.createElement("div", {     "class":`kart-notice`,...__sveltets_2_empty({"data-kind":noticeKind}),"aria-hidden":`true`,});notice; }}
    if(wrongWay){ { svelteHTML.createElement("div", {   "class":`kart-wrong`,"aria-hidden":`true`,});  }}
    if(hud.phase === 'intro'){ { svelteHTML.createElement("div", { "class":`kart-intro`,}); { svelteHTML.createElement("h2", {});trackFor(hud.track).def.name; } { svelteHTML.createElement("p", {});waitingFor.length ? `Waiting for ${waitingFor.join(', ')}…` : `${hud.racers.length} racers · ${hud.laps} laps · ${hud.difficulty} CPUs`; } }}
    if(!active && hud.phase !== 'done' && !menu && !paused){ { svelteHTML.createElement("div", { "class":`kart-hint`,});                            }}
    if(paused && hud.phase !== 'done'){ { svelteHTML.createElement("div", { "class":`kart-overlay`,}); { svelteHTML.createElement("h2", {});   } { svelteHTML.createElement("p", {});       } { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});  } }
    } else if (hud.phase === 'done'){
       { svelteHTML.createElement("div", { "class":`kart-overlay kart-results`,});
         { svelteHTML.createElement("p", { "class":`kart-results-kicker`,});trackFor(hud.track).def.name;   }
         { svelteHTML.createElement("h2", {});headline; }
         { svelteHTML.createElement("table", {});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } } }
           { svelteHTML.createElement("tbody", {});   for(let row of __sveltets_2_ensureArray(results)){row.id; { svelteHTML.createElement("tr", { });row.id === selfIndex; { svelteHTML.createElement("td", {});ordinal(row.place); } { svelteHTML.createElement("td", {});  { svelteHTML.createElement("img", {  "src":`/room-games/portraits/racer-${row.racer}.webp`,"alt":"",});} { svelteHTML.createElement("span", {});row.name;row.human ? '' : ' (CPU)'; } } { svelteHTML.createElement("td", {});raceClock(row.time); } { svelteHTML.createElement("td", {});row.best ? raceClock(row.best) : '—'; } }} }
         }
         { svelteHTML.createElement("div", { "class":`kart-actions`,}); { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":rematchDisabled,"onclick":onrematch,});rematchLabel; } { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});   } }
       }
    } else if (menu){
       { svelteHTML.createElement("div", { "class":`kart-overlay`,}); { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("p", {});link ? 'The race continues for everyone else while this menu is open.' : 'Take a breather.'; }
         { svelteHTML.createElement("p", { "class":`kart-keys`,});                                          }
         { const $$_button2 = svelteHTML.createElement("button", {    "class":`btn primary`,"onclick":resume,});resumeButton = $$_button2;  } { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});  } }
    }
     { svelteHTML.createElement("div", {   "class":`kart-touch`,"aria-label":`Touch controls`,});
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("button", {       "aria-label":`Steer left`,"onpointerdown":e => touchKey(e, 'ArrowLeft', true),"onpointerup":e => touchKey(e, 'ArrowLeft', false),"onpointercancel":e => touchKey(e, 'ArrowLeft', false),});  } { svelteHTML.createElement("button", {       "aria-label":`Steer right`,"onpointerdown":e => touchKey(e, 'ArrowRight', true),"onpointerup":e => touchKey(e, 'ArrowRight', false),"onpointercancel":e => touchKey(e, 'ArrowRight', false),});  } }
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("button", {     "onpointerdown":e => touchKey(e, 'ArrowDown', true),"onpointerup":e => touchKey(e, 'ArrowDown', false),"onpointercancel":e => touchKey(e, 'ArrowDown', false),});  } { svelteHTML.createElement("button", {     "onpointerdown":e => touchKey(e, 'Drift', true),"onpointerup":e => touchKey(e, 'Drift', false),"onpointercancel":e => touchKey(e, 'Drift', false),});  } { svelteHTML.createElement("button", {     "onpointerdown":e => touchKey(e, 'Item', true),"onpointerup":e => touchKey(e, 'Item', false),"onpointercancel":e => touchKey(e, 'Item', false),});  } }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const KartGame__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type KartGame__SvelteComponent_ = ReturnType<typeof KartGame__SvelteComponent_>;
/*Ωignore_endΩ*/export default KartGame__SvelteComponent_;