import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createGame, defaultInput, stepGame } from '../src/modules/rooms/games/simulation.ts';
import { ARENAS, TRACKS } from '../src/modules/rooms/games/maps.ts';
import type { GameState } from '../src/modules/rooms/games/types.ts';
function advance(s: GameState, seconds: number, inputs = [defaultInput(), defaultInput()] as const) {
  for (let i = 0; i < Math.ceil(seconds * 60); i++) stepGame(s, inputs, 1 / 60);
}
test('delta is bounded, countdown blocks actions, and rematch starts fresh', () => {
  const s = createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});
  const x = s.players[0].x;
  stepGame(s, [{...defaultInput(),moveX:1,fire:true},defaultInput()], 1000);
  assert.equal(s.players[0].x,x); assert.equal(s.players[0].ammo,24);
  assert.ok(s.countdown > 2.8);
  advance(s,3); s.players[0].score=6;
  const fresh=createGame(s.config); assert.equal(fresh.players[0].score,0); assert.equal(fresh.phase,'countdown');
});
test('cover blocks hits and movement; an exposed target can be eliminated and respawn', () => {
 const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false}); s.phase='playing';
 const [a,b]=s.players; Object.assign(a,{x:0,z:-8,yaw:0,invulnerable:0}); Object.assign(b,{x:0,z:8,invulnerable:0});
 advance(s,1,[{...defaultInput(),fire:true},defaultInput()]); assert.equal(b.hp,100); assert.ok(a.ammo<24);
 Object.assign(a,{x:15,z:-8,ammo:24}); Object.assign(b,{x:15,z:8});
 advance(s,1,[{...defaultInput(),fire:true},defaultInput()]); assert.equal(b.hp,0); assert.equal(a.score,1);
 advance(s,3); assert.equal(b.hp,100);
 Object.assign(a,{x:0,z:-5}); advance(s,1,[{...defaultInput(),moveZ:1},defaultInput()]); assert.ok(a.z <= -2.4);
});
test('seventh elimination ends match and timer resolves scores', () => {
 const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false}); s.phase='playing';
 Object.assign(s.players[0],{x:15,z:-8,score:6,invulnerable:0}); Object.assign(s.players[1],{x:15,z:8,hp:25,invulnerable:0});
 advance(s,0.5,[{...defaultInput(),fire:true},defaultInput()]); assert.equal(s.phase,'finished'); assert.equal(s.winner,0);
 const timed=createGame(s.config); timed.phase='playing'; timed.remaining=0.01; timed.players[1].score=2; advance(timed,0.1); assert.equal(timed.winner,1);
});
test('hit and kill effects use the victim position while retaining shooter attribution', () => {
 for(const shooter of [0,1])for(const health of [100,25]) {
  const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';
  const attacker=s.players[shooter],victim=s.players[1-shooter];
  Object.assign(attacker,{x:15,y:0,z:-8,invulnerable:0});Object.assign(victim,{x:15,y:0,z:8,hp:health,invulnerable:0});
  const inputs=[defaultInput(),defaultInput()];inputs[shooter].fire=true;
  stepGame(s,inputs,1/60);
  const effects=s.events.filter(e=>e.type==='hit'||e.type==='kill');
  assert.deepEqual(effects.map(e=>e.type),health===25?['hit','kill']:['hit']);
  for(const effect of effects) {
   assert.equal(effect.player,shooter);assert.equal(effect.target,victim.id);
   assert.deepEqual({x:effect.x,y:effect.y,z:effect.z},{x:victim.x,y:victim.y,z:victim.z});
  }
 }
});
test('kart cannot award laps by skipping ordered checkpoints or resetting', () => {
 const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false}); s.phase='playing';
 const p=s.players[0]; const t=TRACKS.coast; Object.assign(p,t.route[0]); advance(s,0.1); assert.equal(p.lap,0); assert.equal(p.checkpoint,1);
 Object.assign(p,t.route[4]); advance(s,0.1); assert.equal(p.checkpoint,1);
 advance(s,0.1,[{...defaultInput(),reset:true},defaultInput()]); assert.equal(p.lap,0); assert.equal(p.checkpoint,1);
});
test('bots race three valid laps on all tracks and difficulties', () => {
 for (const map of Object.keys(TRACKS)) for (const difficulty of ['easy','normal','hard'] as const) {
  const s=createGame({kind:'kart',map,difficulty,vsComputer:true}); advance(s,240);
  assert.equal(s.phase,'finished',`${map}/${difficulty}: checkpoint ${s.players[1].checkpoint}, laps ${s.players[1].lap}`);
  assert.equal(s.winner,1); assert.equal(s.players[1].lap,3);
 }
});
test('shooter bots navigate cover and land eliminations in every arena', () => {
 for (const map of Object.keys(ARENAS)) {
 const s=createGame({kind:'shooter',map,difficulty:'normal',vsComputer:true}); advance(s,90);
 assert.ok(s.players[1].score>0,`${map} bot never eliminated stationary opponent`);
 }
});
test('seeded games and JSON snapshot continuations are deterministic', () => {
 const a=createGame({kind:'kart',map:'forest',difficulty:'hard',vsComputer:true,seed:123}); advance(a,15);
 const b=JSON.parse(JSON.stringify(a)) as GameState; advance(a,10); advance(b,10); assert.deepEqual(a,b);
});
test('reload blocks firing and refills a magazine; invalid inputs cannot poison state', () => {
 const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';s.players[0].ammo=3;
 advance(s,0.5,[{...defaultInput(),reload:true,fire:true},defaultInput()]);assert.equal(s.players[0].ammo,3);assert.ok(s.players[0].reloadTime>0);
 advance(s,1);assert.equal(s.players[0].ammo,24);
 const before=JSON.stringify(s);stepGame(s,[defaultInput(),defaultInput()],NaN);assert.equal(JSON.stringify(s),before);
 advance(s,1,[{...defaultInput(),moveX:NaN,moveZ:Infinity,yaw:NaN,pitch:Infinity},defaultInput()]);assert.ok(Number.isFinite(s.players[0].x));assert.ok(Number.isFinite(s.players[0].yaw));
});
test('jump leaves the ground and lands; sprint is faster but not while firing', () => {
 const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';Object.assign(s.players[0],{x:20,z:-15});
 stepGame(s,[{...defaultInput(),jump:true,moveZ:1,sprint:true},defaultInput()],1/60);assert.ok(s.players[0].y>0);assert.equal(s.players[0].speed,9);
 advance(s,1,[{...defaultInput(),moveZ:1,sprint:true,fire:true},defaultInput()]);assert.equal(s.players[0].y,0);assert.equal(s.players[0].speed,6);
});
test('drifting earns boost; item pickups recharge and off-track recovery preserves progress', () => {
 const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false});s.phase='playing';const p=s.players[0];
 p.speed=16;advance(s,0.45,[{...defaultInput(),moveZ:1,moveX:0.2,drift:true},defaultInput()]);assert.ok(p.driftCharge>0.35);
 stepGame(s,[defaultInput(),defaultInput()],1/60);assert.ok(p.boost>0);
 Object.assign(p,TRACKS.coast.route[2],{speed:0});advance(s,0.05);assert.ok(p.item);assert.ok(s.pickupTimers[0]>0);
 p.item='boost';advance(s,0.05,[{...defaultInput(),item:true},defaultInput()]);assert.equal(p.item,null);assert.ok(p.boost>1);
 const checkpoint=p.checkpoint,lap=p.lap;Object.assign(p,{x:900,z:900,speed:0});advance(s,4.2);assert.equal(p.checkpoint,checkpoint);assert.equal(p.lap,lap);assert.ok(Math.abs(p.x)<100);
});
test('all shooter difficulty levels can acquire and hit a target without wall penetration', () => {
 for(const map of Object.keys(ARENAS))for(const difficulty of ['easy','normal','hard'] as const){const s=createGame({kind:'shooter',map,difficulty,vsComputer:true,seed:2121});advance(s,100);assert.ok(s.players[1].score>0,`${map}/${difficulty}`);}
});
test('right controls move screen-right when viewing +Z in Three.js', () => {
 const shooter=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});shooter.phase='playing';Object.assign(shooter.players[0],{x:20,z:0,yaw:0});
 stepGame(shooter,[{...defaultInput(),moveX:1},defaultInput()],1/60);assert.ok(shooter.players[0].x<20);
 const kart=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false});kart.phase='playing';kart.players[0].yaw=0;kart.players[0].speed=10;
 stepGame(kart,[{...defaultInput(),moveX:1},defaultInput()],1/60);assert.ok(kart.players[0].yaw<0);
});
for (const speed of [0, 4, -4]) {
 test(`neutral kart throttle coasts from ${speed} to rest without reversing`, () => {
  const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false});s.phase='playing';
  const p=s.players[0];p.speed=speed;const start={x:p.x,z:p.z};
  for(let tick=0;tick<180;tick++) {
   stepGame(s,[defaultInput(),defaultInput()],1/60);
   assert.ok(speed>=0?p.speed>=0:p.speed<=0,'friction must not cross zero');
   assert.ok(Math.abs(p.speed)<=Math.abs(speed),'friction must not accelerate the kart');
  }
  assert.equal(p.speed,0);
  if(speed===0){assert.equal(p.x,start.x);assert.equal(p.z,start.z);}
 });
}
test('shooter bot routes around cover to a legal opponent hugging the opposite wall', () => {
 for(const targetZ of [2.45,2.5,2.6]) {
  const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:true});s.phase='playing';
  Object.assign(s.players[0],{x:0,y:0,z:targetZ,invulnerable:0});Object.assign(s.players[1],{x:0,y:0,z:-8,invulnerable:0});
  let damaged=false;
  for(let tick=0;tick<25*60;tick++) {
   stepGame(s,[defaultInput(),defaultInput()],1/60);
   if(s.players[0].hp<100||s.players[1].score>0){damaged=true;break;}
  }
  assert.ok(damaged,`bot stalled against cover with target at z=${targetZ}`);
 }
});
