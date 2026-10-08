import { arenaFor } from './maps.ts';
import { angleDelta, clamp, distance, emit, segmentBox } from './geometry.ts';
import type { GameInput, GamePlayer, GameState, Vec3 } from './types.ts';
export { lineOfSight, segmentBox } from './geometry.ts';
export const BODY_RADIUS=0.45;
export const EYE_HEIGHT=1.55;
function canStand(s:GameState,p:GamePlayer,x:number,z:number):boolean {
 const arena=arenaFor(s.config.map);if(Math.abs(x)>arena.halfSize-BODY_RADIUS||Math.abs(z)>arena.halfSize-BODY_RADIUS)return false;
 return !arena.cover.some(b=>p.y<b.y+b.height&&p.y+1.8>b.y&&Math.abs(x-b.x)<b.width/2+BODY_RADIUS&&Math.abs(z-b.z)<b.depth/2+BODY_RADIUS);
}
function respawn(s:GameState,p:GamePlayer):void {
 const opponent=s.players[1-p.id];const points=arenaFor(s.config.map).spawns;
 const spawn=[...points].sort((a,b)=>distance(b,opponent)-distance(a,opponent))[0];
 Object.assign(p,spawn,{hp:100,ammo:24,respawnTime:0,invulnerable:1.2,reloadTime:0,velocityY:0,grounded:true,cooldown:0});emit(s,'respawn',p.id);
}
function fire(s:GameState,p:GamePlayer):void {
 p.ammo--;p.cooldown=0.16;const enemy=s.players[1-p.id];
 const from={x:p.x,y:p.y+EYE_HEIGHT,z:p.z};
 const direction={x:Math.sin(p.yaw)*Math.cos(p.pitch),y:Math.sin(p.pitch),z:Math.cos(p.yaw)*Math.cos(p.pitch)};
 const end:Vec3={x:from.x+direction.x*90,y:from.y+direction.y*90,z:from.z+direction.z*90};
 let nearest=1;for(const b of arenaFor(s.config.map).cover){const hit=segmentBox(from,end,b);if(hit!==null)nearest=Math.min(nearest,hit);}
 // Body ray intersection and cover use the exact same ray, independent of camera mode.
 const body={x:enemy.x,y:enemy.y,z:enemy.z,width:0.9,height:1.85,depth:0.9};const hit=enemy.hp>0?segmentBox(from,end,body):null;
 const strikes=hit!==null&&hit<nearest&&enemy.invulnerable<=0;
 if(strikes&&hit!==null)nearest=hit;
 emit(s,'shot',p.id,{end:{x:from.x+(end.x-from.x)*nearest,y:from.y+(end.y-from.y)*nearest,z:from.z+(end.z-from.z)*nearest}});
 if(!strikes)return;
 // Effects are centered on the victim; player still identifies the attacker.
 const impact={target:enemy.id,x:enemy.x,y:enemy.y,z:enemy.z};
 enemy.hp=Math.max(0,enemy.hp-25);emit(s,'hit',p.id,impact);
 if(enemy.hp===0){p.score++;enemy.respawnTime=2;enemy.moving=false;emit(s,'kill',p.id,impact);if(p.score>=7){s.phase='finished';s.winner=p.id;emit(s,'finish',p.id);}}
}
export function stepShooter(s:GameState,inputs:GameInput[],dt:number):void {
 for(const p of s.players) {
 const i=inputs[p.id];p.cooldown=Math.max(0,p.cooldown-dt);p.invulnerable=Math.max(0,p.invulnerable-dt);
 if(p.hp<=0){p.respawnTime-=dt;if(p.respawnTime<=0)respawn(s,p);continue;}
 p.yaw+=angleDelta(i.yaw,p.yaw);p.pitch=clamp(i.pitch,-1.35,1.35);
 if(p.reloadTime>0){p.reloadTime=Math.max(0,p.reloadTime-dt);if(p.reloadTime===0)p.ammo=24;}
 if((i.reload||p.ammo===0)&&p.ammo<24&&p.reloadTime===0)p.reloadTime=1.4;
 if(i.jump&&p.grounded){p.velocityY=7;p.grounded=false;}
 p.velocityY-=20*dt;p.y=Math.max(0,p.y+p.velocityY*dt);if(p.y===0){p.velocityY=0;p.grounded=true;}
 const len=Math.max(1,Math.hypot(i.moveX,i.moveZ));const speed=i.sprint&&!i.fire?9:6;
 const dx=(-Math.cos(p.yaw)*i.moveX+Math.sin(p.yaw)*i.moveZ)/len*speed*dt;
 const dz=(Math.sin(p.yaw)*i.moveX+Math.cos(p.yaw)*i.moveZ)/len*speed*dt;
 const oldX=p.x,oldZ=p.z;if(canStand(s,p,p.x+dx,p.z))p.x+=dx;if(canStand(s,p,p.x,p.z+dz))p.z+=dz;
 p.moving=Math.hypot(p.x-oldX,p.z-oldZ)>0.001;p.speed=p.moving?speed:0;
 }
 // Move both before resolving fire so player ordering does not create a movement advantage.
 for(const p of s.players){if(s.phase==='finished')break;if(p.hp>0&&inputs[p.id].fire&&p.ammo>0&&p.reloadTime===0&&p.cooldown===0)fire(s,p);}
 s.remaining=Math.max(0,s.remaining-dt);
 if(s.remaining===0){s.phase='finished';s.winner=s.players[0].score===s.players[1].score?null:s.players[0].score>s.players[1].score?0:1;emit(s,'finish',s.winner??0);}
}
