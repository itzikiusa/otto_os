<script lang="ts">
  import LoadState from '../../../lib/components/LoadState.svelte';
  import {onMount,untrack} from 'svelte';
  import type {GameConfig, GameInput, GameState} from './types';
  import {createGame, defaultInput, stepGame} from './simulation';
  import {GameControls} from './input';
  import {GameAudio} from './audio';
  import type {GameScene} from './scene';
  import RaceHud from './RaceHud.svelte';
  import CombatHud from './CombatHud.svelte';
  import {driverFor,type DriverId} from './roster';
  import {raceClock} from './race-hud';
  let {config, skin='azure', driver='fox', player=0, peerState=null, remoteInput=defaultInput(), paused=false, names=['You','Computer'], onframe, oninput, onfinish, onexit, onrematch, rematchPending=false}: {
    config:GameConfig; skin?:'azure'|'ember'; driver?:DriverId; player?:number; peerState?:GameState|null; remoteInput?:GameInput; paused?:boolean; names?:string[];
    onframe?:(state:GameState)=>void; oninput?:(input:GameInput)=>void; onfinish?:(state:GameState)=>void; onexit:()=>void; onrematch:()=>void; rematchPending?:boolean;
  } = $props();
  let stage:HTMLDivElement,playRoot:HTMLDivElement;
  let fullscreen=$state(false);
  async function toggleFullscreen(){try{if(document.fullscreenElement)await document.exitFullscreen();else await playRoot.requestFullscreen();}catch{/* Browser embedding can disable fullscreen; the game remains playable. */}}
  let scene:GameScene|undefined, controls:GameControls|undefined;
  let loading=$state(true),error=$state(''),active=$state(false),firstPerson=$state(false),muted=$state(false),menu=$state(false);
  let hud=$state.raw(createGame(untrack(()=>config))),reticle=$state({x:50,y:50});
  const audio=new GameAudio();
  let selectedWeapon=0,notice=$state(''),noticeUntil=0,hitMarker=$state(false),hitUntil=0;
  let stop:(()=>void)|undefined;
  let touchLook:{x:number;y:number}|null=null;
  function release(){controls?.release();}
  $effect(()=>{if(paused||menu)release();});
  async function activate(){try{await audio.start(config.kind);}catch{/* Game remains usable without audio device access. */}}
  function touch(event:PointerEvent,key:string,down:boolean){event.preventDefault();if(down){(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);void activate();}controls?.touch(key,down);}
  function look(event:PointerEvent){if(!touchLook)return;controls?.look(event.clientX-touchLook.x,event.clientY-touchLook.y);touchLook={x:event.clientX,y:event.clientY};}
  function initialise(){
    stop?.(); const abort=new AbortController();let frame=0,disposed=false,state=createGame(config),last=performance.now(),lastHud=0,lastSend=0,lastAudio=0,lastCount=4,finished=false;
    loading=true;error='';menu=false;state.players[player].character=driver;hud=state;
    void import('./scene').then(m=>m.createScene(stage,config,skin,abort.signal)).then(created=>{
      if(disposed){created.dispose();return;}scene=created;controls=new GameControls(created.canvas,config.kind==='shooter',v=>{queueMicrotask(()=>{if(!disposed)active=v;});},()=>{firstPerson=!firstPerson;});controls.yaw=state.players[player].yaw;
      loading=false;
      const diagnostic=stage as HTMLDivElement & {__ottoGame?:{snapshot:()=>GameState;stats:()=>ReturnType<GameScene['stats']>}};
      diagnostic.__ottoGame={snapshot:()=>structuredClone(state),stats:()=>created.stats()};
      const tick=(time:number)=>{
        if(disposed)return;const dt=Math.min(.1,Math.max(0,(time-last)/1000));last=time;
        if(player===1&&peerState)state=peerState;
        const held=controls?.read()??defaultInput();held.character=driver;if(held.weapon)selectedWeapon=held.weapon;else if(selectedWeapon)held.weapon=selectedWeapon;
        if(!paused&&(!menu||!config.vsComputer)&&!document.hidden){
          if(player===0){stepGame(state,[held,remoteInput],dt);onframe?.(state);}
          if(time-lastSend>=33){oninput?.(held);lastSend=time;}
        }else if(time-lastSend>=50){const neutral=defaultInput();neutral.yaw=held.yaw;neutral.pitch=held.pitch;oninput?.(neutral);lastSend=time;}
        const count=Math.ceil(state.countdown);if(count!==lastCount&&!paused&&!menu){audio.effect(count>0?'countdown':'go');lastCount=count;}
        created.render(state,player,firstPerson,dt,controls?.aiming()??false);reticle=created.reticle();
        audio.motor(config.kind==='kart'&&!paused&&!menu?state.players[player].speed:0);audio.activity(!paused&&!menu&&state.phase!=='finished',state.players[player].underwater);
        const soundedShots=new Set<number>();
        for(const e of state.events)if(e.id>lastAudio){
          if(e.type!=='shot'||!soundedShots.has(e.player))audio.effect(e.type,e.player===player?0:.5,state.players[e.player]?.weapon);
          if(e.type==='shot')soundedShots.add(e.player);
          if(e.player===player){
            if(e.type==='hit'){hitUntil=time+160;hitMarker=true;}
            const messages:Record<string,string>={trick:'Nice trick!',land:state.players[player].boost>0?'Landing boost!':'',lap:state.players[player].lap===2?'Final lap!':'Next lap!',pickup:config.kind==='shooter'?'Supplies collected':'Item ready · E',kill:'Elimination',shield:'Shield up!'};
            if(messages[e.type]){notice=messages[e.type];noticeUntil=time+1600;}
          }lastAudio=e.id;
        }
        if(time>noticeUntil)notice='';if(time>hitUntil)hitMarker=false;
        if(time-lastHud>70){hud={...state,players:[{...state.players[0]},{...state.players[1]}]};lastHud=time;}
        if(state.phase==='finished'&&!finished){finished=true;release();onfinish?.(state);}
        frame=requestAnimationFrame(tick);
      };
      frame=requestAnimationFrame(tick);
    }).catch(e=>{if(!disposed){loading=false;error=e instanceof Error?e.message:'The game could not start.';}});
    stop=()=>{disposed=true;abort.abort();cancelAnimationFrame(frame);controls?.dispose();controls=undefined;scene?.dispose();scene=undefined;delete (stage as HTMLDivElement & {__ottoGame?:unknown}).__ottoGame;};
  }
  onMount(()=>{const change=()=>{fullscreen=document.fullscreenElement===playRoot;};document.addEventListener('fullscreenchange',change);initialise();return()=>{document.removeEventListener('fullscreenchange',change);stop?.();audio.dispose();};});
  function retryLoad(){initialise();}
  const self=$derived(hud.players[player]);
  const timeLabel=$derived(`${Math.floor(hud.remaining/60)}:${Math.floor(hud.remaining%60).toString().padStart(2,'0')}`);
</script>
<div class="game-play" bind:this={playRoot} data-game={config.kind} data-map={config.map} data-phase={hud.phase} data-camera={firstPerson?'first':'third'}>
  <div class="game-stage" bind:this={stage} onpointerdown={activate} role="presentation"></div>
  {#if loading}<div class="game-overlay" role="status"><h2>Preparing {config.kind==='shooter'?'the arena':'the circuit'}…</h2><p>Loading characters and scenery</p></div>{/if}
  {#if error}<div class="game-overlay"><LoadState what="game" {error} empty onretry={retryLoad}/><button class="btn" onclick={onexit}>Back to games</button></div>{/if}
  {#if !loading&&!error}
    <div class="game-top-hud">
      <div class="game-score">{#if config.kind==='shooter'}<span>{names[0]}</span><strong>{config.kind==='shooter'?hud.players[0].score:Math.min(3,hud.players[0].lap+1)}</strong><span>{config.kind==='shooter'?'First to 7':'Lap / 3'}</span><strong>{config.kind==='shooter'?hud.players[1].score:Math.min(3,hud.players[1].lap+1)}</strong><span>{names[1]}</span><time>{timeLabel}</time>{:else}<span>Circuit Clash</span><strong>{driverFor(driver).name}</strong>{/if}</div>
      <div class="game-tools">
        {#if config.kind==='shooter'}<button class="btn small" onclick={()=>{firstPerson=!firstPerson;release();}}>{firstPerson?'First person':'Third person'}</button>{/if}
        <button class="btn small" aria-label={muted?'Unmute game':'Mute game'} title={muted?'Unmute game':'Mute game'} onclick={()=>{muted=!muted;audio.mute(muted);}}>{muted?'Sound off':'Sound on'}</button>
        {#if typeof document!=='undefined'&&document.fullscreenEnabled}<button class="btn small" onclick={toggleFullscreen}>{fullscreen?'Exit full screen':'Full screen'}</button>{/if}
        <button class="btn small" onclick={()=>{menu=!menu;release();}}>Menu</button>
      </div>
    </div>
    {#if config.kind==='shooter'&&hud.phase==='playing'&&self.hp>0&&!menu}<div class="game-crosshair" style:inset-inline-start={`${reticle.x}%`} style:inset-block-start={`${reticle.y}%`} aria-hidden="true">+</div>{/if}
    {#if config.kind==='kart'}<RaceHud state={hud} {player} {names}/>{:else}<CombatHud {self} onweapon={slot=>{selectedWeapon=slot;scene?.canvas.focus();}}/>{/if}
    {#if self.damageTime>0&&config.kind==='shooter'}<div class="game-damage" style:opacity={Math.min(.8,self.damageTime*1.5)} aria-hidden="true"></div>{/if}
    {#if hitMarker&&config.kind==='shooter'}<div class="game-hit-marker" style:inset-inline-start={`${reticle.x}%`} style:inset-block-start={`${reticle.y}%`} aria-hidden="true">×</div>{/if}
    {#if notice}<div class="game-announcement" aria-live="polite">{notice}</div>{/if}
    {#if hud.phase==='countdown'&&!paused}<div class="game-countdown" aria-live="polite">{Math.max(1,Math.ceil(hud.countdown))}</div>{/if}
    {#if hud.phase==='playing'&&self.hp<=0&&config.kind==='shooter'}<div class="game-centre-note">Respawning in {Math.max(1,Math.ceil(self.respawnTime))}…</div>{/if}
    {#if !active&&!menu&&hud.phase==='playing'&&!paused&&self.hp>0}<div class="game-control-hint">Click the game to {config.kind==='shooter'?'aim and move':'drive'} · {config.kind==='shooter'?'WASD move · Click fire · 1/2/3 weapons · E dash · V camera':'WASD drive · Space drift / air trick · E item · R recover'}</div>{/if}
    {#if paused}<div class="game-overlay" role="status"><h2>Waiting for connection</h2><p>The match is paused while your opponent reconnects.</p><button class="btn" onclick={onexit}>Leave match</button></div>
    {:else if hud.phase==='finished'}<div class="game-overlay game-results" aria-live="polite"><p>{config.kind==='kart'?'Finish line':'Match complete'}</p>{#if config.kind==='kart'&&hud.winner!==null}<img class="game-winner" src={`/room-games/driver-${hud.players[hud.winner].character}.png`} alt={driverFor(hud.players[hud.winner].character).name}/>{/if}<h2>{hud.winner===null?'A draw':hud.winner===player?'You win!':`${names[hud.winner]} wins`}</h2><p>{config.kind==='shooter'?`${hud.players[0].score} — ${hud.players[1].score}`:`Race time ${raceClock(hud.elapsed)}`}</p><button class="btn primary" disabled={rematchPending} onclick={onrematch}>{rematchPending?'Waiting for opponent…':'Rematch'}</button><button class="btn" onclick={onexit}>Back to games</button></div>
    {:else if menu}<div class="game-overlay"><h2>{config.kind==='shooter'?'Arena Duel':'Circuit Clash'}</h2><p>{config.vsComputer?'Game paused':'Your player is idle; the match continues for your opponent.'}</p><p>{config.kind==='shooter'?'WASD move · V camera · Shift sprint · Space jump · 1/2/3 weapons · E dash · Click fire · Right click aim · R reload':'WASD drive · Space drift / airborne trick · Shift release boost · E use item · R recover'}</p><button class="btn primary" onclick={()=>{menu=false;scene?.canvas.focus();}}>Resume</button><button class="btn" onclick={onexit}>Leave match</button></div>{/if}
    <div class="game-touch-controls" aria-label="Touch controls">
      <div class="game-touch-movement">{#each [['ArrowLeft','←'],['ArrowUp','↑'],['ArrowDown','↓'],['ArrowRight','→']] as [key,label]}<button aria-label={`Move ${key.slice(5).toLowerCase()}`} onpointerdown={e=>touch(e,key,true)} onpointerup={e=>touch(e,key,false)} onpointercancel={e=>touch(e,key,false)}>{label}</button>{/each}</div>
      {#if config.kind==='shooter'}<div class="game-look-pad" role="application" aria-label="Drag to aim" onpointerdown={e=>{(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);touchLook={x:e.clientX,y:e.clientY};}} onpointermove={look} onpointerup={()=>touchLook=null} onpointercancel={()=>touchLook=null}>Look</div>{/if}
      <button onpointerdown={e=>touch(e,config.kind==='shooter'?'Fire':'Space',true)} onpointerup={e=>touch(e,config.kind==='shooter'?'Fire':'Space',false)} onpointercancel={e=>touch(e,config.kind==='shooter'?'Fire':'Space',false)}>{config.kind==='shooter'?'Fire':'Drift'}</button>
      <button onpointerdown={e=>touch(e,config.kind==='shooter'?'Space':'KeyE',true)} onpointerup={e=>touch(e,config.kind==='shooter'?'Space':'KeyE',false)} onpointercancel={e=>touch(e,config.kind==='shooter'?'Space':'KeyE',false)}>{config.kind==='shooter'?'Jump':'Item'}</button>
      {#if config.kind==='shooter'}<button onpointerdown={e=>touch(e,'KeyE',true)} onpointerup={e=>touch(e,'KeyE',false)} onpointercancel={e=>touch(e,'KeyE',false)}>Dash</button>{/if}
    </div>
  {/if}
</div>
<style>
  .game-announcement{position:absolute;inset-block-start:26%;inset-inline:12px;text-align:center;font-size:clamp(24px,4vw,42px);font-weight:600;font-style:italic;color:var(--on-scrim);text-shadow:0 3px 8px var(--scrim-media);pointer-events:none;}.game-damage{position:absolute;inset:0;box-shadow:inset 0 0 110px 25px var(--danger);pointer-events:none;}.game-hit-marker{position:absolute;transform:translate(-50%,-50%);font-size:calc(var(--fs-hero) * 1.4);font-weight:600;color:var(--on-scrim);text-shadow:0 1px 3px var(--scrim-media);pointer-events:none;}
  .game-play {position:relative;flex:1;min-height:400px;overflow:hidden;background:var(--surface);isolation:isolate;}
  .game-play:fullscreen{width:100vw;height:100vh;}.game-winner{width:160px;height:160px;object-fit:contain;}
  .game-stage {position:absolute;inset:0;} .game-stage :global(canvas){display:block;width:100%;height:100%;outline-offset:-3px;}
  .game-overlay{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px;padding:24px;text-align:center;background:color-mix(in srgb,var(--surface) 92%,transparent);z-index:5;}
  .game-overlay h2{font-size:var(--fs-2xl);margin:0;} .game-overlay p{max-width:580px;margin:0;color:var(--text-dim);line-height:1.6;}
  .game-top-hud{position:absolute;inset-block-start:16px;inset-inline:16px;display:flex;justify-content:space-between;gap:12px;align-items:flex-start;pointer-events:none;}
  .game-score,.game-tools{background:var(--scrim-media);color:var(--on-scrim);border:1px solid color-mix(in srgb,var(--on-scrim) 18%,transparent);border-radius:var(--radius-m);padding:10px 14px;}
  .game-score{display:flex;align-items:center;gap:14px;font-size:var(--fs-s);} .game-score strong{font-size:var(--fs-xl);} .game-score span{max-width:130px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
  .game-score time{font-variant-numeric:tabular-nums;margin-inline-start:8px;}.game-tools{display:flex;gap:6px;pointer-events:auto;padding:6px;}
  .game-tools :global(.btn){background:color-mix(in srgb,var(--on-scrim) 14%,transparent);color:var(--on-scrim);border-color:color-mix(in srgb,var(--on-scrim) 25%,transparent);}.game-tools :global(.btn:hover){background:color-mix(in srgb,var(--on-scrim) 26%,transparent);}
  .game-countdown,.game-centre-note,.game-crosshair{position:absolute;inset:0;display:grid;place-items:center;pointer-events:none;color:var(--text);text-shadow:0 1px 3px var(--surface);}.game-countdown{font-size:clamp(60px,12vw,120px);font-weight:600;}.game-crosshair{inset:auto;transform:translate(-50%,-50%);font-size:var(--fs-2xl);color:var(--on-scrim);}.game-centre-note{font-size:var(--fs-xl);}
  .game-control-hint{position:absolute;inset-block-end:180px;inset-inline:20px;margin-inline:auto;max-width:620px;text-align:center;background:var(--scrim-media);color:var(--on-scrim);border-radius:var(--radius-s);padding:8px;font-size:var(--fs-s);pointer-events:none;}
  .game-touch-controls{display:none;}
  @media(max-width:1024px){.game-top-hud{inset:8px 8px auto;flex-wrap:wrap;}.game-score{gap:8px;}}
  @media(pointer:coarse){.game-touch-controls{position:absolute;inset-inline:12px;inset-block-end:90px;display:flex;gap:8px;align-items:center;justify-content:space-between;}.game-touch-controls button,.game-look-pad{touch-action:none;min-width:46px;min-height:48px;border-radius:var(--radius-m);border:1px solid var(--border);background:var(--surface);color:var(--text);font-size:var(--fs-m);}.game-touch-movement{display:grid;grid-template-columns:repeat(2,46px);gap:4px;}.game-look-pad{display:grid;place-items:center;min-height:85px;min-width:80px;}.game-control-hint{display:none;}}
</style>
