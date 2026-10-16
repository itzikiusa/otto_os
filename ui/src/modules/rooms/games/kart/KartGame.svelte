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
  import { createRace, idleInput, standings, stepRace, type KartEvent } from './sim';
  import { trackFor } from './tracks';
  import { racerDef } from './roster';
  import { ITEMS } from './items';
  import { InputState } from '../common/input';
  import { GameAudio } from '../common/audio';
  import { ordinal, raceClock, lerp, wrapAngle } from '../common/math';
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
  const audio = new GameAudio(['voice-3', 'voice-2', 'voice-1', 'voice-go', 'item', 'tick', 'boost', 'turbo', 'spin', 'explode', 'bump', 'lap', 'finallap', 'drop', 'shield', 'land', 'pickup', 'voice-win', 'voice-congrats', 'voice-over', 'select']);
  let renderer: KartRenderer | undefined, input: InputState | undefined;
  let stop: (() => void) | undefined;
  const isGuest = $derived(link?.role === 'guest');
  const self = $derived(hud ? hud.racers.find(r => (r.human && r.slot === selfSlot)) ?? hud.racers[0] : null);
  const selfIndex = $derived(hud && self ? hud.racers.indexOf(self) : 0);
  const ROULETTE = ['mushroom', 'banana', 'bomb', 'star', 'bolt', 'triple'] as const;

  function readInput(touchGas: boolean): KartInput {
    const i = input;
    if (!i || menu || paused) return idleInput();
    const up = i.down('KeyW', 'ArrowUp') || touchGas, down = i.down('KeyS', 'ArrowDown');
    const left = i.down('KeyA', 'ArrowLeft'), right = i.down('KeyD', 'ArrowRight');
    return {
      throttle: up && !down ? 1 : down && !up ? -1 : 0,
      steer: (right ? 1 : 0) - (left ? 1 : 0),
      drift: i.down('Space', 'ShiftLeft', 'ShiftRight', 'Drift'),
      item: i.down('KeyE', 'KeyF', 'Enter', 'Item'),
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
    switch (e.type) {
      case 'count': audio.play(`voice-${e.value}`, { volume: 1.1 }); announce(String(e.value), 'count', 900); break;
      case 'go': audio.play('voice-go', { volume: 1.1 }); announce('GO!', 'count', 900); break;
      case 'box': audio.play('pickup', { ...vol, volume: 0.6 }); break;
      case 'item': if (own) audio.play('item'); break;
      case 'boost': audio.play('boost', { ...vol, volume: 0.7 }); break;
      case 'turbo': audio.play('turbo', { ...vol, volume: 0.6 + (e.value ?? 1) * 0.1, rate: 0.9 + (e.value ?? 1) * 0.1 }); if (own && (e.value ?? 0) >= 2) announce((e.value ?? 0) >= 3 ? 'Ultra mini-turbo!' : 'Super mini-turbo!', 'small', 900); break;
      case 'spin': audio.play('spin', vol); if (own) announce('Spun out!', 'bad', 900); break;
      case 'explode': audio.play('explode', vol); break;
      case 'bump': audio.play('bump', { ...vol, volume: 0.5 }); break;
      case 'drop': audio.play('drop', vol); break;
      case 'throw': audio.play('boost', { ...vol, rate: 0.7 }); break;
      case 'star': audio.play('shield', vol); if (own) announce('Super Star!', 'good'); break;
      case 'bolt': audio.play('explode', { volume: 0.8, rate: 1.4 }); announce(own ? 'Thunderbolt!' : `${r?.name ?? 'Someone'} used a Thunderbolt`, 'bad'); break;
      case 'trick': if (own) { audio.play('select'); announce('Trick!', 'small', 700); } break;
      case 'land': if (own) audio.play('land', { volume: 0.6 }); break;
      case 'respawn': if (own) announce('Rescued', 'small', 900); break;
      case 'lap': if (own) { audio.play('lap'); announce(`Lap ${(e.value ?? 0) + 1}`, 'good'); } break;
      case 'final': if (own) { audio.play('finallap'); announce('Final lap!', 'good', 1800); } break;
      case 'finish':
        if (own) { audio.play(r && r.place <= 3 ? 'voice-congrats' : 'lap'); announce(`Finished ${ordinal(r?.place ?? 1)}!`, 'good', 3000); }
        break;
    }
  }

  function drawMinimap(state: KartState, mine: number): void {
    const c = minimap;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const track = trackFor(state.track);
    const w = c.width, h = c.height;
    let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
    for (let i = 0; i < track.count; i++) { minX = Math.min(minX, track.px[i]); maxX = Math.max(maxX, track.px[i]); minZ = Math.min(minZ, track.pz[i]); maxZ = Math.max(maxZ, track.pz[i]); }
    const sc = Math.min((w - 20) / (maxX - minX), (h - 20) / (maxZ - minZ));
    // Mirror X so the map matches the chase view (+X is to the left when facing +Z).
    const mx = (x: number) => w - (10 + (x - minX) * sc + (w - 20 - (maxX - minX) * sc) / 2);
    const mz = (z: number) => 10 + (z - minZ) * sc + (h - 20 - (maxZ - minZ) * sc) / 2;
    g.clearRect(0, 0, w, h);
    g.lineJoin = 'round';
    g.beginPath();
    for (let i = 0; i <= track.count; i++) { const k = i % track.count; if (i === 0) g.moveTo(mx(track.px[k]), mz(track.pz[k])); else g.lineTo(mx(track.px[k]), mz(track.pz[k])); }
    g.strokeStyle = 'rgba(0,0,0,0.55)'; g.lineWidth = 9; g.stroke();
    g.strokeStyle = 'rgba(255,255,255,0.9)'; g.lineWidth = 5; g.stroke();
    g.fillStyle = '#111'; g.fillRect(mx(track.px[0]) - 4, mz(track.pz[0]) - 4, 8, 8);
    for (const b of state.shells) { g.fillStyle = '#ff7a2a'; g.beginPath(); g.arc(mx(b.x), mz(b.z), 3, 0, Math.PI * 2); g.fill(); }
    const order = [...state.racers].sort((a, b) => b.place - a.place);
    for (const r of order) {
      const own = r.id === mine;
      g.beginPath(); g.arc(mx(r.x), mz(r.z), own ? 7 : 5, 0, Math.PI * 2);
      g.fillStyle = racerDef(r.racer).colour; g.fill();
      g.lineWidth = own ? 3 : 1.5; g.strokeStyle = own ? '#fff' : 'rgba(0,0,0,0.6)'; g.stroke();
    }
  }

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
    let disposed = false, frame = 0, last = performance.now(), lastHud = 0, lastSend = 0, lastEvent = 0, rouletteAt = 0;
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
      (stage as HTMLDivElement & { __ottoGame?: unknown }).__ottoGame = { snapshot: () => structuredClone(state ?? view), stats: () => created.stats() };
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
        if (!paused && !(menu && !link)) stepRace(state, inputs, dt);
        pending.push(...state.events.filter(e => e.id > lastEvent));
      } else if (now - lastSend > 33) {
        oninput?.(mine, racer);
        lastSend = now;
      }
      for (const e of state.events) if (e.id > lastEvent) { onEvent(state, e, mineIndex); lastEvent = e.id; }
      if (!isGuest && onframe && now - lastSend > 50) { onframe(state, pending); pending = []; lastSend = now; }
      if (renderer) {
        renderer.frame(state, dt, input?.down('KeyQ', 'KeyC', 'Back') ?? false);
        const me = state.racers[mineIndex];
        audio.engineLevel(Math.abs(me.speed) / 30, me.boost > 0, !paused && !menu && state.phase !== 'done' && me.respawn <= 0);
      }
      input?.endFrame();
      if (now - lastHud > 90) {
        hud = { ...state, racers: state.racers.map(r => ({ ...r })) };
        if (state.racers[mineIndex]?.roll > 0 && now - rouletteAt > 80) { rouletteIcon = (rouletteIcon + 1) % ROULETTE.length; rouletteAt = now; audio.play('tick', { volume: 0.3 }); }
        drawMinimap(state, mineIndex);
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
  {#if loading && !error}<div class="kart-overlay" role="status"><h2>{isGuest && !hud ? 'Waiting for the host…' : 'Building the circuit…'}</h2><p>Loading karts, racers and scenery</p></div>{/if}
  {#if error}<div class="kart-overlay"><LoadState what="race" {error} empty onretry={start} /><button class="btn" onclick={onexit}>Back to games</button></div>{/if}
  {#if hud && self && !loading && !error}
    <div class="kart-top">
      <div class="kart-lap" aria-label="Lap"><span>Lap</span><strong>{lapNow}<small>/{hud.laps}</small></strong><time>{raceClock(hud.raceTime)}</time>
        {#if self.lapTimes.length}<ol class="kart-laptimes">{#each self.lapTimes as t, i (i)}<li class:best={t === best}>L{i + 1} {raceClock(t)}</li>{/each}</ol>{/if}
      </div>
      <div class="kart-item" class:rolling={self.roll > 0} aria-label="Item">
        {#if self.roll > 0}<img src={`/room-games/portraits/item-${ROULETTE[rouletteIcon] === 'triple' ? 'mushroom' : ROULETTE[rouletteIcon] === 'bolt' ? 'star' : ROULETTE[rouletteIcon]}.webp`} alt="" />
        {:else if self.item}
          {#if self.item === 'bolt'}<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M13 2 4 14h6l-1 8 9-12h-6z" /></svg>
          {:else}<img src={`/room-games/portraits/item-${self.item === 'triple' ? 'mushroom' : self.item}.webp`} alt="" />{/if}
          {#if self.itemCount > 1}<b>×{self.itemCount}</b>{/if}
          <span class="kart-item-name">{ITEMS[self.item].name} · E</span>
        {:else}<span class="kart-item-empty">Item</span>{/if}
      </div>
      <div class="kart-tools">
        <button class="btn small" aria-label={muted ? 'Unmute game' : 'Mute game'} title={muted ? 'Unmute game' : 'Mute game'} onclick={() => { muted = !muted; audio.mute(muted); }}>{muted ? 'Sound off' : 'Sound on'}</button>
        {#if typeof document !== 'undefined' && document.fullscreenEnabled}<button class="btn small" onclick={toggleFullscreen}>{fullscreen ? 'Exit full screen' : 'Full screen'}</button>{/if}
        <button class="btn small" onclick={() => { menu = !menu; }}>Menu</button>
      </div>
    </div>
    <ol class="kart-ranking" aria-label="Race positions">
      {#each [...hud.racers].sort((a, b) => a.place - b.place) as r (r.id)}
        <li class:me={r.id === selfIndex}><span class="kart-dot" style:background={racerDef(r.racer).colour}></span><b>{r.place}</b><span class="kart-rname">{r.name}</span>{#if r.finish !== null}<em>✓</em>{/if}</li>
      {/each}
    </ol>
    <div class="kart-place" aria-live="polite"><strong>{self.place}</strong><span>{ordinal(self.place).replace(/^\d+/, '')}</span></div>
    <canvas class="kart-minimap" bind:this={minimap} width="220" height="220" aria-label="Track map"></canvas>
    {#if self.drift !== 0}<div class="kart-drift" data-level={self.driftLevel} aria-hidden="true"><i></i><i></i><i></i></div>{/if}
    {#if notice}<div class="kart-notice" data-kind={noticeKind} aria-live="polite">{notice}</div>{/if}
    {#if self.wrong > 1 && hud.phase === 'race'}<div class="kart-wrong" role="alert">Wrong way!</div>{/if}
    {#if hud.phase === 'intro'}<div class="kart-intro"><h2>{trackFor(hud.track).def.name}</h2><p>{hud.racers.length} racers · {hud.laps} laps · {hud.difficulty} CPUs</p></div>{/if}
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
  .kart-stage :global(canvas) { display: block; width: 100%; height: 100%; outline: none; }
  .kart-overlay { position: absolute; inset: 0; z-index: 6; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 24px; text-align: center; background: color-mix(in srgb, var(--surface) 90%, transparent); }
  .kart-overlay h2 { margin: 0; font-size: var(--fs-2xl); }
  .kart-overlay p { margin: 0; max-width: 560px; color: var(--text-dim); line-height: 1.6; }
  .kart-top { position: absolute; inset-block-start: 14px; inset-inline: 14px; display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; pointer-events: none; }
  .kart-lap, .kart-item, .kart-tools, .kart-ranking, .kart-place, .kart-minimap { background: var(--scrim-media); color: var(--on-scrim); border: 1px solid color-mix(in srgb, var(--on-scrim) 18%, transparent); border-radius: var(--radius-l); }
  .kart-lap { padding: 10px 16px; display: grid; grid-template-columns: auto auto; column-gap: 12px; align-items: baseline; min-width: 150px; }
  .kart-lap span { font-size: var(--fs-s); text-transform: uppercase; letter-spacing: .08em; opacity: .85; }
  .kart-lap strong { font-size: calc(var(--fs-hero) * 1.1); font-style: italic; line-height: 1; }
  .kart-lap small { font-size: var(--fs-l); opacity: .8; }
  .kart-lap time { grid-column: 1 / -1; font-variant-numeric: tabular-nums; font-size: var(--fs-l); margin-block-start: 4px; }
  .kart-laptimes { grid-column: 1 / -1; list-style: none; margin: 6px 0 0; padding: 0; font-size: var(--fs-s); font-variant-numeric: tabular-nums; opacity: .9; }
  .kart-laptimes li.best { color: var(--success); font-weight: 600; }
  .kart-item { width: 104px; height: 104px; display: grid; place-items: center; position: relative; border-width: 3px; }
  .kart-item img, .kart-item svg { width: 76px; height: 76px; object-fit: contain; }
  .kart-item svg { fill: var(--warning); stroke: var(--on-scrim); stroke-width: 1; }
  .kart-item.rolling img { animation: kart-roll .08s linear infinite; }
  .kart-item b { position: absolute; inset-block-end: 4px; inset-inline-end: 8px; font-size: var(--fs-l); }
  .kart-item-name { position: absolute; inset-block-start: calc(100% + 6px); white-space: nowrap; font-size: var(--fs-s); background: var(--scrim-media); padding: 2px 8px; border-radius: var(--radius-s); }
  .kart-item-empty { font-size: var(--fs-s); opacity: .6; text-transform: uppercase; letter-spacing: .1em; }
  @keyframes kart-roll { from { transform: translateY(-6px); } to { transform: translateY(6px); } }
  .kart-tools { display: flex; gap: 6px; padding: 6px; pointer-events: auto; }
  .kart-tools :global(.btn) { background: color-mix(in srgb, var(--on-scrim) 14%, transparent); color: var(--on-scrim); border-color: color-mix(in srgb, var(--on-scrim) 25%, transparent); }
  .kart-ranking { position: absolute; inset-inline-end: 14px; inset-block-start: 80px; list-style: none; margin: 0; padding: 8px; display: grid; gap: 2px; min-width: 170px; font-size: var(--fs-s); pointer-events: none; }
  .kart-ranking li { display: flex; align-items: center; gap: 8px; padding: 3px 6px; border-radius: var(--radius-s); }
  .kart-ranking li.me { background: color-mix(in srgb, var(--on-scrim) 22%, transparent); font-weight: 600; }
  .kart-ranking b { width: 14px; text-align: end; font-variant-numeric: tabular-nums; }
  .kart-rname { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 120px; }
  .kart-ranking em { font-style: normal; color: var(--success); }
  .kart-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; border: 1px solid color-mix(in srgb, var(--on-scrim) 50%, transparent); }
  .kart-place { position: absolute; inset-inline-end: 18px; inset-block-end: 18px; padding: 6px 18px 10px; display: flex; align-items: baseline; pointer-events: none; }
  .kart-place strong { font-size: calc(var(--fs-hero) * 2.6); font-style: italic; line-height: 1; }
  .kart-place span { font-size: var(--fs-2xl); font-style: italic; margin-inline-start: 2px; }
  .kart-minimap { position: absolute; inset-inline-start: 14px; inset-block-end: 14px; width: 200px; height: 200px; padding: 0; pointer-events: none; }
  .kart-drift { position: absolute; inset-block-end: 26px; inset-inline-start: 50%; transform: translateX(-50%); display: flex; gap: 6px; pointer-events: none; }
  .kart-drift i { width: 34px; height: 10px; border-radius: 5px; background: color-mix(in srgb, var(--on-scrim) 25%, transparent); }
  .kart-drift[data-level='1'] i:nth-child(1), .kart-drift[data-level='2'] i:nth-child(-n+2), .kart-drift[data-level='3'] i { background: var(--info); }
  .kart-drift[data-level='2'] i:nth-child(-n+2) { background: var(--warning); }
  .kart-drift[data-level='3'] i { background: var(--cat-5); }
  .kart-notice { position: absolute; inset-block-start: 28%; inset-inline: 12px; text-align: center; font-size: clamp(26px, 4.4vw, 48px); font-weight: 700; font-style: italic; color: var(--on-scrim); text-shadow: 0 3px 10px var(--scrim-media); pointer-events: none; }
  .kart-notice[data-kind='count'] { font-size: clamp(64px, 12vw, 140px); inset-block-start: 22%; }
  .kart-notice[data-kind='small'] { font-size: clamp(18px, 2.6vw, 28px); inset-block-start: 62%; }
  .kart-notice[data-kind='bad'] { color: var(--danger); }
  .kart-wrong { position: absolute; inset-block-start: 40%; inset-inline: 0; text-align: center; font-size: var(--fs-2xl); font-weight: 700; color: var(--on-scrim); background: color-mix(in srgb, var(--danger) 70%, transparent); padding: 8px; pointer-events: none; }
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
  .kart-results-kicker { text-transform: uppercase; letter-spacing: .1em; font-size: var(--fs-s); }
  .kart-actions { display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
  .kart-keys { font-size: var(--fs-s); }
  .kart-touch { display: none; }
  @media (max-width: 760px) { .kart-ranking { display: none; } .kart-minimap { width: 130px; height: 130px; } .kart-lap { min-width: 0; } .kart-tools { flex-wrap: wrap; justify-content: flex-end; } }
  @media (pointer: coarse) {
    .kart-touch { position: absolute; inset-inline: 12px; inset-block-end: 150px; display: flex; justify-content: space-between; gap: 8px; }
    .kart-touch div { display: flex; gap: 8px; }
    .kart-touch button { touch-action: none; min-width: 64px; min-height: 56px; border-radius: var(--radius-m); border: 1px solid var(--border); background: color-mix(in srgb, var(--surface) 80%, transparent); color: var(--text); font-size: var(--fs-m); }
    .kart-hint { display: none; }
  }
  @media (prefers-reduced-motion: reduce) { .kart-item.rolling img { animation: none; } }
</style>
