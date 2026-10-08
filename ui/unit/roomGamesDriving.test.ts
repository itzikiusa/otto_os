import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createGame,defaultInput,stepGame,FIXED_STEP} from '../src/modules/rooms/games/simulation.ts';
import {TRACKS} from '../src/modules/rooms/games/maps.ts';
import {angleDelta,distance} from '../src/modules/rooms/games/geometry.ts';
import type {GameInput} from '../src/modules/rooms/games/types.ts';

function driving(offset=0) {
 const s=createGame({kind:'kart',map:'coast',difficulty:'normal',vsComputer:false});s.phase='playing';
 const [a,b]=TRACKS.coast.route,dx=b.x-a.x,dz=b.z-a.z,length=Math.hypot(dx,dz);
 Object.assign(s.players[0],{x:(a.x+b.x)/2+dz/length*offset,z:(a.z+b.z)/2-dx/length*offset,yaw:Math.atan2(dx,dz)});
 Object.assign(s.players[1],TRACKS.coast.route[5]);
 return s;
}
function tick(s:ReturnType<typeof driving>,input:Partial<GameInput>={}) {
 stepGame(s,[{...defaultInput(),...input},defaultInput()],FIXED_STEP);
}

test('entering grass slows a fast kart gradually without a position jump',()=>{
 const s=driving(7),p=s.players[0];p.speed=23;
 for(let n=0;n<60;n++) {
  const before={...p};tick(s,{moveZ:1});
  assert.ok(p.speed<=before.speed);
  assert.ok(before.speed-p.speed<=13*FIXED_STEP,'terrain drag must not instantly clamp to grass speed');
  assert.ok(distance(before,p)<=before.speed*FIXED_STEP+1e-9,'terrain transition must not teleport');
 }
 assert.ok(p.speed<15&&p.speed>=8);
});

test('boost expiry decelerates toward road speed rather than snapping ten metres per second',()=>{
 const s=driving(),p=s.players[0];p.speed=33;p.boost=FIXED_STEP/2;
 tick(s,{moveZ:1});
 assert.equal(p.boost,0);
 assert.ok(p.speed<33&&p.speed>32.7);
});

test('throttle and braking have bounded speed changes and neutral coasts to rest',()=>{
 const s=driving(),p=s.players[0];
 tick(s,{moveZ:1});assert.ok(p.speed>0&&p.speed<=16*FIXED_STEP+1e-9);
 p.speed=10;tick(s,{moveZ:-1});assert.ok(p.speed>=10-24*FIXED_STEP-1e-9&&p.speed<10);
 for(const speed of [-4,0,4]) {
  const coast=driving(),kart=coast.players[0];kart.speed=speed;
  for(let n=0;n<180;n++) {
   const before=kart.speed;tick(coast);
   assert.ok(Math.abs(kart.speed)<=Math.abs(before));
   assert.ok(speed<0?kart.speed<=0:kart.speed>=0,'coasting must not cross through zero');
  }
  assert.equal(kart.speed,0);
 }
});

test('steering eases in and reverses gradually instead of changing full turn rate in one tick',()=>{
 const s=driving(),p=s.players[0];p.speed=23;
 const initialYaw=p.yaw;tick(s,{moveX:1,moveZ:1});
 assert.ok(Math.abs(angleDelta(p.yaw,initialYaw))<.005,'first steering tick should ramp in');
 for(let n=0;n<12;n++)tick(s,{moveX:1,moveZ:1});
 const beforeReverse=p.yaw;tick(s,{moveX:-1,moveZ:1});
 assert.ok(angleDelta(p.yaw,beforeReverse)<0,'steering wheel cannot reverse instantly');
 for(let n=0;n<20;n++)tick(s,{moveX:-1,moveZ:1});
 const afterReverse=p.yaw;tick(s,{moveX:-1,moveZ:1});
 assert.ok(angleDelta(p.yaw,afterReverse)>0,'sustained opposite input must turn the other way');
});

test('full steering has a lower angular rate at racing speed than at cornering speed',()=>{
 const turn=(speed:number)=>{const s=driving(),p=s.players[0];p.speed=speed;p.steering=1;const yaw=p.yaw;tick(s,{moveX:1,moveZ:1});return Math.abs(angleDelta(p.yaw,yaw));};
 const slow=turn(10),fast=turn(23);
 assert.ok(fast<slow*.9);assert.ok(fast<1.5*FIXED_STEP);
});

test('nearby grass never triggers an automatic checkpoint teleport',()=>{
 const s=driving(7),p=s.players[0],before={...p};
 for(let n=0;n<8*60;n++)tick(s);
 assert.ok(p.offTrack);assert.ok(p.offTrackTime>7);
 assert.equal(distance(before,p),0);assert.equal(p.checkpoint,before.checkpoint);assert.equal(p.lap,before.lap);
});

test('distant recovery and explicit reset preserve earned progress and clear steering',()=>{
 for(const manual of [false,true]) {
  const s=driving(),p=s.players[0];p.checkpoint=3;p.lap=1;p.steering=.7;
  Object.assign(p,manual?{x:12,z:-45}:{x:900,z:900});
  for(let n=0;n<(manual?1:4.2*60);n++)tick(s,{reset:manual});
  assert.equal(p.checkpoint,3);assert.equal(p.lap,1);assert.equal(p.steering,0);
  assert.equal(distance(p,TRACKS.coast.route[2]),0);
 }
});
