<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { GameClient, type GameConnection } from './client';
  import type { GameRoomCredential, GameRoomEvent, GameRoomMember, GameRoomState } from '../../../lib/api/game-room-types';
  import { gameFor, RACERS, FIGHTERS, type Difficulty } from './catalog';
  import { kartSeats, shooterSeats, type HumanSeat } from './setup';
  import KartGame, { type KartLink, type KartSetup } from './kart/KartGame.svelte';
  import ShooterGame from './shooter/ShooterGame.svelte';
  import type { ShooterLink, ShooterSetup, TeamSize } from './shooter/sim';
  import * as kartNet from './kart/net';
  import * as shooterNet from './shooter/net';
  import { standings, type KartEvent, type KartInput, type KartState } from './kart/sim';
  import type { RacerId } from './kart/roster';
  import type { FighterId, ShooterMode } from './shooter/catalog';
  import { shooterResults, type ShooterEvent, type ShooterInput, type ShooterState } from './shooter/sim';

  interface Props { credential: GameRoomCredential; origin: string; pick?: string; difficulty?: Difficulty; bots?: number; teamSize?: TeamSize; shooterMode?: ShooterMode; onexit: () => void }
  let { credential, origin, pick = '', difficulty = 'normal', bots = 9, teamSize = 1, shooterMode = 'royale', onexit }: Props = $props();
  const kind = $derived(credential.config.game);
  const game = $derived(gameFor(kind));
  const isHost = $derived(credential.role === 'host');

  let client: GameClient;
  let room = $state<GameRoomState | null>(null);
  let connection = $state<GameConnection>('connecting');
  let error = $state(''), closed = $state(''), copied = $state(false);
  let choice = $state(untrack(() => pick));
  let cpuFill = $state(true);
  let botCount = $state(untrack(() => bots));
  let cpuLevel = $state<Difficulty>(untrack(() => difficulty));
  let kartSetup = $state<KartSetup | null>(null);
  let shooterSetup = $state<ShooterSetup | null>(null);
  let finished = $state(false);
  let seq = 0, lastSeq = new Map<number, number>();
  const kartLink: KartLink = { role: 'host', inputs: new Map(), snapshots: [] };
  const shooterLink: ShooterLink = { role: 'host', inputs: new Map(), snapshots: [] };

  const self = $derived(room?.members.find(m => m.id === credential.member_id));
  const selfSlot = $derived(self?.slot ?? (isHost ? 0 : 1));
  const active = $derived(room?.members.filter(m => !m.left) ?? []);
  const everyoneReady = $derived(active.length >= 2 && active.every(m => m.connected && m.ready));
  const memberName = (slot: number) => room?.members.find(m => m.slot === slot)?.name ?? 'Player';

  $effect(() => { if (!choice) choice = kind === 'kart' ? 'oobi' : 'male-a'; });

  function resetRound() {
    seq = 0; lastSeq = new Map(); finished = false;
    kartLink.inputs!.clear(); kartLink.snapshots!.length = 0;
    shooterLink.inputs!.clear(); shooterLink.snapshots!.length = 0;
  }
  function beginRound(state: GameRoomState) {
    // The host seats every connected member; guests only render snapshots.
    const humans: HumanSeat[] = state.members.filter(m => !m.left).map(m => ({ slot: m.slot, name: m.name, pick: m.slot === selfSlot ? choice : '' }));
    const seed = (Date.now() ^ Math.floor(Math.random() * 1e9)) >>> 0;
    if (kind === 'kart') {
      kartLink.role = isHost ? 'host' : 'guest';
      kartSetup = isHost ? { track: state.config.map as KartSetup['track'], difficulty: cpuLevel, seed, seats: kartSeats(humans.map(h => ({ ...h, pick: h.pick || RACERS[h.slot % RACERS.length].id })), cpuFill) } : null;
    } else {
      shooterLink.role = isHost ? 'host' : 'guest';
      shooterSetup = isHost ? { map: state.config.map as ShooterSetup['map'], difficulty: cpuLevel, seed, seats: shooterSeats(humans.map(h => ({ ...h, pick: h.pick || FIGHTERS[(h.slot * 3) % FIGHTERS.length].id })), botCount), teamSize, mode: shooterMode } : null;
    }
  }

  function handle(event: GameRoomEvent) {
    if (event.type === 'state') {
      if (event.room.id !== credential.room_id) return;
      const prev = room;
      if (!prev || prev.round !== event.room.round || prev.generation !== event.room.generation) { seq = 0; lastSeq = new Map(); }
      if (prev && prev.round !== event.room.round) { resetRound(); kartSetup = null; shooterSetup = null; }
      room = event.room;
      error = '';
      if (event.room.phase === 'playing' && (!prev || prev.phase === 'lobby')) { resetRound(); beginRound(event.room); }
      return;
    }
    if (event.type === 'closed') { closed = event.reason; return; }
    if (event.type === 'error') { if (!event.message.includes('Stale game round')) error = event.message; return; }
    if (event.type === 'pong' || !room || !('round' in event) || event.round !== room.round || event.generation !== room.generation) return;
    if (event.type === 'snapshot' && !isHost) {
      if (event.seq <= (lastSeq.get(0) ?? 0)) return;
      lastSeq.set(0, event.seq);
      const at = performance.now();
      if (kind === 'kart') {
        const state = kartNet.decodeSnapshot(event.data, (slot, racer) => (slot >= 0 ? memberName(slot) : RACERS.find(r => r.id === racer)?.name ?? 'CPU'));
        if (state) { kartLink.snapshots!.push({ state, at }); if (kartLink.snapshots!.length > 4) kartLink.snapshots!.shift(); }
      } else {
        const prev = shooterLink.snapshots!.at(-1)?.state;
        const state = shooterNet.decodeSnapshot(event.data, slot => memberName(slot), prev?.loot ?? null, prev?.builds ?? null);
        if (state) { shooterLink.snapshots!.push({ state, at }); if (shooterLink.snapshots!.length > 4) shooterLink.snapshots!.shift(); }
      }
    }
    if (event.type === 'input' && isHost) {
      if (event.seq <= (lastSeq.get(event.slot) ?? 0)) return;
      lastSeq.set(event.slot, event.seq);
      if (kind === 'kart') { const d = kartNet.decodeInput(event.data); if (d) kartLink.inputs!.set(event.slot, d); }
      else { const d = shooterNet.decodeInput(event.data); if (d) shooterLink.inputs!.set(event.slot, d); }
    }
    if (event.type === 'finished') finished = true;
  }

  onMount(() => {
    client = new GameClient(origin, credential, handle, status => {
      connection = status;
      if (status !== 'connected') { kartLink.inputs!.clear(); shooterLink.inputs!.clear(); }
    });
    client.connect();
    return () => client.dispose();
  });

  const live = () => room && room.phase === 'playing' && !room.paused;
  function sendSnapshot(data: unknown) { if (live() && room) client.send({ type: 'snapshot', round: room.round, generation: room.generation, seq: ++seq, data }); }
  function kartFrame(state: KartState, events: KartEvent[]) { sendSnapshot(kartNet.encodeSnapshot(state, events)); }
  function kartInput(input: KartInput, racer: RacerId) { if (live() && room) client.send({ type: 'input', round: room.round, generation: room.generation, seq: ++seq, data: kartNet.encodeInput(input, racer) }); }
  function kartFinish(state: KartState) {
    if (!isHost || !room) return;
    kartFrame(state, state.events);
    const result = standings(state).map(r => ({ place: r.place, name: r.name.slice(0, 40), time: Math.round(r.time * 1000) / 1000 }));
    client.send({ type: 'finish', round: room.round, generation: room.generation, result: { game: 'kart', standings: result } });
  }
  function shooterFrame(state: ShooterState, events: ShooterEvent[], withLoot: boolean, withBuilds = false) { sendSnapshot(shooterNet.encodeSnapshot(state, events, withLoot, withBuilds)); }
  function shooterInput(input: ShooterInput, fighter: FighterId) { if (live() && room) client.send({ type: 'input', round: room.round, generation: room.generation, seq: ++seq, data: shooterNet.encodeInput(input, fighter) }); }
  function shooterFinish(state: ShooterState) {
    if (!isHost || !room) return;
    shooterFrame(state, state.events, true);
    const result = shooterResults(state).map(r => ({ place: r.place, name: r.name.slice(0, 40), kills: r.kills }));
    client.send({ type: 'finish', round: room.round, generation: room.generation, result: { game: 'shooter', standings: result } });
  }
  function rematch() { client.send({ type: 'rematch' }); }
  async function copyInvite() { try { await navigator.clipboard.writeText(credential.invite_url ?? ''); copied = true; } catch { error = 'Copy the invitation from the field below.'; } }
  const statusOf = (m: GameRoomMember) => (m.left ? 'Left' : !m.connected ? 'Connecting…' : m.ready ? 'Ready' : 'Not ready');
  const rematchRequests = $derived(room?.members.filter(m => m.rematch && !m.left).length ?? 0);
</script>

<div class="match">
  {#if closed}
    <div class="match-wait" role="status"><h2>Match ended</h2><p>{closed}</p><button class="btn primary" onclick={onexit}>Back to games</button></div>
  {:else if room && (room.phase === 'playing' || room.phase === 'finished')}
    {#key room.round}
      {#if kind === 'kart'}
        <KartGame setup={kartSetup} {selfSlot} racer={choice as RacerId} link={kartLink} names={memberName} paused={connection !== 'connected' || room.paused}
          onframe={isHost ? kartFrame : undefined} oninput={isHost ? undefined : kartInput} onfinish={kartFinish} {onexit}
          onrematch={rematch} rematchLabel={isHost ? (rematchRequests ? `Back to lobby (${rematchRequests} want a rematch)` : 'Back to lobby') : self?.rematch ? 'Rematch requested' : 'Ask for a rematch'} rematchDisabled={!finished || (!isHost && !!self?.rematch)} />
      {:else}
        <ShooterGame setup={shooterSetup} {selfSlot} fighter={choice as FighterId} link={shooterLink} names={memberName} paused={connection !== 'connected' || room.paused}
          onframe={isHost ? shooterFrame : undefined} oninput={isHost ? undefined : shooterInput} onfinish={shooterFinish} {onexit}
          onrematch={rematch} rematchLabel={isHost ? (rematchRequests ? `Back to lobby (${rematchRequests} want a rematch)` : 'Back to lobby') : self?.rematch ? 'Rematch requested' : 'Ask for a rematch'} rematchDisabled={!finished || (!isHost && !!self?.rematch)} />
      {/if}
    {/key}
  {:else}
    <div class="match-wait">
      <h2>{game.title}</h2>
      <p>{connection === 'connected' ? `${game.maps.find(m => m.id === credential.config.map)?.name ?? ''} · up to ${game.maxPlayers} people` : connection === 'connecting' ? 'Connecting to game room…' : 'Connection lost. Reconnecting…'}</p>
      {#if credential.invite_url}
        <div class="match-invite">
          <label for="game-invite">Invite players</label>
          <div class="match-invite-row"><input dir="ltr" id="game-invite" readonly value={credential.invite_url} aria-label="Game invitation" /><button class="btn" onclick={copyInvite}>{copied ? 'Copied' : 'Copy invitation'}</button></div>
          <p>One invitation admits up to {game.maxPlayers - 1} guests until the match starts. For another computer, use the HTTPS origin configured in Rooms connection settings.</p>
        </div>
      {/if}
      <fieldset class="match-roster"><legend>{kind === 'kart' ? 'Your racer' : 'Your survivor'}</legend>
        <div>
          {#if kind === 'kart'}
            {#each RACERS as r (r.id)}<button aria-pressed={choice === r.id} onclick={() => (choice = r.id)} title={r.name}><img src={`/room-games/portraits/racer-${r.id}.webp`} alt={r.name} /></button>{/each}
          {:else}
            {#each FIGHTERS as f (f.id)}<button aria-pressed={choice === f.id} onclick={() => (choice = f.id)} title={f.name}><img src={`/room-games/portraits/fighter-${f.id}.webp`} alt={f.name} /></button>{/each}
          {/if}
        </div>
      </fieldset>
      {#if isHost}
        <div class="match-settings">
          <label>Computer difficulty<select bind:value={cpuLevel}><option value="easy">Easy</option><option value="normal">Normal</option><option value="hard">Hard</option></select></label>
          {#if kind === 'kart'}<label class="match-check"><input type="checkbox" bind:checked={cpuFill} /> Fill the grid with CPU racers</label>
          {:else}<label>Bots: {botCount}<input type="range" min="0" max="15" bind:value={botCount} /></label>{/if}
        </div>
      {/if}
      <ul class="match-members" aria-label="Players">
        {#each room?.members ?? [] as member (member.id)}
          <li class:left={member.left}><strong>{member.name}{member.id === credential.member_id ? ' (you)' : ''}</strong><span>{member.role === 'host' ? 'Host · ' : ''}{statusOf(member)}</span></li>
        {/each}
        {#if active.length < 2}<li class="match-empty">Waiting for players to join…</li>{/if}
      </ul>
      <div class="match-actions">
        <button class="btn" disabled={connection !== 'connected' || !room} onclick={() => client.send({ type: 'ready', ready: !self?.ready })}>{self?.ready ? 'Not ready' : 'Ready'}</button>
        {#if isHost}<button class="btn primary" disabled={!everyoneReady || connection !== 'connected'} onclick={() => room && client.send({ type: 'start', round: room.round, generation: room.generation })}>Start match</button>{/if}
        <button class="btn" onclick={onexit}>Leave</button>
      </div>
    </div>
  {/if}
  {#if error}<p class="match-error" role="alert">{error}</p>{/if}
</div>

<style>
  .match { position: relative; display: flex; flex-direction: column; flex: 1; min-height: 450px; }
  .match-wait { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px; padding: 28px; text-align: center; }
  .match-wait h2 { font-size: var(--fs-2xl); margin: 0; }
  .match-wait p { color: var(--text-dim); margin: 0; line-height: 1.5; }
  .match-invite { width: min(100%, 640px); display: grid; gap: 8px; text-align: start; }
  .match-invite-row { display: flex; gap: 8px; }
  .match-invite input { flex: 1; min-width: 0; }
  .match-invite p { font-size: var(--fs-s); }
  .match-roster { border: 0; padding: 0; margin: 0; max-width: 100%; }
  .match-roster legend { font-size: var(--fs-m); margin-block-end: 8px; }
  .match-roster div { display: flex; flex-wrap: wrap; justify-content: center; gap: 6px; }
  .match-roster button { border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); padding: 4px; cursor: pointer; }
  .match-roster button[aria-pressed='true'] { border-color: var(--accent-solid); background: var(--accent-soft); }
  .match-roster img { width: 56px; height: 56px; object-fit: contain; display: block; }
  .match-settings { display: flex; flex-wrap: wrap; gap: 16px; align-items: end; justify-content: center; }
  .match-settings label { display: grid; gap: 6px; text-align: start; }
  .match-settings .match-check { display: flex; align-items: center; gap: 8px; }
  .match-members { width: min(100%, 440px); list-style: none; margin: 0; padding: 0; }
  .match-members li { display: flex; justify-content: space-between; gap: 16px; padding: 10px 12px; border-block-end: 1px solid var(--border); }
  .match-members li.left { opacity: .55; }
  .match-members span { color: var(--text-dim); font-size: var(--fs-s); }
  .match-empty { color: var(--text-dim); justify-content: center !important; }
  .match-actions { display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; }
  .match-error { position: absolute; inset: 72px 20px auto; z-index: 8; background: var(--danger-soft); color: var(--danger); padding: 12px; border-radius: var(--radius-m); margin: 0; }
</style>
