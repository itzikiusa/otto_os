<script lang="ts">
  import LoadState from '../../../lib/components/LoadState.svelte';
  import {onMount,untrack} from 'svelte';
  import type {GameConfig, GameInput, GameState} from './types';
  import {createGame, defaultInput, stepGame} from './simulation';
  import {GameControls} from './input';
  import {GameAudio} from './audio';
  import type {GameScene} from './scene';
  let {config, skin='azure', player=0, peerState=null, remoteInput=defaultInput(), paused=false, names=['You','Computer'], onframe, oninput, onfinish, onexit, onrematch, rematchPending=false}: {
    config:GameConfig; skin?:'azure'|'ember'; player?:number; peerState?:GameState|null; remoteInput?:GameInput; paused?:boolean; names?:string[];
    onframe?:(state:GameState)=>void; oninput?:(input:GameInput)=>void; onfinish?:(state:GameState)=>void; onexit:()=>void; onrematch:()=>void; rematchPending?:boolean;
  } = $props();
  let stage:HTMLDivElement;
  let scene:GameScene|undefined, controls:GameControls|undefined;
  let loading=$state(true),error=$state(''),active=$state(false),firstPerson=$state(false),muted=$state(false),menu=$state(false);
  let hud=$state.raw(createGame(untrack(()=>config))), frameRate=$state(0),reticle=$state({x:50,y:50});
  const audio=new GameAudio();
  let stop:(()=>void)|undefined;
  let touchLook:{x:number;y:number}|null=null;
  function release(){controls?.release();}
  $effect(()=>{if(paused||menu)release();});
  async function activate(){try{await audio.start();}catch{/* Game remains usable without audio device access. */}}
  function touch(event:PointerEvent,key:string,down:boolean){event.preventDefault();if(down){(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);void activate();}controls?.touch(key,down);}
  function look(event:PointerEvent){if(!touchLook)return;controls?.look(event.clientX-touchLook.x,event.clientY-touchLook.y);touchLook={x:event.clientX,y:event.clientY};}
  function initialise(){
    stop?.(); const abort=new AbortController();let frame=0,disposed=false,state=createGame(config),last=performance.now(),lastHud=0,lastSend=0,lastAudio=0,finished=false,frames=0,lastFps=last;
    loading=true;error='';menu=false;hud=state;
    void import('./scene').then(m=>m.createScene(stage,config,skin,abort.signal)).then(created=>{
      if(disposed){created.dispose();return;}scene=created;controls=new GameControls(created.canvas,config.kind==='shooter',v=>{queueMicrotask(()=>{if(!disposed)active=v;});},()=>{firstPerson=!firstPerson;});controls.yaw=state.players[player].yaw;
      loading=false;
      const diagnostic=stage as HTMLDivElement & {__ottoGame?:{snapshot:()=>GameState;stats:()=>ReturnType<GameScene['stats']>}};
      diagnostic.__ottoGame={snapshot:()=>structuredClone(state),stats:()=>created.stats()};
      const tick=(time:number)=>{
        if(disposed)return;const dt=Math.min(.1,Math.max(0,(time-last)/1000));last=time;frames++;
        if(time-lastFps>1000){frameRate=Math.round(frames*1000/(time-lastFps));frames=0;lastFps=time;}
        if(player===1&&peerState)state=peerState;
        const held=controls?.read()??defaultInput();
        if(!paused&&(!menu||!config.vsComputer)&&!document.hidden){
          if(player===0){stepGame(state,[held,remoteInput],dt);onframe?.(state);}
          if(time-lastSend>=33){oninput?.(held);lastSend=time;}
        }else if(time-lastSend>=50){const neutral=defaultInput();neutral.yaw=held.yaw;neutral.pitch=held.pitch;oninput?.(neutral);lastSend=time;}
        created.render(state,player,firstPerson,dt,controls?.aiming()??false);reticle=created.reticle();
        audio.motor(config.kind==='kart'&&!paused&&!menu?state.players[player].speed:0);
        for(const e of state.events)if(e.id>lastAudio){audio.effect(e.type,e.player===player?0:.5);lastAudio=e.id;}
        if(time-lastHud>70){hud={...state,players:[{...state.players[0]},{...state.players[1]}]};lastHud=time;}
        if(state.phase==='finished'&&!finished){finished=true;release();onfinish?.(state);}
        frame=requestAnimationFrame(tick);
      };
      frame=requestAnimationFrame(tick);
    }).catch(e=>{if(!disposed){loading=false;error=e instanceof Error?e.message:'The game could not start.';}});
    stop=()=>{disposed=true;abort.abort();cancelAnimationFrame(frame);controls?.dispose();controls=undefined;scene?.dispose();scene=undefined;delete (stage as HTMLDivElement & {__ottoGame?:unknown}).__ottoGame;};
  }
  onMount(()=>{initialise();return()=>{stop?.();audio.dispose();};});
  function retryLoad(){initialise();}
  const self=$derived(hud.players[player]);
  const timeLabel=$derived(`${Math.floor(hud.remaining/60)}:${Math.floor(hud.remaining%60).toString().padStart(2,'0')}`);
</script>
<div class="game-play" data-game={config.kind} data-map={config.map} data-phase={hud.phase} data-camera={firstPerson?'first':'third'}>
  <div class="game-stage" bind:this={stage} onpointerdown={activate} role="presentation"></div>
  {#if loading}<div class="game-overlay" role="status"><h2>Preparing {config.kind==='shooter'?'the arena':'the circuit'}…</h2><p>Loading characters and scenery</p></div>{/if}
  {#if error}<div class="game-overlay"><LoadState what="game" {error} empty onretry={retryLoad}/><button class="btn" onclick={onexit}>Back to games</button></div>{/if}
  {#if !loading&&!error}
    <div class="game-top-hud">
      <div class="game-score"><span>{names[0]}</span><strong>{config.kind==='shooter'?hud.players[0].score:Math.min(3,hud.players[0].lap+1)}</strong><span>{config.kind==='shooter'?'First to 7':'Lap / 3'}</span><strong>{config.kind==='shooter'?hud.players[1].score:Math.min(3,hud.players[1].lap+1)}</strong><span>{names[1]}</span></div>
      <div class="game-tools">
        {#if config.kind==='shooter'}<button class="btn small" onclick={()=>{firstPerson=!firstPerson;release();}}>{firstPerson?'First person':'Third person'}</button>{/if}
        <button class="btn small" aria-label={muted?'Unmute game':'Mute game'} title={muted?'Unmute game':'Mute game'} onclick={()=>{muted=!muted;audio.mute(muted);}}>{muted?'Sound off':'Sound on'}</button>
        <button class="btn small" onclick={()=>{menu=!menu;release();}}>Menu</button>
      </div>
    </div>
    {#if config.kind==='shooter'&&hud.phase==='playing'&&self.hp>0&&!menu}<div class="game-crosshair" style:inset-inline-start={`${reticle.x}%`} style:inset-block-start={`${reticle.y}%`} aria-hidden="true">+</div>{/if}
    <div class="game-bottom-hud">
      {#if config.kind==='shooter'}<div><span>Health</span><strong>{self.hp}</strong><progress aria-label="Health" value={self.hp} max="100"></progress></div><div><span>{self.reloadTime>0?'Reloading':'Ammo'}</span><strong>{self.ammo}<small> / 24</small></strong></div><div><span>Time</span><strong>{timeLabel}</strong></div>
      {:else}<div><span>Speed</span><strong>{Math.round(Math.abs(self.speed)*3.6)}<small> km/h</small></strong></div><div><span>{self.boost>0?'Boost active':'Drift charge'}</span><progress aria-label="Drift charge" value={self.driftCharge} max="1.5"></progress><strong>{self.offTrack?'Off track':'On circuit'}</strong></div><div><span>Item · E</span><strong>{self.item==='boost'?'Turbo':self.item==='pulse'?'Pulse':'Empty'}</strong></div>{/if}
      <span class="game-fps">{frameRate} fps</span>
    </div>
    {#if hud.phase==='countdown'&&!paused}<div class="game-countdown" aria-live="polite">{Math.max(1,Math.ceil(hud.countdown))}</div>{/if}
    {#if hud.phase==='playing'&&self.hp<=0&&config.kind==='shooter'}<div class="game-centre-note">Respawning in {Math.max(1,Math.ceil(self.respawnTime))}…</div>{/if}
    {#if !active&&!menu&&hud.phase==='playing'&&!paused&&self.hp>0}<div class="game-control-hint">Click the game to {config.kind==='shooter'?'aim and move':'drive'} · {config.kind==='shooter'?'WASD move · Click fire · R reload · Space jump · V camera':'WASD drive · Space drift · E item · R recover'}</div>{/if}
    {#if paused}<div class="game-overlay" role="status"><h2>Waiting for connection</h2><p>The match is paused while your opponent reconnects.</p><button class="btn" onclick={onexit}>Leave match</button></div>
    {:else if hud.phase==='finished'}<div class="game-overlay game-results" aria-live="polite"><p>Match complete</p><h2>{hud.winner===null?'A draw':hud.winner===player?'You win!':`${names[hud.winner]} wins`}</h2><p>{config.kind==='shooter'?`${hud.players[0].score} — ${hud.players[1].score}`:`Race time ${hud.elapsed.toFixed(1)} seconds`}</p><button class="btn primary" disabled={rematchPending} onclick={onrematch}>{rematchPending?'Waiting for opponent…':'Rematch'}</button><button class="btn" onclick={onexit}>Back to games</button></div>
    {:else if menu}<div class="game-overlay"><h2>{config.kind==='shooter'?'Arena Duel':'Circuit Clash'}</h2><p>{config.vsComputer?'Game paused':'Your player is idle; the match continues for your opponent.'}</p><p>{config.kind==='shooter'?'WASD move · V camera · Shift sprint · Space jump · Click fire · Right click aim · R reload':'WASD drive · Space drift · Shift release boost · E use item · R recover'}</p><button class="btn primary" onclick={()=>{menu=false;scene?.canvas.focus();}}>Resume</button><button class="btn" onclick={onexit}>Leave match</button></div>{/if}
    <div class="game-touch-controls" aria-label="Touch controls">
      <div class="game-touch-movement">{#each [['ArrowLeft','←'],['ArrowUp','↑'],['ArrowDown','↓'],['ArrowRight','→']] as [key,label]}<button aria-label={`Move ${key.slice(5).toLowerCase()}`} onpointerdown={e=>touch(e,key,true)} onpointerup={e=>touch(e,key,false)} onpointercancel={e=>touch(e,key,false)}>{label}</button>{/each}</div>
      {#if config.kind==='shooter'}<div class="game-look-pad" role="application" aria-label="Drag to aim" onpointerdown={e=>{(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);touchLook={x:e.clientX,y:e.clientY};}} onpointermove={look} onpointerup={()=>touchLook=null} onpointercancel={()=>touchLook=null}>Look</div>{/if}
      <button onpointerdown={e=>touch(e,config.kind==='shooter'?'Fire':'Space',true)} onpointerup={e=>touch(e,config.kind==='shooter'?'Fire':'Space',false)} onpointercancel={e=>touch(e,config.kind==='shooter'?'Fire':'Space',false)}>{config.kind==='shooter'?'Fire':'Drift'}</button>
      <button onpointerdown={e=>touch(e,config.kind==='shooter'?'Space':'KeyE',true)} onpointerup={e=>touch(e,config.kind==='shooter'?'Space':'KeyE',false)} onpointercancel={e=>touch(e,config.kind==='shooter'?'Space':'KeyE',false)}>{config.kind==='shooter'?'Jump':'Item'}</button>
    </div>
  {/if}
</div>
<style>
  .game-play {position:relative;flex:1;min-height:400px;overflow:hidden;background:var(--surface);isolation:isolate;}
  .game-stage {position:absolute;inset:0;} .game-stage :global(canvas){display:block;width:100%;height:100%;outline-offset:-3px;}
  .game-overlay{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px;padding:24px;text-align:center;background:color-mix(in srgb,var(--surface) 92%,transparent);z-index:5;}
  .game-overlay h2{font-size:var(--fs-2xl);margin:0;} .game-overlay p{max-width:580px;margin:0;color:var(--text-dim);line-height:1.6;}
  .game-top-hud{position:absolute;inset-block-start:16px;inset-inline:16px;display:flex;justify-content:space-between;gap:12px;align-items:flex-start;pointer-events:none;}
  .game-score,.game-tools,.game-bottom-hud{background:var(--scrim-media);color:var(--on-scrim);border:1px solid color-mix(in srgb,var(--on-scrim) 18%,transparent);border-radius:var(--radius-m);padding:10px 14px;}
  .game-score{display:flex;align-items:center;gap:14px;font-size:var(--fs-s);} .game-score strong{font-size:var(--fs-xl);} .game-score span{max-width:130px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
  .game-tools{display:flex;gap:6px;pointer-events:auto;padding:6px;}
  .game-bottom-hud{position:absolute;inset-block-end:18px;inset-inline-start:18px;display:flex;gap:24px;align-items:center;pointer-events:none;}
  .game-bottom-hud>div{display:grid;gap:4px;min-width:75px;}.game-bottom-hud span{font-size:var(--fs-s);color:var(--on-scrim);opacity:.8;}.game-bottom-hud strong{font-size:var(--fs-xl);}.game-bottom-hud small{font-size:var(--fs-s);font-weight:normal;}
  progress{width:90px;height:6px;accent-color:var(--accent-solid);}.game-fps{font-variant-numeric:tabular-nums;}
  .game-countdown,.game-centre-note,.game-crosshair{position:absolute;inset:0;display:grid;place-items:center;pointer-events:none;color:var(--text);text-shadow:0 1px 3px var(--surface);}.game-countdown{font-size:clamp(60px,12vw,120px);font-weight:600;}.game-crosshair{inset:auto;transform:translate(-50%,-50%);font-size:var(--fs-2xl);color:var(--on-scrim);}.game-centre-note{font-size:var(--fs-xl);}
  .game-control-hint{position:absolute;inset-block-end:115px;inset-inline:20px;text-align:center;background:var(--surface);border-radius:var(--radius-s);padding:8px;font-size:var(--fs-s);pointer-events:none;}
  .game-touch-controls{display:none;}
  @media(max-width:1024px){.game-top-hud{inset:8px 8px auto;flex-wrap:wrap;}.game-score{gap:8px;}.game-bottom-hud{inset-inline:8px;inset-block-end:8px;gap:12px;padding:8px;}.game-bottom-hud strong{font-size:var(--fs-l);}.game-bottom-hud>div{min-width:60px;}.game-fps{display:none;}}
  @media(pointer:coarse){.game-touch-controls{position:absolute;inset-inline:12px;inset-block-end:90px;display:flex;gap:8px;align-items:center;justify-content:space-between;}.game-touch-controls button,.game-look-pad{touch-action:none;min-width:46px;min-height:48px;border-radius:var(--radius-m);border:1px solid var(--border);background:var(--surface);color:var(--text);font-size:var(--fs-m);}.game-touch-movement{display:grid;grid-template-columns:repeat(2,46px);gap:4px;}.game-look-pad{display:grid;place-items:center;min-height:85px;min-width:80px;}.game-control-hint{display:none;}}
</style>
