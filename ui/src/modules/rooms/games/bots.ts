import { arenaFor, trackFor } from './maps.ts';
import { angleDelta, clamp, distance, lineOfSight, random } from './geometry.ts';
import { BODY_RADIUS } from './shooter.ts';
import type { GameInput, GameState, Vec3 } from './types.ts';
const neutral=():GameInput=>({moveX:0,moveZ:0,yaw:0,pitch:0,fire:false,reload:false,jump:false,sprint:false,drift:false,item:false,reset:false});
const TUNING={easy:{reaction:0.32,aim:2.1,error:0.10,throttle:0.7},normal:{reaction:0.16,aim:3.7,error:0.042,throttle:0.88},hard:{reaction:0.07,aim:5.2,error:0.015,throttle:1}};
// Combat tuning is deliberately independent of kart driving. Accuracy is a spatial
// aim offset (metres), so Easy can miss even when a player gets close.
const SHOOTER_TUNING={
 easy:{reaction:0.38,aim:0.9,error:1.15,acquire:2.2,burst:0.12,cycle:3.0,pace:0.55,strafe:0.12,range:18},
 normal:{reaction:0.2,aim:2.4,error:0.5,acquire:0.85,burst:0.45,cycle:1.8,pace:0.85,strafe:0.3,range:15},
 hard:{reaction:0.09,aim:4.5,error:0.12,acquire:0.25,burst:0.9,cycle:1.15,pace:1,strafe:0.45,range:15},
};
/** Visibility graph around expanded cover corners. Bots walk the same collision geometry as humans. */
function nextWaypoint(s:GameState,from:Vec3,to:Vec3):Vec3 {
 // Match the physical body clearance, including legal contact at its boundary.
 const clearance=BODY_RADIUS-1e-6;
 const arena=arenaFor(s.config.map);const ground=(p:Vec3)=>({...p,y:0.7});const visible=(a:Vec3,b:Vec3)=>lineOfSight(ground(a),ground(b),arena.cover,clearance);
 if(visible(from,to))return to;
 const nodes:Vec3[]=[from,to];for(const b of arena.cover)for(const x of [-1,1])for(const z of [-1,1]){const p={x:b.x+x*(b.width/2+0.8),y:0,z:b.z+z*(b.depth/2+0.8)};if(Math.abs(p.x)<arena.halfSize-0.6&&Math.abs(p.z)<arena.halfSize-0.6)nodes.push(p);}
 const costs=nodes.map(()=>Infinity),previous=nodes.map(()=>-1),visited=new Set<number>();costs[0]=0;
 for(let loop=0;loop<nodes.length;loop++) {
 let current=-1;for(let n=0;n<nodes.length;n++)if(!visited.has(n)&&(current===-1||costs[n]<costs[current]))current=n;
 if(current<0||!Number.isFinite(costs[current]))break;if(current===1)break;visited.add(current);
 for(let n=0;n<nodes.length;n++){if(visited.has(n)||n===current||!visible(nodes[current],nodes[n]))continue;const cost=costs[current]+distance(nodes[current],nodes[n]);if(cost<costs[n]){costs[n]=cost;previous[n]=current;}}
 }
 // An unreachable destination is never a valid direct movement fallback.
 if(previous[1]===-1)return from;let next=1;while(previous[next]>0)next=previous[next];return nodes[next];
}
function shooterBot(s:GameState,id:number,dt:number):GameInput {
 const p=s.players[id],enemy=s.players[1-id],t=SHOOTER_TUNING[s.config.difficulty];
 // botTarget is the time acquisition completes, zero when no live target is seen.
 // Keeping it in the snapshot preserves deterministic reconnects without wire changes.
 const burstInput=():GameInput=>{
  const engagedFor=s.elapsed-p.botTarget;
  return {...p.botInput,fire:p.botInput.fire&&p.botTarget>0&&engagedFor>=0&&engagedFor%t.cycle<t.burst};
 };
 if(p.hp<=0||enemy.hp<=0){p.botTarget=0;p.botThink=0;p.botInput=neutral();return p.botInput;}
 p.botThink-=dt;if(p.botThink>0)return burstInput();p.botThink=t.reaction;
 const desired=Math.atan2(enemy.x-p.x,enemy.z-p.z);const input=neutral();
 const tracked=p.yaw+clamp(angleDelta(desired,p.yaw),-t.aim*t.reaction,t.aim*t.reaction);
 const d=distance(p,enemy);
 input.yaw=tracked+Math.atan2((random(s)*2-1)*t.error,Math.max(0.5,d));
 input.pitch=Math.atan2(enemy.y-p.y,Math.max(0.1,d))+(random(s)-0.5)*0.025;
 const visible=lineOfSight({x:p.x,y:p.y+1.55,z:p.z},{x:enemy.x,y:enemy.y+1.1,z:enemy.z},arenaFor(s.config.map).cover);
 if(!visible)p.botTarget=0;else if(p.botTarget===0)p.botTarget=s.elapsed+t.acquire;
 // Gate on tracking before inaccuracy; inaccurate shots must actually miss,
 // rather than suppressing misses and firing only the accurate samples.
 input.fire=visible&&Math.abs(angleDelta(desired,tracked))<0.2;
 input.reload=p.ammo<5;
 let worldX=0,worldZ=0;
 if(!visible||d>t.range){const next=nextWaypoint(s,p,enemy);const length=Math.max(0.001,distance(p,next));worldX=(next.x-p.x)/length*t.pace;worldZ=(next.z-p.z)/length*t.pace;}
 else {
 if(random(s)<0.12)p.botStrafe*=-1;
 const forward=d<7?-0.25:0;worldX=Math.sin(desired)*forward+Math.cos(desired)*p.botStrafe*t.strafe;worldZ=Math.cos(desired)*forward-Math.sin(desired)*p.botStrafe*t.strafe;
 }
 input.moveX=-worldX*Math.cos(input.yaw)+worldZ*Math.sin(input.yaw);input.moveZ=worldX*Math.sin(input.yaw)+worldZ*Math.cos(input.yaw);
 input.sprint=!visible&&s.config.difficulty==='hard';input.jump=visible&&s.config.difficulty==='hard'&&random(s)<0.05;
 p.botInput=input;return burstInput();
}
function kartBot(s:GameState,id:number,dt:number):GameInput {
 const p=s.players[id],track=trackFor(s.config.map),t=TUNING[s.config.difficulty];p.botThink-=dt;
 // Driving needs tighter feedback than combat, but reaction still scales by difficulty.
 if(p.botThink>0)return p.botInput;p.botThink=t.reaction*0.35;
 const target=track.route[p.checkpoint];const desired=Math.atan2(target.x-p.x,target.z-p.z);const delta=angleDelta(desired,p.yaw);const input=neutral();
 input.moveX=clamp(-delta*2.0,-1,1);
 const desiredSpeed=Math.abs(delta)>0.7?10:Math.abs(delta)>0.35?15:23*t.throttle;
 input.moveZ=p.speed>desiredSpeed?0:1;
 input.drift=s.config.difficulty!=='easy'&&Math.abs(delta)>0.3&&Math.abs(delta)<1&&p.speed>10;
 input.item=p.item==='boost'?Math.abs(delta)<0.2:distance(p,s.players[1-id])<15;
 input.reset=p.offTrackTime>2.5;
 p.botInput=input;return input;
}
export function botInput(s:GameState,id:number,dt:number):GameInput {return s.config.kind==='shooter'?shooterBot(s,id,dt):kartBot(s,id,dt);}
