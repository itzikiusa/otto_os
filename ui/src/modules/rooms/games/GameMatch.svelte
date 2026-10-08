<script lang="ts">
  import {onMount} from 'svelte';
  import GamePlay from './GamePlay.svelte';
  import {DRIVERS,type DriverId} from './roster';
  import {GameClient,decodeInput,decodeSnapshot,type GameConnection} from './client';
  import {FinishDelivery} from './finish-delivery';
  import {defaultInput} from './simulation';
  import type {GameRoomCredential,GameRoomEvent,GameRoomState} from '../../../lib/api/game-room-types';
  import type {GameConfig,GameEvent,GameInput,GameState} from './types';
  let {credential,origin,onexit,skin='azure',driver='fox'}: {credential:GameRoomCredential;origin:string;onexit:()=>void;skin?:'azure'|'ember';driver?:DriverId}=$props();
  const config:GameConfig=$derived({kind:credential.config.game,map:credential.config.map,difficulty:'normal',vsComputer:false});
  let client:GameClient,room=$state<GameRoomState|null>(null),connection=$state<GameConnection>('connecting'),error=$state(''),closed=$state(''),peer=$state.raw<GameState|null>(null),remote=$state.raw<GameInput>(defaultInput()),rematchPending=$state(false),copied=$state(false);
  const delivery=new FinishDelivery();
  let seq=0,lastSend=0,lastInput=0,lastReceived=0,events:GameEvent[]=[];
  let terminal:{winner:number|null;scores:number[];elapsed:number}|null=null;
  const player=$derived(credential.role==='host'?0:1);
  const names=$derived([room?.members.find(m=>m.role==='host')?.name??'Host',room?.members.find(m=>m.role==='guest')?.name??'Opponent']);
  const self=$derived(room?.members.find(m=>m.id===credential.member_id));
  const ready=$derived(room?.members.length===2&&room.members.every(m=>m.connected&&m.ready));
  function handle(event:GameRoomEvent){
    if(event.type==='state'){
      if(event.room.id!==credential.room_id)return;
      if(!room||event.room.round!==room.round||event.room.generation!==room.generation){seq=0;lastReceived=0;lastInput=0;remote=defaultInput();events=[];}
      if(room&&event.room.round!==room.round){peer=null;terminal=null;rematchPending=false;}
      delivery.observe(event.room);room=event.room;error='';return;
    }
    if(event.type==='closed'){closed=event.reason;return;}
    if(event.type==='error'){if(!event.message.includes('Stale game round'))error=event.message;rematchPending=false;return;}
    if(!room||!('round' in event)||event.round!==room.round||event.generation!==room.generation)return;
    if(event.type==='snapshot'&&player===1&&event.seq>lastReceived){const decoded=decodeSnapshot(event.data,config);if(decoded){peer=decoded;lastReceived=event.seq;}}
    if(event.type==='input'&&player===0&&event.seq>lastReceived){const input=decodeInput(event.data);if(input){remote=input;lastReceived=event.seq;lastInput=performance.now();}}
    if(event.type==='finished'){
      const r=event.result as {winner?:unknown;scores?:unknown;elapsed?:unknown}|null;
      if(r&&[null,0,1].includes(r.winner as number|null)&&Array.isArray(r.scores)&&r.scores.length===2&&r.scores.every(n=>typeof n==='number'&&Number.isFinite(n))&&typeof r.elapsed==='number'&&Number.isFinite(r.elapsed)){
        terminal={winner:r.winner as number|null,scores:r.scores,elapsed:r.elapsed};
        if(peer)peer={...peer,phase:'finished',winner:terminal.winner,elapsed:terminal.elapsed,players:[{...peer.players[0],score:terminal.scores[0]},{...peer.players[1],score:terminal.scores[1]}]};
      }
    }
  }
  onMount(()=>{
    client=new GameClient(origin,credential,handle,status=>{connection=status;if(status!=='connected')remote=defaultInput();});client.connect();
    return()=>{client.dispose();};
  });
  function input(value:GameInput){if(player===1&&room?.phase==='playing'&&!room.paused)client.send({type:'input',round:room.round,generation:room.generation,seq:++seq,data:value});}
  function snapshot(state:GameState){
    if(lastInput&&performance.now()-lastInput>250){const neutral=defaultInput();neutral.yaw=remote.yaw;neutral.pitch=remote.pitch;remote=neutral;}
    if(!room||room.phase!=='playing'||room.paused)return;
    events.push(...state.events);events=events.slice(-32);
    if(performance.now()-lastSend<50)return;
    lastSend=performance.now();
    if(client.send({type:'snapshot',round:room.round,generation:room.generation,seq:++seq,data:{...state,events}}))events=[];
    if(state.phase==='finished')finish(state);
  }
  function finish(state:GameState){if(player!==0||!room)return;delivery.queue({winner:state.winner,scores:state.players.map(p=>p.score),elapsed:state.elapsed});const command=delivery.command(room,performance.now());if(command)client.send(command);}
  function rematch(){if(client.send({type:'rematch'}))rematchPending=true;}
  async function copyInvite(){try{await navigator.clipboard.writeText(credential.invite_url??'');copied=true;}catch{error='Copy the invitation from the field below.';}}
</script>
<div class="game-match">
  {#if closed}<div class="game-wait" role="status"><h2>Match ended</h2><p>{closed}</p><button class="btn primary" onclick={onexit}>Back to games</button></div>
  {:else if room&&(room.phase==='playing'||room.phase==='finished')}
    {#key room.round}<GamePlay {config} {skin} {driver} {player} {names} peerState={peer} remoteInput={remote} paused={connection!=='connected'||room.paused} onframe={snapshot} oninput={input} onfinish={finish} {onexit} onrematch={rematch} {rematchPending}/>{/key}
  {:else}<div class="game-wait">
    <h2>{config.kind==='shooter'?'Arena Duel':'Circuit Clash'}</h2>
    <p>{connection==='connected'?'Get ready for a one-on-one match.':connection==='connecting'?'Connecting to game room…':'Connection lost. Reconnecting…'}</p>
    {#if credential.invite_url}<div class="game-invite"><label for="game-invite">Invite your opponent</label><input dir="ltr" id="game-invite" readonly value={credential.invite_url} aria-label="Game invitation"/><button class="btn" onclick={copyInvite}>{copied?'Copied':'Copy invitation'}</button><p>For another computer, use the reachable HTTPS origin configured in Rooms connection settings.</p></div>{/if}
    {#if config.kind==='kart'}<div class="game-wait-drivers" aria-label="Choose your driver">{#each DRIVERS as racer}<button aria-pressed={driver===racer.id} onclick={()=>driver=racer.id}><img src={`/room-games/driver-${racer.id}.png`} alt={racer.species}/><span>{racer.name}</span></button>{/each}</div>{/if}
    <div class="game-members">{#each room?.members??[] as member (member.id)}<p><strong>{member.name}</strong><span>{!member.connected?'Connecting':member.ready?'Ready':'Not ready'}</span></p>{/each}{#if (room?.members.length??0)<2}<p>Waiting for opponent…</p>{/if}</div>
    <div class="game-wait-actions"><button class="btn" disabled={connection!=='connected'||!room} onclick={()=>client.send({type:'ready',ready:!self?.ready})}>{self?.ready?'Not ready':'Ready'}</button>{#if player===0}<button class="btn primary" disabled={!ready||connection!=='connected'} onclick={()=>room&&client.send({type:'start',round:room.round,generation:room.generation})}>Start match</button>{/if}<button class="btn" onclick={onexit}>Leave</button></div>
  </div>{/if}
  {#if error}<p class="game-network-error" role="alert">{error}</p>{/if}
</div>
<style>
 .game-wait-drivers{display:flex;gap:8px;max-width:100%;}.game-wait-drivers button{display:grid;gap:4px;border:1px solid var(--border);border-radius:var(--radius-m);background:var(--surface);color:var(--text);padding:6px;cursor:pointer;}.game-wait-drivers button[aria-pressed=true]{border-color:var(--accent-solid);background:var(--accent-soft);}.game-wait-drivers img{width:64px;max-width:100%;aspect-ratio:1;border-radius:var(--radius-s);}.game-wait-drivers span{font-size:var(--fs-s);}
 .game-match{position:relative;display:flex;flex-direction:column;flex:1;min-height:450px;}.game-wait{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;padding:28px;text-align:center;}.game-wait h2{font-size:var(--fs-2xl);margin:0;}.game-wait p{color:var(--text-dim);margin:0;line-height:1.5;}.game-invite{width:min(100%,600px);display:grid;gap:10px;text-align:start;}.game-invite input{width:100%;min-width:0;}.game-invite p{font-size:var(--fs-s);}.game-members{width:min(100%,400px);}.game-members p{display:flex;justify-content:space-between;border-block-end:1px solid var(--border);padding:12px;gap:20px;}.game-wait-actions{display:flex;flex-wrap:wrap;gap:8px;}.game-network-error{position:absolute;inset:80px 20px auto;z-index:8;background:var(--danger-soft);color:var(--danger);padding:12px;border-radius:var(--radius-m);}
</style>
