import { trackFor } from './maps.ts';
import { clamp, distance, emit, random } from './geometry.ts';
import type { GameInput, GamePlayer, GameState, TrackMap, Vec3 } from './types.ts';
import {sampleRoadHeight,sampleTrack} from './track-course.ts';
export function nearestTrackPoint(point:Vec3,track:TrackMap):{point:Vec3;distance:number;segment:number} {
 return sampleTrack(track,point.x,point.z);
}
function reset(s:GameState,p:GamePlayer,t:TrackMap):void {
 // Reset to the last earned checkpoint; no nearest-segment shortcut can award progress.
 const previous=(p.checkpoint+t.route.length-1)%t.route.length;const a=t.route[previous],b=t.route[p.checkpoint];
 Object.assign(p,a);p.yaw=Math.atan2(b.x-a.x,b.z-a.z);p.speed=0;p.steering=0;p.boost=0;p.driftCharge=0;p.drifting=false;p.offTrack=false;p.offTrackTime=0;p.resetCooldown=2;p.velocityY=0;p.grounded=true;p.airTime=0;p.trick=false;p.launchCooldown=2;
}
function itemHit(s:GameState,owner:number,target:number):void {
 const p=s.players[target];if(p.shield>0){p.shield=Math.max(0,p.shield-35);emit(s,'shield',target);}else{p.speed*=.55;p.boost=0;p.damageTime=.6;emit(s,'hit',owner,{target,x:p.x,y:p.y,z:p.z});}
}
export function stepKart(s:GameState,inputs:GameInput[],dt:number):void {
 const track=trackFor(s.config.map);
 s.pickupTimers=s.pickupTimers.map(t=>Math.max(0,t-dt));
 for(const p of s.players) {
 const i=inputs[p.id];p.itemCooldown=Math.max(0,p.itemCooldown-dt);p.resetCooldown=Math.max(0,p.resetCooldown-dt);p.boost=Math.max(0,p.boost-dt);p.launchCooldown=Math.max(0,p.launchCooldown-dt);p.damageTime=Math.max(0,p.damageTime-dt);p.shield=Math.max(0,p.shield-dt*3);
 if(i.reset&&p.resetCooldown===0)reset(s,p,track);
 const trackDistance=nearestTrackPoint(p,track).distance;p.offTrack=trackDistance>track.width/2;
 // Nearby grass remains drivable. Only a prolonged, distant excursion needs rescue.
 p.offTrackTime=p.offTrack?p.offTrackTime+dt:0;if(p.offTrackTime>4&&trackDistance>track.width*3)reset(s,p,track);
 const drifting=i.drift&&Math.abs(i.moveX)>0.12&&p.speed>7&&!p.offTrack&&p.grounded;
 if(drifting)p.driftCharge=Math.min(1.5,p.driftCharge+dt);
 if(p.drifting&&!drifting){if(p.driftCharge>0.35){p.boost=Math.max(p.boost,p.driftCharge*0.8);emit(s,'boost',p.id);}p.driftCharge=0;}
 p.drifting=drifting;
 // Shift releases an earned drift charge early, without creating free boosts.
 if(i.sprint&&p.driftCharge>0.35){p.boost=p.driftCharge*0.8;p.driftCharge=0;emit(s,'boost',p.id);}
 if(i.item&&p.item&&p.itemCooldown===0) {
 if(p.item==='boost'){p.boost=Math.max(p.boost,1.8);emit(s,'boost',p.id);}
 else if(p.item==='shield'){p.shield=75;emit(s,'shield',p.id);}
 else if(p.item==='seeker'){if(s.projectiles.length<8)s.projectiles.push({id:++s.eventSequence,owner:p.id,target:1-p.id,x:p.x,y:p.y+.7,z:p.z,life:5});}
 else {const enemy=s.players[1-p.id];if(distance(p,enemy)<18)itemHit(s,p.id,enemy.id);}
 p.item=null;p.itemCooldown=1;
 }
 const maxSpeed=p.offTrack?8:p.boost>0?33:p.underwater?21:23;
 // A changed speed limit is a drag target, never an instantaneous velocity clamp.
 // Opposite throttle brakes before reversing; neutral rolls to zero without crossing it.
 const targetSpeed=i.moveZ>0?i.moveZ*maxSpeed:i.moveZ*6;
 const braking=p.speed*i.moveZ<0;
 const slowing=Math.abs(p.speed)>Math.abs(targetSpeed);
 const rate=braking?24:slowing?(p.offTrack&&Math.abs(p.speed)>8?12:i.moveZ===0?5:8):i.moveZ<0?9:16*(1-.5*clamp(p.speed/maxSpeed,0,1));
 p.speed+=clamp(targetSpeed-p.speed,-rate*dt,rate*dt);
 // Persist steering for smooth onset/reversal, with less yaw authority at high speed.
 p.steering+=clamp(i.moveX-p.steering,-6*dt,6*dt);
 const grip=drifting?1.28:1;const steering=p.steering*2.15*clamp(Math.abs(p.speed)/8,0,1)/(1+Math.abs(p.speed)/40)*grip;
 p.yaw-=steering*dt*(p.speed>=0?1:-1);p.yaw=Math.atan2(Math.sin(p.yaw),Math.cos(p.yaw));
 p.x+=Math.sin(p.yaw)*p.speed*dt;p.z+=Math.cos(p.yaw)*p.speed*dt;p.moving=Math.abs(p.speed)>0.1;
 const surface=sampleTrack(track,p.x,p.z);
 const roadHeight=sampleRoadHeight(track,p.x,p.z),shoulderHeight=roadHeight===null?sampleRoadHeight(track,p.x,p.z,track.width/2+5):null;
 // Neon has no grass beneath the elevated shoulder. Other courses keep forgiving off-road driving.
 const floor=roadHeight??(shoulderHeight===null?(track.id==='neon'?null:surface.point.y):shoulderHeight-.1);
 if(p.grounded&&floor===null){p.grounded=false;p.velocityY=0;p.airTime=0;p.trick=false;}
 if(p.grounded){
  p.y=floor!;
  const ramp=track.ramps.find(r=>distance(p,track.route[r.index])<2.8);
  if(ramp&&p.speed>9&&!p.offTrack&&p.launchCooldown===0){p.grounded=false;p.velocityY=ramp.launch;p.airTime=0;p.trick=false;p.launchCooldown=2.5;emit(s,'launch',p.id);}
 }else{
  p.airTime+=dt;p.velocityY-=16*dt;p.y+=p.velocityY*dt;
  if(i.jump&&!p.trick&&p.airTime>.12){p.trick=true;emit(s,'trick',p.id);}
  if(floor!==null&&p.y<=floor&&p.velocityY<=0){p.y=floor;p.velocityY=0;p.grounded=true;emit(s,'land',p.id);if(p.trick){p.boost=Math.max(p.boost,1.25);emit(s,'boost',p.id);}p.airTime=0;p.trick=false;}
  else if(floor===null&&p.y<surface.point.y-6)reset(s,p,track);
 }
 p.underwater=track.waterLevel!==null&&p.y+.5<track.waterLevel;
 const checkpoint=track.route[p.checkpoint];const previous=track.route[(p.checkpoint+track.route.length-1)%track.route.length];
 const forward=Math.sin(p.yaw)*(checkpoint.x-previous.x)+Math.cos(p.yaw)*(checkpoint.z-previous.z);
 // Must approach the next checkpoint in order and in the racing direction.
 if(distance(p,checkpoint)<track.width*0.65&&p.speed>0&&forward>0) {
 if(p.checkpoint===0){p.lap++;emit(s,'lap',p.id);if(p.lap>=3){p.finishTime=s.elapsed;s.phase='finished';s.winner=p.id;emit(s,'finish',p.id);return;}}
 p.checkpoint=(p.checkpoint+1)%track.route.length;
 }
 if(!p.item)track.pickups.forEach((node,index)=>{if(s.pickupTimers[index]===0&&distance(p,track.route[node])<track.width*0.6&&Math.abs(p.y-track.route[node].y)<2){const items=['boost','pulse','shield','seeker'] as const;p.item=items[Math.floor(random(s)*items.length)];s.pickupTimers[index]=5;emit(s,'pickup',p.id);}});
 }
 const [a,b]=s.players;const d=distance(a,b);
 if(d<1.4&&d>0.001&&Math.abs(a.y-b.y)<1){const push=(1.4-d)/2,dx=(a.x-b.x)/d,dz=(a.z-b.z)/d;a.x+=dx*push;a.z+=dz*push;b.x-=dx*push;b.z-=dz*push;const mean=(a.speed+b.speed)/2;a.speed=a.speed*0.8+mean*0.2;b.speed=b.speed*0.8+mean*0.2;}
 s.projectiles=s.projectiles.filter(projectile=>{
  projectile.life-=dt;if(projectile.life<=0)return false;const target=s.players[projectile.target];
  const dx=target.x-projectile.x,dy=target.y+.7-projectile.y,dz=target.z-projectile.z,length=Math.hypot(dx,dy,dz);
  if(length<1.3+28*dt){itemHit(s,projectile.owner,projectile.target);return false;}
  const step=28*dt/length;projectile.x+=dx*step;projectile.y+=dy*step;projectile.z+=dz*step;return true;
 });
 s.remaining=0;
}
