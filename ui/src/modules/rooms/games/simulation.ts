import { arenaFor, trackFor } from './maps.ts';
import type { GameConfig, GameInput, GamePlayer, GameState } from './types.ts';
import { stepShooter } from './shooter.ts';
import { stepKart } from './kart.ts';
import { botInput } from './bots.ts';
export const FIXED_STEP = 1/60;
const finished = (state:GameState):boolean => state.phase==='finished';
export function defaultInput():GameInput { return {moveX:0,moveZ:0,yaw:0,pitch:0,fire:false,reload:false,jump:false,sprint:false,drift:false,item:false,reset:false}; }
function player(id:number):GamePlayer { return {id,x:0,y:0,z:0,yaw:0,pitch:0,hp:100,ammo:24,score:0,speed:0,steering:0,velocityY:0,grounded:true,moving:false,cooldown:0,reloadTime:0,respawnTime:0,invulnerable:1,lap:0,checkpoint:1,boost:0,driftCharge:0,drifting:false,item:null,itemCooldown:0,offTrack:false,offTrackTime:0,finishTime:null,resetCooldown:0,botThink:0,botInput:defaultInput(),botTarget:0,botStrafe:1}; }
export function createGame(config:GameConfig):GameState {
 const normalized={...config,map:config.kind==='shooter'?arenaFor(config.map).id:trackFor(config.map).id};
 const s:GameState={config:normalized,phase:'countdown',countdown:3,elapsed:0,remaining:180,winner:null,players:[player(0),player(1)],events:[],eventSequence:0,rng:(config.seed??72813)>>>0,accumulator:0,tick:0,pickupTimers:[]};
 if(config.kind==='shooter') s.players.forEach((p,i)=>{Object.assign(p,arenaFor(normalized.map).spawns[i]);p.yaw=i===0?Math.PI/4:-3*Math.PI/4;});
 else {const t=trackFor(normalized.map);const start=t.route[0], next=t.route[1];const yaw=Math.atan2(next.x-start.x,next.z-start.z);s.players.forEach((p,i)=>{p.x=start.x+Math.cos(yaw)*(i===0?-1.8:1.8);p.z=start.z-Math.sin(yaw)*(i===0?-1.8:1.8);p.yaw=yaw;});s.pickupTimers=t.pickups.map(()=>0);}
 return s;
}
const finite=(v:number,limit:number)=>Number.isFinite(v)?Math.max(-limit,Math.min(limit,v)):0;
/** Clamp peer input before it touches authoritative simulation. */
export function sanitizeInput(input:GameInput):GameInput { return {moveX:finite(input.moveX,1),moveZ:finite(input.moveZ,1),yaw:finite(input.yaw,Math.PI*1000),pitch:finite(input.pitch,1.35),fire:input.fire===true,reload:input.reload===true,jump:input.jump===true,sprint:input.sprint===true,drift:input.drift===true,item:input.item===true,reset:input.reset===true}; }
/** Fixed 60 Hz, at most six ticks per call. No wall-clock catch-up after tab suspension. */
export function stepGame(state:GameState,inputs:readonly GameInput[],dt:number):GameState {
 state.events=[];
 if(state.phase==='finished'||!Number.isFinite(dt)||dt<=0)return state;
 state.accumulator+=Math.min(dt,0.1);
 const safe=[sanitizeInput(inputs[0]??defaultInput()),sanitizeInput(inputs[1]??defaultInput())];
 for(let steps=0;state.accumulator+1e-9>=FIXED_STEP&&steps<6;steps++) {
  state.accumulator=Math.max(0,state.accumulator-FIXED_STEP);state.tick++;
  if(state.phase==='countdown'){state.countdown=Math.max(0,state.countdown-FIXED_STEP);if(state.countdown<1e-8){state.countdown=0;state.phase='playing';}continue;}
  state.elapsed+=FIXED_STEP;
  if(state.config.vsComputer)safe[1]=botInput(state,1,FIXED_STEP);
  if(state.config.kind==='shooter')stepShooter(state,safe,FIXED_STEP);else stepKart(state,safe,FIXED_STEP);
  if(finished(state))break;
 }
 return state;
}
