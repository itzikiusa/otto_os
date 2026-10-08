import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createGame,defaultInput,stepGame} from '../src/modules/rooms/games/simulation.ts';
import {TRACKS,ARENAS} from '../src/modules/rooms/games/maps.ts';
import type {GameState,GameInput} from '../src/modules/rooms/games/types.ts';
import {sampleTrack} from '../src/modules/rooms/games/track-course.ts';
import {decodeInput,decodeSnapshot} from '../src/modules/rooms/games/client.ts';
import {WEAPONS} from '../src/modules/rooms/games/weapons.ts';
function tick(s:GameState,input:Partial<GameInput>={}){stepGame(s,[{...defaultInput(),...input},defaultInput()],1/60);}
test('courses contain sampled curves, real elevation and a submerged coast road',()=>{
 for(const track of Object.values(TRACKS)){assert.ok(track.route.length>30);assert.ok(Math.max(...track.route.map(p=>p.y))-Math.min(...track.route.map(p=>p.y))>3);assert.ok(track.ramps.length>0);}
 assert.ok(TRACKS.coast.route.some(p=>p.y<-3));assert.equal(TRACKS.forest.waterLevel,null);
});
test('ramp launches, one airborne trick rewards landing, and road height is authoritative',()=>{
 const s=createGame({kind:'kart',map:'forest',difficulty:'normal',vsComputer:false});s.phase='playing';const p=s.players[0],track=TRACKS.forest,ramp=track.ramps[0],node=track.route[ramp.index],next=track.route[ramp.index+1];
 Object.assign(p,node,{speed:18,yaw:Math.atan2(next.x-node.x,next.z-node.z),checkpoint:ramp.index+1});Object.assign(s.players[1],track.route[20]);
 tick(s,{moveZ:1});assert.equal(p.grounded,false);assert.ok(p.velocityY>0);
 let tricks=0,landed=false;
 for(let n=0;n<240;n++){tick(s,{jump:true});tricks+=s.events.filter(e=>e.type==='trick').length;if(p.grounded){landed=true;break;}}
 assert.ok(landed);assert.equal(tricks,1);assert.ok(p.boost>0);assert.equal(p.velocityY,0);
});
test('coast immersion follows the road and clears again on dry land',()=>{
 const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false});s.phase='playing';const p=s.players[0],track=TRACKS.coast;
 Object.assign(p,track.route.find(p=>p.y<-4)!);tick(s);assert.ok(p.underwater);assert.ok(p.y<-3);
 Object.assign(p,track.route[0]);tick(s);assert.equal(p.underwater,false);
});
test('shield absorbs a seeker and unshielded seeker slows its target',()=>{
 for(const shield of [0,60]){const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false});s.phase='playing';const [a,b]=s.players;Object.assign(a,{item:'seeker',x:0,z:-44});Object.assign(b,{x:0,z:-39,speed:12,shield});tick(s,{item:true});assert.equal(a.item,null);assert.ok(s.projectiles.length>0);let hit=false;
 for(let n=0;n<180;n++){tick(s);if(s.events.some(e=>e.type==='hit'||e.type==='shield')){hit=true;break;}}
 assert.ok(hit);assert.equal(s.projectiles.length,0);if(shield)assert.ok(b.shield<shield&&b.speed>5);else assert.ok(b.speed<8);
 }
});
test('weapon choices have distinct magazines and covered shots stay blocked',()=>{
 for(const [weapon,capacity] of [[1,24],[2,6],[3,4]]){const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';const [a,b]=s.players;Object.assign(a,{x:0,z:-8,invulnerable:0});Object.assign(b,{x:0,z:8,invulnerable:0});tick(s,{weapon});assert.equal(a.ammo,capacity);tick(s,{fire:true});assert.equal(b.hp,100);assert.equal(a.ammo,capacity-1);}
});
test('health and shield pickups recharge and dash cannot tunnel through cover',()=>{
 const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';const p=s.players[0],arena=ARENAS.station;
 for(const kind of ['health','shield']){const index=arena.pickups.findIndex(p=>p.kind===kind);Object.assign(p,arena.pickups[index],{hp:40,shield:0});tick(s);assert.ok(kind==='health'?p.hp>40:p.shield>0);assert.ok(s.pickupTimers[index]>0);}
 Object.assign(p,{x:0,y:0,z:-5});tick(s,{item:true,moveZ:1});assert.ok(p.dashCooldown>0);assert.ok(p.z<=-2.45);
});
test('road samples interpolate authoritative height and include a normalized course position',()=>{
 for(const track of Object.values(TRACKS))for(let n=0;n<track.route.length;n++){
  const a=track.route[n],b=track.route[(n+1)%track.route.length],sample=sampleTrack(track,(a.x+b.x)/2,(a.z+b.z)/2);
  assert.ok(sample.distance<1e-8);assert.ok(Math.abs(sample.point.y-(a.y+b.y)/2)<1e-8);assert.ok(sample.progress>=0&&sample.progress<1);
 }
});
test('rifle, scatter and rail deliver their distinct damage and reload their own magazine',()=>{
 const damage:number[]=[];
 for(const weapon of [1,2,3]){
  const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';const [a,b]=s.players;
  Object.assign(a,{x:20,z:0,invulnerable:0});Object.assign(b,{x:20,z:3,invulnerable:0});tick(s,{weapon,fire:true});damage.push(100-b.hp);
  a.ammo=1;tick(s,{reload:true});assert.ok(a.reloadTime>0);const definition=WEAPONS[a.weapon];
  for(let n=0;n<Math.ceil((definition.reload+.1)*60);n++)tick(s);
  assert.equal(a.ammo,definition.capacity);
 }
 assert.equal(damage[0],25);assert.ok(damage[1]>damage[0]);assert.equal(damage[2],70);
});
test('switching weapons cannot refill spent ammunition and shields absorb damage before health',()=>{
 const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';const [a,b]=s.players;
 Object.assign(a,{x:20,z:0,invulnerable:0,ammo:2});Object.assign(b,{x:20,z:3,invulnerable:0,shield:30});
 tick(s,{weapon:2});tick(s,{weapon:1,fire:true});assert.equal(a.ammo,1);assert.equal(b.hp,100);assert.equal(b.shield,5);
});
test('switching during reload restarts the selected weapon reload instead of transferring a shorter timer',()=>{
 for(const ammo of [0,2]){
  const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';const p=s.players[0];p.ammo=ammo;
  tick(s,{reload:true});tick(s,{weapon:3});
  for(let n=0;n<85;n++)tick(s);
  assert.equal(p.ammo,ammo);assert.ok(p.reloadTime>0,'rail must not complete after the rifle reload duration');
  for(let n=0;n<55;n++)tick(s);
  assert.equal(p.ammo,4);assert.equal(p.reloadTime,0);
 }
});
test('leaving the Neon shoulder falls then recovers without awarding checkpoints',()=>{
 const s=createGame({kind:'kart',map:'neon',difficulty:'normal',vsComputer:false});s.phase='playing';const p=s.players[0];
 Object.assign(p,{x:33.72648555144039,y:7.330434934333084,z:-12.30107655537026,checkpoint:12});
 const initialY=p.y;tick(s);
 assert.equal(p.grounded,false);assert.ok(p.y<initialY);
 let recovered=false;
 for(let n=0;n<120;n++){tick(s);if(p.resetCooldown>0){recovered=true;break;}}
 assert.ok(recovered);assert.equal(p.grounded,true);assert.equal(p.checkpoint,12);assert.equal(p.lap,0);
 assert.ok(Math.hypot(p.x-TRACKS.neon.route[11].x,p.z-TRACKS.neon.route[11].z)<.01);
});
test('pickup cooldown prevents repeated consumption and eventually makes the pickup available',()=>{
 const s=createGame({kind:'shooter',map:'station',difficulty:'normal',vsComputer:false});s.phase='playing';const p=s.players[0],index=ARENAS.station.pickups.findIndex(p=>p.kind==='health');
 Object.assign(p,ARENAS.station.pickups[index],{hp:20});tick(s);assert.equal(p.hp,60);tick(s);assert.equal(p.hp,60);
 for(let n=0;n<13*60;n++)tick(s);assert.equal(p.hp,100);
});
test('snapshot validates new state, full unsigned RNG range and bounded projectiles',()=>{
 const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false,seed:0xffffffff});
 assert.deepEqual(decodeSnapshot(JSON.parse(JSON.stringify(s)),s.config),s);
 assert.equal(decodeSnapshot({...s,players:[{...s.players[0],weapon:'unknown'},s.players[1]]},s.config),null);
 assert.equal(decodeSnapshot({...s,players:[{...s.players[0],character:'unknown'},s.players[1]]},s.config),null);
 assert.equal(decodeSnapshot({...s,projectiles:Array.from({length:9},()=>({id:1,owner:0,target:1,x:0,y:0,z:0,life:4}))},s.config),null);
 assert.equal(decodeInput({...defaultInput(),weapon:99}),null);assert.equal(decodeInput({...defaultInput(),character:'unknown'}),null);
 s.projectiles=Array.from({length:8},(_,id)=>({id,owner:0,target:1,x:0,y:0,z:0,life:4}));
 assert.ok(Buffer.byteLength(JSON.stringify(s))<16*1024);
});
test('kart snapshots reject checkpoints that cannot index the active course',()=>{
 for(const map of Object.keys(TRACKS)){
  const s=createGame({kind:'kart',map,difficulty:'normal',vsComputer:false});
  for(const checkpoint of [-1,.5,TRACKS[map].route.length,999999]){
   assert.equal(decodeSnapshot({...s,players:[{...s.players[0],checkpoint},s.players[1]]},s.config),null,`${map}: checkpoint ${checkpoint}`);
  }
  assert.ok(decodeSnapshot({...s,players:[{...s.players[0],checkpoint:0},s.players[1]]},s.config));
  assert.ok(decodeSnapshot({...s,players:[{...s.players[0],checkpoint:TRACKS[map].route.length-1},s.players[1]]},s.config));
 }
});
test('character selection is authoritative and seekers expire instead of accumulating',()=>{
 const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false});s.phase='playing';
 tick(s,{character:'robot'});assert.equal(s.players[0].character,'robot');
 s.projectiles.push({id:1,owner:0,target:1,x:500,y:0,z:500,life:.02});tick(s);tick(s);assert.equal(s.projectiles.length,0);
});
test('a full bot race traverses jumps on every course and the underwater coast without recovery shortcuts',()=>{
 for(const map of Object.keys(TRACKS)){
  const s=createGame({kind:'kart',map,difficulty:'normal',vsComputer:true});let launches=0,tricks=0,wet=0,resets=0,previousCooldown=0;
  for(let n=0;n<120*60&&s.phase!=='finished';n++){
   tick(s);const p=s.players[1];
   launches+=s.events.filter(e=>e.type==='launch'&&e.player===1).length;
   tricks+=s.events.filter(e=>e.type==='trick'&&e.player===1).length;
   wet+=Number(p.underwater);if(p.resetCooldown>previousCooldown)resets++;previousCooldown=p.resetCooldown;
  }
  assert.equal(s.players[1].lap,3,map);assert.equal(resets,0,`${map}: race should not rely on recovery teleports`);
  assert.ok(launches>=3,`${map}: bot must experience jumps`);assert.equal(tricks,launches);
  if(map==='coast')assert.ok(wet>5*60,'the race must include a substantial underwater stretch');
  assert.ok(decodeSnapshot(JSON.parse(JSON.stringify(s)),s.config));
 }
});
