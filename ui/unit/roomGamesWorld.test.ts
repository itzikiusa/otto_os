import {test} from 'node:test';
import assert from 'node:assert/strict';
import {roadStrip} from '../src/modules/rooms/games/environment.ts';
import * as T from 'three';
import {TRACKS} from '../src/modules/rooms/games/maps.ts';
import {createGame,defaultInput,stepGame} from '../src/modules/rooms/games/simulation.ts';

test('road and curb vertices follow the authoritative raised and submerged course heights',()=>{
 const route=[{x:-20,y:0,z:-20},{x:20,y:5,z:-20},{x:20,y:-4,z:20},{x:-20,y:0,z:20}];
 for(const [left,right] of [[-5,5],[-5.6,-5.05],[5.05,5.6]]){
  const geometry=roadStrip(route,left,right,.04),positions=geometry.getAttribute('position');
  for(let i=0;i<positions.count;i++)assert.ok(Math.abs(positions.getY(i)-(route[Math.floor(i/2)%route.length].y+.04))<1e-5);
  const normals=geometry.getAttribute('normal');
  for(let i=0;i<normals.count;i++)assert.ok(normals.getY(i)>.5);
  geometry.dispose();
 }
});

test('grounded karts match rendered road triangles across sloped lanes',()=>{
 for(const track of Object.values(TRACKS)){
  const road=new T.Mesh(roadStrip(track.route,-track.width/2,track.width/2,.045),new T.MeshBasicMaterial({side:T.DoubleSide}));road.updateMatrixWorld();
  const state=createGame({kind:'kart',map:track.id,difficulty:'normal',vsComputer:false});state.phase='playing';state.players[1].x=1000;
  for(let segment=0;segment<track.route.length;segment++)for(const fraction of [.1,.4,.8])for(const offset of [-.9,-.5,0,.5,.9]){
   const a=track.route[segment],b=track.route[(segment+1)%track.route.length],length=Math.hypot(b.x-a.x,b.z-a.z);
   const x=a.x+(b.x-a.x)*fraction-(b.z-a.z)/length*offset*track.width/2,z=a.z+(b.z-a.z)*fraction+(b.x-a.x)/length*offset*track.width/2;
   const hit=new T.Raycaster(new T.Vector3(x,100,z),new T.Vector3(0,-1,0)).intersectObject(road)[0];if(!hit)continue;
   Object.assign(state.players[0],{x,z,y:a.y,speed:0,grounded:true,offTrackTime:0});stepGame(state,[defaultInput(),defaultInput()],1/60);
   assert.ok(Math.abs(state.players[0].y+.045-hit.point.y)<1e-5,`${track.id} segment ${segment}, lateral ${offset}: physics ${state.players[0].y+.045}, mesh ${hit.point.y}`);
  }
  road.geometry.dispose();road.material.dispose();
 }
});

import {placeBeyondArena} from '../src/modules/rooms/games/world-primitives.ts';
test('perimeter decorations cannot enclose diagonal spawn or follow-camera space',()=>{
 for(let i=0;i<28;i++){
  const angle=i/28*Math.PI*2,half=28;
  const object=new T.Mesh(new T.BoxGeometry(11,18,8),new T.MeshBasicMaterial());
  object.position.set(Math.sin(angle)*(half+4),9,Math.cos(angle)*(half+4));
  object.rotation.y=angle;
  placeBeyondArena(object,half,2);
  const bounds=new T.Box3().setFromObject(object);
  assert.ok(bounds.min.x>=half+2-1e-9||bounds.max.x<=-half-2+1e-9||bounds.min.z>=half+2-1e-9||bounds.max.z<=-half-2+1e-9,`decoration ${i} must lie completely beyond the playable square`);
  object.geometry.dispose();object.material.dispose();
 }
});
