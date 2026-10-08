import * as T from 'three';
import {material,mesh,sign,placeBeyondArena} from './world-primitives.ts';
import type {ArenaMap,GameConfig,GameState} from './types';

/** Architecture stays within solid cover footprints, above head clearance, or outside the arena. */
export function dressArena(scene:T.Scene,arena:ArenaMap,config:GameConfig,accent:number){
 const n=arena.halfSize,dunes=config.map==='dunes',foundry=config.map==='foundry';
 const stone=material(0xd4b17c),dark=material(foundry?0x382e30:0x182b3b,.6,.4),steel=material(0x71868c,.6,.45),copper=material(0xb36f48,.6,.4),leaf=material(0x5f8662);
 const glow=new T.MeshBasicMaterial({color:accent}),animated:T.Object3D[]=[],fans:T.Group[]=[];
 function beam(a:T.Vector3,b:T.Vector3,radius:number,mat:T.Material){const delta=b.clone().sub(a);const o=mesh(scene,new T.CylinderGeometry(radius,radius,delta.length(),8),mat,...a.clone().add(b).multiplyScalar(.5).toArray() as [number,number,number]);o.quaternion.setFromUnitVectors(new T.Vector3(0,1,0),delta.normalize());return o;}
 if(dunes){
  // A roofless desert observatory: columns and broken pediments frame the playable courtyard.
  for(const side of [-1,1])for(let i=-2;i<=2;i++){
   const x=i*11,z=side*(n+4);
   mesh(scene,new T.CylinderGeometry(.9,1.3,8+(i%2),12),stone,x,4,z);
   mesh(scene,new T.BoxGeometry(3,.6,3),stone,x,8.1,z);
   mesh(scene,new T.BoxGeometry(10.8,.8,2.2),stone,x,8.8,z);
   for(let k=0;k<4;k++){const vine=mesh(scene,new T.IcosahedronGeometry(.65,0),leaf,x+.8,6-k*.8,z+.9);vine.scale.set(1,.65,.6);}
  }
  for(const side of [-1,1]){
   const arch=mesh(scene,new T.TorusGeometry(7,1.2,8,24,Math.PI),stone,side*(n+4),4,0);arch.rotation.y=Math.PI/2;
   for(const z of [-7,7])mesh(scene,new T.CylinderGeometry(1.2,1.5,4,10),stone,side*(n+4),2,z);
  }
  arena.cover.forEach((c,i)=>{
   // Cornices, shallow pilasters and vegetation never widen the collision footprint.
   mesh(scene,new T.BoxGeometry(c.width,.18,c.depth),stone,c.x,c.y+c.height-.09,c.z);
   for(const side of [-1,1])for(const end of [-1,1])mesh(scene,new T.BoxGeometry(.28,c.height,.28),stone,c.x+side*(c.width/2-.14),c.y+c.height/2,c.z+end*(c.depth/2-.14));
   if(i%2===0)for(let k=0;k<4;k++){const shrub=mesh(scene,new T.IcosahedronGeometry(.35,0),leaf,c.x+(k-1.5)*.4,c.y+c.height+.1,c.z);shrub.scale.y=.5;}
  });
  // Low dune ridges lie beyond the court, leaving long-range sightlines uncluttered.
  for(let i=0;i<14;i++){const a=i/14*Math.PI*2;const dune=mesh(scene,new T.SphereGeometry(1,18,10),stone,Math.sin(a)*(n+24),-2,Math.cos(a)*(n+24));dune.scale.set(17,6+(i%3),12);placeBeyondArena(dune,n,4);}
 }else{
  // High side canopies and exposed trusses give the arena a believable industrial envelope.
  for(const side of [-1,1]){
   mesh(scene,new T.BoxGeometry(10,.5,n*2+6),dark,side*(n-2),10.5,0);
   for(let z=-n;z<=n;z+=8){
    beam(new T.Vector3(side*(n+1),0,z),new T.Vector3(side*(n+1),10,z),.25,steel);
    beam(new T.Vector3(side*(n+1),8,z),new T.Vector3(side*(n-6),10,z),.14,steel);
   }
   for(let j=0;j<3;j++)beam(new T.Vector3(side*(n+.8),6+j*.5,-n),new T.Vector3(side*(n+.8),6+j*.5,n),.18,j===1?copper:steel);
   // Turbine housings remain beyond the play boundary; the rotors visibly turn.
   for(const z of [-12,12]){
    const rotor=new T.Group();rotor.position.set(side*(n+1),5.5,z);rotor.rotation.y=side*Math.PI/2;scene.add(rotor);
    const housing=mesh(rotor,new T.TorusGeometry(2,.3,8,24),steel,0,0,0);housing.castShadow=false;
    const spin=new T.Group();rotor.add(spin);
    for(let b=0;b<6;b++){const blade=mesh(spin,new T.BoxGeometry(.5,1.65,.12),dark,0,0,0);blade.geometry.translate(0,1,0);blade.rotation.z=b*Math.PI/3;}
    mesh(spin,new T.SphereGeometry(.45,10,8),copper,0,0,.2);animated.push(rotor);fans.push(spin);
   }
  }
  if(foundry){
   // Suspended furnace ducts and orange molten channels sit above clear player headroom.
   for(const x of [-6,6]){
    mesh(scene,new T.CylinderGeometry(1.1,1.6,5,16),copper,x,7.5,0);
    for(const y of [5.2,6.2,8.5])mesh(scene,new T.TorusGeometry(1.4,.15,6,20),steel,x,y,0).rotation.x=Math.PI/2;
    beam(new T.Vector3(x,10,0),new T.Vector3(x,10,n+7),.65,dark);
   }
   for(const side of [-1,1]){const furnace=mesh(scene,new T.CylinderGeometry(4,5,17,18),dark,side*17,8.5,n+11);furnace.castShadow=false;for(const y of [4,7,10,13])mesh(scene,new T.TorusGeometry(4.3,.22,7,24),glow,side*17,y,n+11).rotation.x=Math.PI/2;}
  }else{
   // The central collision block becomes a contained reactor rather than a loose crate.
   const c=arena.cover[0],reactor=mesh(scene,new T.CylinderGeometry(1.15,1.4,2.7,20),steel,c.x,c.height+1.35,c.z);
   reactor.name='Orbital reactor';
   for(const y of [.3,1,1.7,2.4])mesh(scene,new T.TorusGeometry(1.3,.12,8,24),glow,c.x,c.height+y,c.z).rotation.x=Math.PI/2;
   mesh(scene,new T.SphereGeometry(.95,16,12),glow,c.x,c.height+1.3,c.z);
   for(const side of [-1,1]){const solar=mesh(scene,new T.BoxGeometry(15,.12,10),material(0x30558a,.5,.35),side*(n+12),14,8);solar.rotation.z=side*.3;beam(new T.Vector3(side*(n+3),7,8),solar.position,.35,steel);}
  }
  for(const side of [-1,1]){const board=sign(foundry?'EMBER':'ORBITAL',foundry?'REACTOR HALL / 07':'LOW ORBIT / ARENA 01',accent);board.scale.set(3,2,1);board.position.set(0,7,side*(n+.4));board.rotation.y=side<0?0:Math.PI;scene.add(board);}
 }
 const pickups=arena.pickups.map((p)=>{
  const root=new T.Group();root.position.set(p.x,p.y+.85,p.z);root.userData.baseY=p.y+.85;scene.add(root);
  const color=new T.MeshBasicMaterial({color:p.kind==='health'?0x79e3b6:0x77bbff});
  mesh(root,new T.TorusGeometry(.6,.07,6,20),color,0,0,0);
  if(p.kind==='health'){mesh(root,new T.BoxGeometry(.22,.75,.16),color,0,0,0);mesh(root,new T.BoxGeometry(.7,.22,.16),color,0,0,0);}else mesh(root,new T.OctahedronGeometry(.45),color,0,0,0);
  return root;
 });
 return {pickups,animated,update(_state:GameState,dt:number){for(const fan of fans)fan.rotation.z+=dt*(foundry?1.8:.8);}};
}
