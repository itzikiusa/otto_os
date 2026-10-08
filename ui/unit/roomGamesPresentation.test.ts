import * as T from 'three';
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {followVehicle, vehicleWheels, type VehiclePose} from '../src/modules/rooms/games/presentation.ts';

test('visual yaw follows the short arc through the signed-angle boundary', () => {
 const current:VehiclePose={x:0,y:0,z:0,yaw:Math.PI-.01};
 const next=followVehicle(current,{...current,yaw:-Math.PI+.01},1/144);
 assert.ok(Math.abs(next.yaw-current.yaw)<.02,'must not spin almost a full turn');
});
test('presentation converges consistently at 30,60 and144 Hz', () => {
 const target={x:2,y:0,z:4,yaw:1};
 const poses=[30,60,144].map(rate=>{
  let pose={x:0,y:0,z:0,yaw:0};
  for(let i=0;i<rate;i++)pose=followVehicle(pose,target,1/rate);
  return pose;
 });
 for(const p of poses){assert.ok(Math.abs(p.x-target.x)<.001);assert.ok(Math.abs(p.yaw-target.yaw)<.001);}
 assert.ok(Math.abs(poses[0].x-poses[2].x)<.0001);
});
test('sub-tick frames interpolate while recovery snaps once to the new location', () => {
 const origin={x:0,y:0,z:0,yaw:0}; const target={...origin,z:.4};
 const first=followVehicle(origin,target,1/144),second=followVehicle(first,target,1/144);
 assert.ok(first.z>0&&first.z<second.z&&second.z<target.z);
 const recovered={...origin,x:30}; assert.deepEqual(followVehicle(second,recovered,1/60),recovered);
});

test('steered wheels keep their axle horizontal through a complete revolution',()=>{
 const root=new T.Group();for(const name of ['WheelFL','WheelFR','WheelRL','WheelRR']){const wheel=new T.Group();wheel.name=name;root.add(wheel);}
 for(const wheel of vehicleWheels(root))for(const spin of [0,Math.PI/2,Math.PI,Math.PI*1.5]){
  wheel.rotation.x=spin;wheel.rotation.y=.32;
  const axle=new T.Vector3(1,0,0).applyEuler(wheel.rotation);
  assert.ok(Math.abs(axle.y)<1e-9,'rolling must not wobble the steering axle');
 }
});
