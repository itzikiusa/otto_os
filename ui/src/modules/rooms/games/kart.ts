import { trackFor } from './maps.ts';
import { clamp, distance, emit, random } from './geometry.ts';
import type { GameInput, GamePlayer, GameState, TrackMap, Vec3 } from './types.ts';
export function nearestTrackPoint(point:Vec3,track:TrackMap):{point:Vec3;distance:number;segment:number} {
 let result={point:track.route[0],distance:Infinity,segment:0};
 for(let n=0;n<track.route.length;n++) {const a=track.route[n],b=track.route[(n+1)%track.route.length];const dx=b.x-a.x,dz=b.z-a.z;const t=clamp(((point.x-a.x)*dx+(point.z-a.z)*dz)/(dx*dx+dz*dz),0,1);const q={x:a.x+dx*t,y:0,z:a.z+dz*t};const d=distance(q,point);if(d<result.distance)result={point:q,distance:d,segment:n};}
 return result;
}
function reset(s:GameState,p:GamePlayer,t:TrackMap):void {
 // Reset to the last earned checkpoint; no nearest-segment shortcut can award progress.
 const previous=(p.checkpoint+t.route.length-1)%t.route.length;const a=t.route[previous],b=t.route[p.checkpoint];
 Object.assign(p,a);p.yaw=Math.atan2(b.x-a.x,b.z-a.z);p.speed=0;p.boost=0;p.driftCharge=0;p.drifting=false;p.offTrackTime=0;p.resetCooldown=2;
}
export function stepKart(s:GameState,inputs:GameInput[],dt:number):void {
 const track=trackFor(s.config.map);
 s.pickupTimers=s.pickupTimers.map(t=>Math.max(0,t-dt));
 for(const p of s.players) {
 const i=inputs[p.id];p.itemCooldown=Math.max(0,p.itemCooldown-dt);p.resetCooldown=Math.max(0,p.resetCooldown-dt);p.boost=Math.max(0,p.boost-dt);
 if(i.reset&&p.resetCooldown===0)reset(s,p,track);
 p.offTrack=nearestTrackPoint(p,track).distance>track.width/2;
 p.offTrackTime=p.offTrack?p.offTrackTime+dt:0;if(p.offTrackTime>4)reset(s,p,track);
 const drifting=i.drift&&Math.abs(i.moveX)>0.12&&p.speed>7&&!p.offTrack;
 if(drifting)p.driftCharge=Math.min(1.5,p.driftCharge+dt);
 if(p.drifting&&!drifting){if(p.driftCharge>0.35){p.boost=Math.max(p.boost,p.driftCharge*0.8);emit(s,'boost',p.id);}p.driftCharge=0;}
 p.drifting=drifting;
 // Shift releases an earned drift charge early, without creating free boosts.
 if(i.sprint&&p.driftCharge>0.35){p.boost=p.driftCharge*0.8;p.driftCharge=0;emit(s,'boost',p.id);}
 if(i.item&&p.item&&p.itemCooldown===0) {
 if(p.item==='boost'){p.boost=Math.max(p.boost,1.8);emit(s,'boost',p.id);}
 else {const enemy=s.players[1-p.id];if(distance(p,enemy)<18){enemy.speed*=0.35;enemy.boost=0;emit(s,'hit',p.id,{target:enemy.id});}}
 p.item=null;p.itemCooldown=1;
 }
 const maxSpeed=p.offTrack?8:p.boost>0?33:23;
 // Coasting removes speed in either direction and stops at zero.
 const acceleration=i.moveZ>0?i.moveZ*18:i.moveZ*28;
 const nextSpeed=i.moveZ===0?p.speed-clamp(p.speed,-5*dt,5*dt):p.speed+acceleration*dt;
 p.speed=clamp(nextSpeed,-6,maxSpeed);
 const grip=drifting?1.35:1;const steering=i.moveX*1.9*clamp(Math.abs(p.speed)/9,0,1)*grip;
 p.yaw-=steering*dt*(p.speed>=0?1:-1);p.yaw=Math.atan2(Math.sin(p.yaw),Math.cos(p.yaw));
 p.x+=Math.sin(p.yaw)*p.speed*dt;p.z+=Math.cos(p.yaw)*p.speed*dt;p.moving=Math.abs(p.speed)>0.1;
 const checkpoint=track.route[p.checkpoint];const previous=track.route[(p.checkpoint+track.route.length-1)%track.route.length];
 const forward=Math.sin(p.yaw)*(checkpoint.x-previous.x)+Math.cos(p.yaw)*(checkpoint.z-previous.z);
 // Must approach the next checkpoint in order and in the racing direction.
 if(distance(p,checkpoint)<track.width*0.65&&p.speed>0&&forward>0) {
 if(p.checkpoint===0){p.lap++;emit(s,'lap',p.id);if(p.lap>=3){p.finishTime=s.elapsed;s.phase='finished';s.winner=p.id;emit(s,'finish',p.id);return;}}
 p.checkpoint=(p.checkpoint+1)%track.route.length;
 }
 if(!p.item)track.pickups.forEach((node,index)=>{if(s.pickupTimers[index]===0&&distance(p,track.route[node])<track.width*0.6){p.item=random(s)<0.65?'boost':'pulse';s.pickupTimers[index]=5;emit(s,'pickup',p.id);}});
 }
 const [a,b]=s.players;const d=distance(a,b);
 if(d<1.4&&d>0.001){const push=(1.4-d)/2,dx=(a.x-b.x)/d,dz=(a.z-b.z)/d;a.x+=dx*push;a.z+=dz*push;b.x-=dx*push;b.z-=dz*push;const mean=(a.speed+b.speed)/2;a.speed=a.speed*0.8+mean*0.2;b.speed=b.speed*0.8+mean*0.2;}
 s.remaining=0;
}
