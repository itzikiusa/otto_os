<script lang="ts">
  import LoadState from '../../../lib/components/LoadState.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import {router} from '../../../lib/router.svelte';
  import {api,baseUrl} from '../../../lib/api/client';
  import type {GameRoomCredential} from '../../../lib/api/game-room-types';
  import type {GameConfig,GameKind,Difficulty} from './types';
  import {ARENAS,TRACKS} from './maps';
  import {DRIVERS,type DriverId} from './roster';
  let driver=$state<DriverId>('fox');
  import GamePlay from './GamePlay.svelte';
  import GameMatch from './GameMatch.svelte';
  let kind=$state<GameKind>('shooter'),opponent=$state<'computer'|'person'>('computer'),difficulty=$state<Difficulty>('normal'),map=$state('station'),skin=$state<'azure'|'ember'>('azure'),name=$state('Player');
  let config=$state<GameConfig|null>(null),credential=$state<GameRoomCredential|null>(null),busy=$state(false),error=$state(''),round=$state(0);
  const maps=$derived(Object.values(kind==='shooter'?ARENAS:TRACKS));
  const descriptions:Record<string,string>={station:'An orbital outpost. Reactor lanes, weapon choices and shield pickups.',foundry:'A working foundry. Industrial machinery and close-quarter duels.',dunes:'Weathered ruins. Long sightlines, shaded cover and supply pickups.',coast:'Island resort, a dive through coral reefs and a leap back into the sunshine.',forest:'Country lanes, a hillside village, woodland bridges and big air.',neon:'Elevated expressways, rooftop jumps and a city of lights.'};
  function choose(value:GameKind){kind=value;map=value==='shooter'?'station':'coast';}
  async function start(){
    busy=true;error='';
    try{
      if(opponent==='computer')config={kind,map,difficulty,vsComputer:true,seed:Date.now()>>>0};
      else credential=await api.post<GameRoomCredential>('/game-rooms',{name:name.trim()||'Player',config:{game:kind,map}});
    }catch(e){error=e instanceof Error?e.message:'Could not create a game room. Try again.';}finally{busy=false;}
  }
  function exit(){config=null;credential=null;round++;}
</script>
<div class="games-page">
  <PageHeader title={config||credential?(kind==='shooter'?'Arena Duel':'Circuit Clash'):'Games'} subtitle={config||credential?'Rooms':'One opponent. Your arena.'}>
    {#snippet actions()}<button class="btn small" onclick={()=>{exit();router.go('rooms');}}>Rooms</button>{#if config||credential}<button class="btn small" onclick={exit}>Back to games</button>{/if}{/snippet}
  </PageHeader>
  {#if config}<PageBody fill padded={false}>{#key round}<GamePlay {config} {skin} {driver} onexit={exit} onrematch={()=>round++}/>{/key}</PageBody>
  {:else if credential}<PageBody fill padded={false}><GameMatch {credential} {skin} {driver} origin={baseUrl()} onexit={exit}/></PageBody>
  {:else}<PageBody>
    <div class="games-chooser">
      <div class="games-titles" aria-label="Choose a game">
        <button class:selected={kind==='shooter'} aria-pressed={kind==='shooter'} onclick={()=>choose('shooter')}><img src="/room-games/arena-preview.png" alt="Armored fighter in a sci-fi arena"/><div><h2>Arena Duel</h2><p>Find your angle. Win the duel.</p><span>First or third person · 1v1 shooter</span></div></button>
        <button class:selected={kind==='kart'} aria-pressed={kind==='kart'} onclick={()=>choose('kart')}><img src="/room-games/kart-preview.png" alt="Original character kart racing"/><div><h2>Circuit Clash</h2><p>From country roads to coral reefs.</p><span>Three laps · 1v1 kart racing</span></div></button>
      </div>
      <div class="games-setup">
        <section class="games-options"><h3>Set up your match</h3>
          <fieldset><legend>Opponent</legend><div class="games-segments"><button aria-pressed={opponent==='computer'} onclick={()=>opponent='computer'}>Computer</button><button aria-pressed={opponent==='person'} onclick={()=>opponent='person'}>Another person</button></div></fieldset>
          {#if opponent==='computer'}<label>Difficulty<select bind:value={difficulty}><option value="easy">Easy</option><option value="normal">Normal</option><option value="hard">Hard</option></select></label>
          {:else}<label>Your display name<input dir="auto" maxlength="40" bind:value={name}/></label><p class="games-note">Create a room, share its invitation and ready up together.</p>{/if}
          {#if kind==='kart'}<fieldset><legend>Choose your driver</legend><div class="games-drivers">{#each DRIVERS as racer}<button class:selected={driver===racer.id} aria-pressed={driver===racer.id} onclick={()=>driver=racer.id}><img src={`/room-games/driver-${racer.id}.png`} alt={racer.species}/><strong>{racer.name}</strong><small>{racer.species}</small></button>{/each}</div><p class="games-note">{DRIVERS.find(r=>r.id===driver)?.motto}</p></fieldset>{:else}<fieldset><legend>Fighter</legend><div class="games-segments"><button aria-pressed={skin==='azure'} onclick={()=>skin='azure'}>Azure</button><button aria-pressed={skin==='ember'} onclick={()=>skin='ember'}>Ember</button></div></fieldset>{/if}
          <div class="games-start"><button class="btn primary" disabled={busy} onclick={start}>{busy?'Creating room…':opponent==='computer'?'Play computer':'Create game room'}</button>{#if error}<LoadState what="game room" {error} empty onretry={start} variant="compact"/>{/if}</div>
        </section>
        <section class="games-environments"><h3>{kind==='shooter'?'Choose your arena':'Choose your circuit'}</h3>
          {#each maps as item}<button class:selected={map===item.id} aria-pressed={map===item.id} onclick={()=>map=item.id}><span class="game-map-art" data-map={item.id} aria-hidden="true"></span><span><strong>{item.name}</strong><small>{descriptions[item.id]}</small></span><span aria-hidden="true">{map===item.id?'●':'○'}</span></button>{/each}
          <p class="games-note">{kind==='shooter'?'WASD move · Click fire · 1/2/3 weapons · E dash · R reload':'WASD drive · Space drift / air trick · E item · R recover'}</p>
        </section>
      </div>
    </div>
  </PageBody>{/if}
</div>
<style>
 .games-drivers{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-block-end:8px;}.games-drivers button{display:grid;gap:4px;padding:6px;border:1px solid var(--border);border-radius:var(--radius-m);background:var(--surface);color:var(--text);cursor:pointer;}.games-drivers button.selected{border-color:var(--accent-solid);background:var(--accent-soft);}.games-drivers img{width:100%;aspect-ratio:1;object-fit:cover;border-radius:var(--radius-s);}.games-drivers strong{font-size:var(--fs-m);}.games-drivers small{font-size:var(--fs-s);color:var(--text-dim);}
 .games-page{display:flex;flex-direction:column;height:100%;min-width:0;}.games-chooser{max-width:1200px;margin-inline:auto;}.games-titles{display:grid;grid-template-columns:1fr 1fr;gap:18px;}.games-titles>button{display:block;text-align:start;padding:0;overflow:hidden;background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-l);color:var(--text);cursor:pointer;}.games-titles>button.selected{outline:2px solid var(--accent-solid);outline-offset:2px;}.games-titles img{width:100%;height:clamp(140px,19vw,250px);object-fit:cover;display:block;}.games-titles>button>div{padding:18px 20px;}.games-titles h2{font-size:var(--fs-2xl);margin:0 0 6px;}.games-titles p{margin:0 0 10px;font-size:var(--fs-m);}.games-titles span{font-size:var(--fs-s);color:var(--text-dim);}.games-setup{display:grid;grid-template-columns:minmax(220px,.8fr) minmax(0,1.2fr);gap:36px;margin-block-start:32px;}.games-setup h3{font-size:var(--fs-l);margin:0 0 20px;}.games-options{display:flex;flex-direction:column;gap:18px;}.games-options h3{margin-bottom:0;}.games-options fieldset{padding:0;margin:0;border:0;}.games-options legend{margin-bottom:8px;font-size:var(--fs-m);}.games-segments{display:flex;border:1px solid var(--border);border-radius:var(--radius-m);overflow:hidden;}.games-segments button{flex:1;border:0;padding:10px;background:var(--surface);color:var(--text-dim);cursor:pointer;}.games-segments button[aria-pressed=true]{background:var(--accent-soft);color:var(--accent-text);}.games-options label{display:grid;gap:8px;}.games-options select,.games-options input{width:100%;min-height:36px;}.games-start{margin-top:8px;}.games-start :global(.btn.primary){width:100%;}.games-note{font-size:var(--fs-s);line-height:1.6;color:var(--text-dim);margin:0;}.games-environments>button{width:100%;display:flex;align-items:center;gap:16px;text-align:start;color:var(--text);border:1px solid var(--border);background:var(--surface);border-radius:var(--radius-m);padding:12px;margin-bottom:12px;cursor:pointer;}.games-environments>button.selected{border-color:var(--accent-solid);background:var(--accent-soft);}.games-environments strong{font-size:var(--fs-m);display:block;}.games-environments small{font-size:var(--fs-s);color:var(--text-dim);display:block;line-height:1.5;margin-top:6px;}.games-environments>button>span:last-child{margin-inline-start:auto;color:var(--accent-text);}.game-map-art{width:68px;height:60px;flex-shrink:0;border-radius:var(--radius-s);background:var(--info-soft);border:1px solid var(--border);position:relative;overflow:hidden;}.game-map-art::before,.game-map-art::after{content:'';position:absolute;background:var(--info);width:20px;height:35px;inset-block-start:20px;inset-inline-start:8px;transform:skewY(-15deg);opacity:.7;}.game-map-art::after{inset-block-start:10px;inset-inline-start:auto;inset-inline-end:8px;height:45px;}.game-map-art[data-map=foundry],.game-map-art[data-map=dunes]{background:var(--warning-soft);}.game-map-art[data-map=foundry]::before,.game-map-art[data-map=dunes]::before,.game-map-art[data-map=foundry]::after,.game-map-art[data-map=dunes]::after{background:var(--warning);}.game-map-art[data-map=forest]{background:var(--success-soft);}.game-map-art[data-map=forest]::before,.game-map-art[data-map=forest]::after{background:var(--success);border-radius:50%;}
 @media(max-width:640px){.games-titles{gap:12px;}.games-titles>button>div{padding:12px;}.games-titles h2{font-size:var(--fs-l);}.games-titles img{height:130px;}.games-setup{grid-template-columns:1fr;gap:24px;}.games-environments{grid-row:1;}.games-titles span{display:none;}}
</style>
