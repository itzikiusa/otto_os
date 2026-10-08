import { WEAPONS, WEAPON_ORDER } from './weapons.ts';
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
 Object.assign(p,spawn,{hp:100,ammo:WEAPONS[p.weapon].capacity,shield:0,damageTime:0,dashCooldown:0,respawnTime:0,invulnerable:1.2,reloadTime:0,velocityY:0,grounded:true,cooldown:0});emit(s,'respawn',p.id);
}
function fire(s:GameState,p:GamePlayer):void {
 const weapon=WEAPONS[p.weapon];p.ammo--;p.cooldown=weapon.interval;const enemy=s.players[1-p.id];
 const from={x:p.x,y:p.y+EYE_HEIGHT,z:p.z};let damage=0;
 for(let pellet=0;pellet<weapon.pellets;pellet++){
  const angle=pellet*2.399963,spread=pellet===0?0:weapon.spread*Math.sqrt(pellet/(weapon.pellets-1));
  const yaw=p.yaw+Math.cos(angle)*spread,pitch=p.pitch+Math.sin(angle)*spread;
  const direction={x:Math.sin(yaw)*Math.cos(pitch),y:Math.sin(pitch),z:Math.cos(yaw)*Math.cos(pitch)};
  const end:Vec3={x:from.x+direction.x*weapon.range,y:from.y+direction.y*weapon.range,z:from.z+direction.z*weapon.range};
  let nearest=1;for(const b of arenaFor(s.config.map).cover){const hit=segmentBox(from,end,b);if(hit!==null)nearest=Math.min(nearest,hit);}
  const body={x:enemy.x,y:enemy.y,z:enemy.z,width:.9,height:1.85,depth:.9};const hit=enemy.hp>0?segmentBox(from,end,body):null;
  if(hit!==null&&hit<nearest){nearest=hit;if(enemy.invulnerable<=0)damage+=weapon.damage;}
  emit(s,'shot',p.id,{end:{x:from.x+(end.x-from.x)*nearest,y:from.y+(end.y-from.y)*nearest,z:from.z+(end.z-from.z)*nearest}});
 }
 if(damage===0)return;
 const absorbed=Math.min(enemy.shield,damage);enemy.shield-=absorbed;enemy.hp=Math.max(0,enemy.hp-damage+absorbed);enemy.damageTime=.3;
 const impact={target:enemy.id,x:enemy.x,y:enemy.y,z:enemy.z};emit(s,'hit',p.id,impact);
 if(absorbed>0)emit(s,'shield',enemy.id,impact);
 if(enemy.hp===0){p.score++;enemy.respawnTime=2;enemy.moving=false;emit(s,'kill',p.id,impact);if(p.score>=7){s.phase='finished';s.winner=p.id;emit(s,'finish',p.id);}}
}
export function stepShooter(s:GameState,inputs:GameInput[],dt:number):void {
 s.pickupTimers=s.pickupTimers.map(t=>Math.max(0,t-dt));
 for(const p of s.players) {
 const i=inputs[p.id];p.damageTime=Math.max(0,p.damageTime-dt);p.dashCooldown=Math.max(0,p.dashCooldown-dt);
 if(i.weapon&&WEAPON_ORDER[i.weapon-1]&&p.weapon!==WEAPON_ORDER[i.weapon-1]){
  p.weapon=WEAPON_ORDER[i.weapon-1];p.ammo=Math.min(p.ammo,WEAPONS[p.weapon].capacity);
  // Reload progress belongs to its weapon; switching cannot borrow a faster magazine refill.
  if(p.reloadTime>0)p.reloadTime=WEAPONS[p.weapon].reload;
 }
 const weapon=WEAPONS[p.weapon];p.cooldown=Math.max(0,p.cooldown-dt);p.invulnerable=Math.max(0,p.invulnerable-dt);
 if(p.hp<=0){p.respawnTime-=dt;if(p.respawnTime<=0)respawn(s,p);continue;}
 p.yaw+=angleDelta(i.yaw,p.yaw);p.pitch=clamp(i.pitch,-1.35,1.35);
 if(p.reloadTime>0){p.reloadTime=Math.max(0,p.reloadTime-dt);if(p.reloadTime===0)p.ammo=weapon.capacity;}
 if((i.reload||p.ammo===0)&&p.ammo<weapon.capacity&&p.reloadTime===0)p.reloadTime=weapon.reload;
 if(i.jump&&p.grounded){p.velocityY=7;p.grounded=false;}
 p.velocityY-=20*dt;p.y=Math.max(0,p.y+p.velocityY*dt);if(p.y===0){p.velocityY=0;p.grounded=true;}
 const len=Math.max(1,Math.hypot(i.moveX,i.moveZ));const speed=i.sprint&&!i.fire?9:6;
 const dx=(-Math.cos(p.yaw)*i.moveX+Math.sin(p.yaw)*i.moveZ)/len*speed*dt;
 const dz=(Math.sin(p.yaw)*i.moveX+Math.cos(p.yaw)*i.moveZ)/len*speed*dt;
 const oldX=p.x,oldZ=p.z;
 if(i.item&&p.dashCooldown===0){
  const forward=i.moveX===0&&i.moveZ===0?1:i.moveZ,side=i.moveX,length=Math.max(1,Math.hypot(side,forward));
  const dashX=(-Math.cos(p.yaw)*side+Math.sin(p.yaw)*forward)/length*.2,dashZ=(Math.sin(p.yaw)*side+Math.cos(p.yaw)*forward)/length*.2;
  for(let step=0;step<25;step++){if(canStand(s,p,p.x+dashX,p.z))p.x+=dashX;if(canStand(s,p,p.x,p.z+dashZ))p.z+=dashZ;}
  p.dashCooldown=4;emit(s,'dash',p.id);
 }
 if(canStand(s,p,p.x+dx,p.z))p.x+=dx;if(canStand(s,p,p.x,p.z+dz))p.z+=dz;
 p.moving=Math.hypot(p.x-oldX,p.z-oldZ)>0.001;p.speed=p.moving?speed:0;
 arenaFor(s.config.map).pickups.forEach((pickup,index)=>{
  if(s.pickupTimers[index]>0||distance(p,pickup)>1.5||Math.abs(p.y-pickup.y)>1.5)return;
  if(pickup.kind==='health'){if(p.hp>=100)return;p.hp=Math.min(100,p.hp+40);}else{if(p.shield>=75)return;p.shield=Math.min(75,p.shield+50);}
  s.pickupTimers[index]=12;emit(s,'pickup',p.id);
 });
 }
 // Move both before resolving fire so player ordering does not create a movement advantage.
 for(const p of s.players){if(s.phase==='finished')break;if(p.hp>0&&inputs[p.id].fire&&p.ammo>0&&p.reloadTime===0&&p.cooldown===0)fire(s,p);}
 s.remaining=Math.max(0,s.remaining-dt);
 if(s.remaining===0){s.phase='finished';s.winner=s.players[0].score===s.players[1].score?null:s.players[0].score>s.players[1].score?0:1;emit(s,'finish',s.winner??0);}
}
