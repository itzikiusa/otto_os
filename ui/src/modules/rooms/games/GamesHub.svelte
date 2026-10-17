<script lang="ts">
  import LoadState from '../../../lib/components/LoadState.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import { router } from '../../../lib/router.svelte';
  import { api, baseUrl } from '../../../lib/api/client';
  import type { GameRoomCredential, GameRoomKind, GameRoomMap } from '../../../lib/api/game-room-types';
  import { GAMES, gameFor, RACERS, FIGHTERS, type Difficulty } from './catalog';
  import { kartSeats, shooterSeats } from './setup';
  import type { RacerId } from './kart/roster';
  import type { FighterId } from './shooter/catalog';
  import type { KartSetup } from './kart/KartGame.svelte';
  import type { ShooterSetup } from './shooter/sim';
  import KartGame from './kart/KartGame.svelte';
  import ShooterGame from './shooter/ShooterGame.svelte';
  import GameMatch from './GameMatch.svelte';

  let kind = $state<GameRoomKind>('kart');
  let mode = $state<'solo' | 'room'>('solo');
  let difficulty = $state<Difficulty>('normal');
  let map = $state<GameRoomMap>('meadow');
  let racer = $state<RacerId>('oobi');
  let fighter = $state<FighterId>('male-a');
  let bots = $state(11);
  let name = $state('Player');
  let kartSetup = $state<KartSetup | null>(null);
  let shooterSetup = $state<ShooterSetup | null>(null);
  let credential = $state<GameRoomCredential | null>(null);
  let busy = $state(false), error = $state(''), round = $state(0);
  const game = $derived(gameFor(kind));
  const playing = $derived(!!kartSetup || !!shooterSetup || !!credential);

  function choose(next: GameRoomKind) { kind = next; map = gameFor(next).maps[0].id; }
  async function start() {
    busy = true; error = '';
    try {
      const seed = (Date.now() ^ Math.floor(Math.random() * 1e9)) >>> 0;
      if (mode === 'solo') {
        if (kind === 'kart') kartSetup = { track: map as KartSetup['track'], difficulty, seed, seats: kartSeats([{ slot: 0, name: 'You', pick: racer }], true) };
        else shooterSetup = { map: map as ShooterSetup['map'], difficulty, seed, seats: shooterSeats([{ slot: 0, name: 'You', pick: fighter }], bots) };
      } else {
        credential = await api.post<GameRoomCredential>('/game-rooms', { name: name.trim() || 'Player', config: { game: kind, map } });
      }
    } catch (e) { error = e instanceof Error ? e.message : 'Could not create a game room. Try again.'; }
    finally { busy = false; }
  }
  function exit() { kartSetup = null; shooterSetup = null; credential = null; round++; }
  function again() {
    const seed = (Date.now() ^ Math.floor(Math.random() * 1e9)) >>> 0;
    if (kartSetup) kartSetup = { ...kartSetup, seed };
    if (shooterSetup) shooterSetup = { ...shooterSetup, seed };
    round++;
  }
</script>

<div class="games-page">
  <PageHeader title={playing ? game.title : 'Games'} subtitle={playing ? 'Rooms' : 'Play together between builds'}>
    {#snippet actions()}
      {#if playing}<button class="btn small" onclick={exit}>Back to games</button>{/if}
      <button class="btn small" onclick={() => { exit(); router.go('rooms'); }}>Rooms</button>
    {/snippet}
  </PageHeader>
  {#if kartSetup}
    <PageBody fill padded={false}>{#key round}<KartGame setup={kartSetup} selfSlot={0} {racer} onexit={exit} onrematch={again} />{/key}</PageBody>
  {:else if shooterSetup}
    <PageBody fill padded={false}>{#key round}<ShooterGame setup={shooterSetup} selfSlot={0} {fighter} onexit={exit} onrematch={again} />{/key}</PageBody>
  {:else if credential}
    <PageBody fill padded={false}><GameMatch {credential} origin={baseUrl()} pick={kind === 'kart' ? racer : fighter} {difficulty} {bots} onexit={exit} /></PageBody>
  {:else}
    <PageBody>
      <div class="games-chooser">
        <div class="games-titles" role="group" aria-label="Choose a game">
          {#each GAMES as g (g.kind)}
            <button class:selected={kind === g.kind} aria-pressed={kind === g.kind} onclick={() => choose(g.kind)}>
              <img src={g.cover} alt={g.coverAlt} />
              <div><h2>{g.title}</h2><p>{g.tagline}</p><span>{g.summary}</span></div>
            </button>
          {/each}
        </div>
        <div class="games-setup">
          <section class="games-options" aria-labelledby="games-setup-title">
            <h3 id="games-setup-title">Set up your match</h3>
            <fieldset><legend>Players</legend>
              <div class="games-segments">
                <button aria-pressed={mode === 'solo'} onclick={() => (mode = 'solo')}>Solo vs computer</button>
                <button aria-pressed={mode === 'room'} onclick={() => (mode = 'room')}>Invite people</button>
              </div>
            </fieldset>
            <label>Computer difficulty
              <select bind:value={difficulty}><option value="easy">Easy</option><option value="normal">Normal</option><option value="hard">Hard</option></select>
            </label>
            {#if kind === 'shooter'}
              <label>Bots on the island: {bots}
                <input type="range" min={mode === 'solo' ? 1 : 0} max="15" bind:value={bots} />
              </label>
            {/if}
            {#if mode === 'room'}
              <label>Your display name<input dir="auto" maxlength="40" bind:value={name} /></label>
              <p class="games-note">Up to {game.maxPlayers} people join with one invitation; {kind === 'kart' ? 'CPU racers fill the remaining grid slots' : 'bots fill the island'}.</p>
            {/if}
            <fieldset><legend>{kind === 'kart' ? 'Choose your racer' : 'Choose your survivor'}</legend>
              <div class="games-roster">
                {#if kind === 'kart'}
                  {#each RACERS as r (r.id)}
                    <button class:selected={racer === r.id} aria-pressed={racer === r.id} onclick={() => (racer = r.id)} title={r.tagline}><img src={`/room-games/portraits/racer-${r.id}.webp`} alt="" /><strong>{r.name}</strong><small>{r.tagline}</small></button>
                  {/each}
                {:else}
                  {#each FIGHTERS as f (f.id)}
                    <button class:selected={fighter === f.id} aria-pressed={fighter === f.id} onclick={() => (fighter = f.id)}><img src={`/room-games/portraits/fighter-${f.id}.webp`} alt="" /><strong>{f.name}</strong></button>
                  {/each}
                {/if}
              </div>
            </fieldset>
            <div class="games-start">
              <button class="btn primary" disabled={busy} onclick={start}>{busy ? 'Creating room…' : mode === 'solo' ? (kind === 'kart' ? 'Start race' : 'Drop in') : 'Create game room'}</button>
              {#if error}<LoadState what="game room" {error} empty onretry={start} variant="compact" />{/if}
            </div>
          </section>
          <section class="games-environments" aria-labelledby="games-map-title">
            <h3 id="games-map-title">{kind === 'kart' ? 'Choose a track' : 'Choose an island'}</h3>
            {#each game.maps as item (item.id)}
              <button class:selected={map === item.id} aria-pressed={map === item.id} onclick={() => (map = item.id)}>
                <img src={`/room-games/map-${item.id}.webp`} alt="" />
                <span><strong>{item.name}</strong><small>{item.blurb}</small></span>
              </button>
            {/each}
            <p class="games-note">{kind === 'kart' ? 'W/S gas & brake · A/D steer · Space drift & trick · E item · Q look back' : 'WASD move · Mouse aim · Click fire · Right-click aim · 1–3 weapons · E interact · R reload · Space jump'}</p>
          </section>
        </div>
      </div>
    </PageBody>
  {/if}
</div>

<style>
  .games-page { display: flex; flex-direction: column; height: 100%; min-width: 0; }
  .games-chooser { max-width: 1200px; margin-inline: auto; }
  .games-titles { display: grid; grid-template-columns: 1fr 1fr; gap: 18px; }
  .games-titles > button { display: block; text-align: start; padding: 0; overflow: hidden; background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-l); color: var(--text); cursor: pointer; }
  .games-titles > button.selected { outline: 2px solid var(--accent-solid); outline-offset: 2px; }
  .games-titles img { width: 100%; height: clamp(140px, 19vw, 250px); object-fit: cover; display: block; background: var(--surface-2); }
  .games-titles > button > div { padding: 16px 20px; }
  .games-titles h2 { font-size: var(--fs-2xl); margin: 0 0 6px; }
  .games-titles p { margin: 0 0 8px; font-size: var(--fs-m); }
  .games-titles span { font-size: var(--fs-s); color: var(--text-dim); }
  .games-setup { display: grid; grid-template-columns: minmax(260px, 1fr) minmax(0, 1fr); gap: 36px; margin-block-start: 28px; }
  .games-setup h3 { font-size: var(--fs-l); margin: 0 0 16px; }
  .games-options { display: flex; flex-direction: column; gap: 16px; }
  .games-options h3 { margin-block-end: 0; }
  .games-options fieldset { padding: 0; margin: 0; border: 0; min-width: 0; }
  .games-options legend { margin-block-end: 8px; font-size: var(--fs-m); }
  .games-segments { display: flex; border: 1px solid var(--border); border-radius: var(--radius-m); overflow: hidden; }
  .games-segments button { flex: 1; border: 0; padding: 10px; background: var(--surface); color: var(--text-dim); cursor: pointer; }
  .games-segments button[aria-pressed='true'] { background: var(--accent-soft); color: var(--accent-text); }
  .games-options label { display: grid; gap: 8px; }
  .games-options select, .games-options input:not([type='range']) { width: 100%; min-height: 36px; }
  .games-roster { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
  .games-roster button { display: grid; justify-items: center; gap: 2px; padding: 6px 4px; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); color: var(--text); cursor: pointer; min-width: 0; }
  .games-roster button.selected { border-color: var(--accent-solid); background: var(--accent-soft); }
  .games-roster img { width: 100%; max-width: 88px; aspect-ratio: 1; object-fit: contain; }
  .games-roster strong { font-size: var(--fs-s); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .games-roster small { font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .games-start :global(.btn.primary) { width: 100%; }
  .games-note { font-size: var(--fs-s); line-height: 1.6; color: var(--text-dim); margin: 0; }
  .games-environments > button { width: 100%; display: flex; align-items: center; gap: 14px; text-align: start; color: var(--text); border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 10px; margin-block-end: 10px; cursor: pointer; }
  .games-environments > button.selected { border-color: var(--accent-solid); background: var(--accent-soft); }
  .games-environments img { width: 112px; height: 70px; object-fit: cover; border-radius: var(--radius-s); flex-shrink: 0; background: var(--surface-2); }
  .games-environments strong { font-size: var(--fs-m); display: block; }
  .games-environments small { font-size: var(--fs-s); color: var(--text-dim); display: block; line-height: 1.5; margin-block-start: 4px; }
  @media (max-width: 640px) {
    .games-titles { grid-template-columns: 1fr; gap: 12px; }
    .games-titles img { height: 140px; }
    .games-setup { grid-template-columns: 1fr; gap: 24px; }
    .games-roster { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  }
</style>
