import {test} from 'node:test';
import assert from 'node:assert/strict';
import {roadStrip} from '../src/modules/rooms/games/environment.ts';
import {TRACKS} from '../src/modules/rooms/games/maps.ts';
test('all race surfaces and both curb edges face the camera above the ground',()=>{
 for(const track of Object.values(TRACKS))for(const [left,right] of [[-track.width/2,track.width/2],[-track.width/2-.05,-track.width/2-.6],[track.width/2+.05,track.width/2+.6]]){
  const geometry=roadStrip(track.route,left,right,.04);const normals=geometry.getAttribute('normal');
  for(let i=0;i<normals.count;i++)assert.ok(normals.getY(i)>.9,`${track.id} surface normal must face upward`);
  geometry.dispose();
 }
});
