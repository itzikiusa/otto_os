<script lang="ts">
  import {onMount} from 'svelte';
  import GameMatch from './GameMatch.svelte';
  import {consumeGameInvite} from './access';
  import type {GameRoomCredential} from '../../../lib/api/game-room-types';
  let {roomId}:{roomId:string}=$props();
  let name=$state(''),invite=$state(''),error=$state(''),busy=$state(false),ended=$state(false),credential=$state<GameRoomCredential|null>(null);
  onMount(()=>{invite=consumeGameInvite(roomId)??'';if(!invite)error='This invitation is missing. Open a fresh game invitation from the host.';});
  async function join(){
    busy=true;error='';
    try{const r=await fetch(new URL('/api/v1/game-room-join',location.origin),{method:'POST',credentials:'omit',referrerPolicy:'no-referrer',headers:{'Content-Type':'application/json'},body:JSON.stringify({room_id:roomId,invite,name:name.trim()})});if(!r.ok){const problem=await r.json().catch(()=>({}));throw new Error(problem.message??'Could not join this match. Ask the host for a fresh invitation.');}credential=await r.json() as GameRoomCredential;invite='';}
    catch(e){error=e instanceof Error?e.message:'Could not join the game.';}finally{busy=false;}
  }
</script>
<div class="game-guest">
 {#if ended}<div class="game-join"><h1>You left the match</h1><p>You can close this tab.</p></div>
 {:else if credential}<GameMatch {credential} origin={location.origin} onexit={()=>{ended=true;credential=null;}}/>
 {:else}<form class="game-join" onsubmit={e=>{e.preventDefault();void join();}}><h1>Join a game</h1><p>Play a one-on-one match in Otto Rooms.</p><p>Your display name and game activity will be shared with <strong>{location.host}</strong>.</p><label>Your display name<input dir="auto" bind:value={name} maxlength="40" autocomplete="nickname" required/></label>{#if error}<p role="alert">{error}</p>{/if}<button class="btn primary" disabled={busy||!invite||!name.trim()}>{busy?'Joining…':'Join game'}</button></form>{/if}
</div>
<style>
 .game-guest{display:flex;flex-direction:column;height:100dvh;min-height:450px;background:var(--surface);color:var(--text);}.game-join{margin:auto;width:min(100%,480px);padding:28px;display:flex;flex-direction:column;gap:18px;box-sizing:border-box;}.game-join h1{font-size:var(--fs-2xl);margin:0;}.game-join p{color:var(--text-dim);line-height:1.6;margin:0;overflow-wrap:anywhere;}.game-join label{display:grid;gap:8px;}.game-join input{width:100%;}.game-join [role=alert]{color:var(--danger);}
</style>
