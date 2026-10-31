///<reference types="svelte" />
;
import {onMount} from 'svelte';
import {announceShown} from '../../../lib/announce.svelte';
import GameMatch from './GameMatch.svelte';
import {consumeGameInvite} from './access';
import type {GameRoomCredential} from '../../../lib/api/game-room-types';

;type $$ComponentProps = {roomId:string};function $$render() {

  
  
  
  
  
  let {roomId}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/=$props();
  let name=$state(''),invite=$state(''),error=$state(''),busy=$state(false),ended=$state(false),credential=$state<GameRoomCredential|null>(null);
  onMount(()=>{invite=consumeGameInvite(roomId)??'';if(!invite)error='This invitation is missing. Open a fresh game invitation from the host.';});
  async function join(){
    busy=true;error='';
    try{const r=await fetch(new URL('/api/v1/game-room-join',location.origin),{method:'POST',credentials:'omit',referrerPolicy:'no-referrer',headers:{'Content-Type':'application/json'},body:JSON.stringify({room_id:roomId,invite,name:name.trim()})});if(!r.ok){const problem=await r.json().catch(()=>({}));throw new Error(problem.message??'Couldn’t join this match. Ask the host for a fresh invitation.');}credential=await r.json() as GameRoomCredential;invite='';}
    catch(e){error=e instanceof Error?e.message:'Couldn’t join the game.';}finally{busy=false;}
  }
;
async () => {
 { svelteHTML.createElement("div", { "class":`game-guest`,});
 if(ended){ { svelteHTML.createElement("div", { "class":`game-join`,}); { svelteHTML.createElement("h1", {});    } { svelteHTML.createElement("p", {});     } }
 } else if (credential){  { const $$_hctaMemaG1C = __sveltets_2_ensureComponent(GameMatch); new $$_hctaMemaG1C({ target: __sveltets_2_any(), props: {    credential,"origin":location.origin,"onexit":()=>{ended=true;credential=null;},}});}
 }else{ { svelteHTML.createElement("form", {   "class":`game-join`,"onsubmit":e=>{e.preventDefault();void join();},}); { svelteHTML.createElement("h1", {});   } { svelteHTML.createElement("p", {});        } { svelteHTML.createElement("p", {});           { svelteHTML.createElement("strong", {});location.host; }  } { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "dir":`auto`,"bind:value":name,"maxlength":40,"autocomplete":`nickname`,"required":true,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/} }if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({text: error, assertive: true})));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`join-error`,});error; }}} { svelteHTML.createElement("button", {   "class":`btn primary`,"disabled":busy||!invite||!name.trim(),});busy?'Joining…':'Join game'; } }}
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GameGuest__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GameGuest__SvelteComponent_ = ReturnType<typeof GameGuest__SvelteComponent_>;
/*Ωignore_endΩ*/export default GameGuest__SvelteComponent_;