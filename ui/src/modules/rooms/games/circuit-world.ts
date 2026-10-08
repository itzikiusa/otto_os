import * as T from 'three';
import {trackFor} from './maps.ts';
import {sampleTrack} from './track-course.ts';
import {material,mesh,prop,roadStrip,sign,distanceToRoute} from './world-primitives.ts';
import {surfaceMaterial} from './surfaces.ts';
import type {GameConfig,GameState,Vec3} from './types';

type Palette={ground:number;road:number;accent:number;sky:number};
/** Every road, marking, prop anchor and bridge consumes the simulation route's Y. */
export function buildCircuitWorld(scene:T.Scene,kit:T.Object3D,config:GameConfig,palette:Palette){
 const track=trackFor(config.map),route=track.route,half=track.width/2;
 const coast=config.map==='coast',forest=config.map==='forest',neon=config.map==='neon';
 const course=track;
 const waterLevel=course.waterLevel??-.8;
 const animated:T.Object3D[]=[],fish:{object:T.Object3D;origin:T.Vector3;phase:number}[]=[],fans:T.Object3D[]=[];
 const white=material(0xf3eddc),wood=material(0x785035),iron=material(0x243a4a,.5,.4),paint=material(palette.accent),cream=material(0xffe6be),roof=material(forest?0xa94530:0xc16546);
 const glow=new T.MeshBasicMaterial({color:neon?0x5cf5f0:0xffe6a2}),coralMaterials=[0xee7469,0xae78cf,0xedbd69].map(color=>material(color));
 const terrainMaterial=surfaceMaterial(coast?'sand':forest?'grass':'metal',palette.ground,24);
 const groundY=coast?-8:neon?-5:-1;
 const terrain=mesh(scene,new T.PlaneGeometry(340,340,120,120),terrainMaterial,0,0,0);
 terrain.rotation.x=-Math.PI/2;terrain.castShadow=false;
 // Terrain and roadside props use the same exact route projection as physics.
 function terrainY(x:number,z:number){
  const sample=sampleTrack(track,x,z),y=sample.point.y,distance=sample.distance;
  if(neon)return groundY;
  if(coast){const plateau=y>-.2?y-.3:y-1,blend=T.MathUtils.clamp(1-(distance-half-15)/22,0,1);return plateau*blend+groundY*(1-blend);}
  const rolling=Math.sin(x*.035)*Math.cos(z*.045)*4-1,blend=T.MathUtils.clamp(1-(distance-half)/18,0,1);
  return rolling*(1-blend)+(y-.35)*blend;
 }
 const positions=terrain.geometry.getAttribute('position');
 for(let i=0;i<positions.count;i++)positions.setZ(i,terrainY(positions.getX(i),-positions.getY(i)));
 terrain.geometry.computeVertexNormals();
 // Wide shoulders make the route part of its landscape instead of a floating ribbon.
 const shoulder=mesh(scene,roadStrip(route,-half-5,half+5,-.10),surfaceMaterial(coast?'sand':forest?'grass':'metal',coast?0xd9c89b:forest?0x789d50:0x253243,3),0,0,0);shoulder.castShadow=false;
 if(neon){const deck=mesh(scene,roadStrip(route,-half-.9,half+.9,-.28),iron,0,0,0);deck.castShadow=false;}
 const road=mesh(scene,roadStrip(route,-half,half,.045),surfaceMaterial(coast?'asphalt':forest?'asphalt':'metal',neon?0x28324e:0x3b4349),0,0,0);road.castShadow=false;
 for(const side of [-1,1]){
  mesh(scene,roadStrip(route,side*(half+.05),side*(half+.5),.055),paint,0,0,0).castShadow=false;
  mesh(scene,roadStrip(route,side*(half-.22),side*(half-.1),.06),white,0,0,0).castShadow=false;
 }
 function anchor(index:number,side:number,offset:number):{p:Vec3;yaw:number;x:number;z:number}{
  const p=route[index%route.length],q=route[(index+1)%route.length],yaw=Math.atan2(q.x-p.x,q.z-p.z);
  return {p,yaw,x:p.x+Math.cos(yaw)*offset*side,z:p.z-Math.sin(yaw)*offset*side};
 }
 function kitAt(name:string,x:number,y:number,z:number,scale=1,yaw=0){const o=prop(kit,scene,name,x,z,scale,yaw);if(o)o.position.y=y;return o;}
 function house(x:number,y:number,z:number,yaw:number,barn=false){
  const authored=kitAt(barn?'Barn':'Cottage',x,y,z,barn?1.1:1,yaw);if(authored)return;
  const group=new T.Group();group.position.set(x,y,z);group.rotation.y=yaw;scene.add(group);
  mesh(group,new T.BoxGeometry(5,3,4),barn?roof:cream,0,1.5,0);
  const top=mesh(group,new T.ConeGeometry(4,2.1,4),barn?iron:roof,0,4,0);top.rotation.y=Math.PI/4;top.scale.z=.82;
  mesh(group,new T.BoxGeometry(1,1.9,.12),wood,0,.95,-2.05);
  for(const side of [-1,1]){mesh(group,new T.BoxGeometry(.9,.9,.15),glow,side*1.65,1.85,-2.08);mesh(group,new T.BoxGeometry(1.1,.1,.2),white,side*1.65,1.3,-2.12);}
  mesh(group,new T.BoxGeometry(.65,2,.6),cream,1.5,4.3,.5);
 }
 function fence(x:number,y:number,z:number,yaw:number,length=4){
  const group=new T.Group();group.position.set(x,y,z);group.rotation.y=yaw;scene.add(group);
  for(const end of [-1,1])mesh(group,new T.BoxGeometry(.14,1.05,.14),white,end*length/2,.52,0);
  for(const height of [.35,.78])mesh(group,new T.BoxGeometry(length,.12,.1),white,0,height,0);
 }
 function bridge(index:number){
  const {p,yaw}=anchor(index,1,0);
  for(const side of [-1,1]){
   const a=anchor(index,side,half+.8),base=neon?groundY:terrainY(a.x,a.z)-.3,height=Math.max(.5,p.y-base);
   mesh(scene,new T.CylinderGeometry(.4,.65,height,8),neon?iron:cream,a.x,base+height/2,a.z);
   fence(a.x,p.y,a.z,yaw+Math.PI/2,4.5);
  }
  const girder=mesh(scene,new T.BoxGeometry(track.width+2,.6,1),neon?iron:wood,p.x,p.y-.35,p.z);girder.rotation.y=yaw;
 }
 // Markings follow the route's tangent and slope, including submerged sections.
 route.forEach((p,i)=>{
  const q=route[(i+1)%route.length],length=Math.hypot(q.x-p.x,q.z-p.z),yaw=Math.atan2(q.x-p.x,q.z-p.z);
  if(i%2===0){const dash=mesh(scene,new T.BoxGeometry(.15,.015,Math.min(1.4,length*.55)),white,p.x,p.y+.065,p.z);dash.rotation.y=yaw;dash.rotation.x=-Math.atan2(q.y-p.y,length);dash.castShadow=false;}
  if(i%3===0&&p.y>1.1)bridge(i);
  if(i%6===0){
   const a=anchor(i,1,half+1.5);mesh(scene,new T.CylinderGeometry(.1,.16,4.8,6),iron,a.x,p.y+2.4,a.z);
   const lamp=mesh(scene,new T.BoxGeometry(1.8,.18,.4),glow,a.x,p.y+4.8,a.z);lamp.rotation.y=yaw;
   const board=sign(i===0?'GO!':coast?(p.y<waterLevel?'REEF':'COAST'):forest?'COUNTRY':'SKYLINE',i===0?'START / FINISH':'CIRCUIT CLASH',palette.accent);
   board.position.set(a.x,p.y+2.5,a.z);board.rotation.y=yaw+Math.PI;scene.add(board);
  }
 });
 const start=route[0],next=route[1],startYaw=Math.atan2(next.x-start.x,next.z-start.z);
 kitAt('RaceGantry',start.x,start.y,start.z,1.35,startYaw);
 for(let i=0;i<10;i++)for(let j=0;j<2;j++){
  const x=(i-4.5)*track.width/10,z=(j-.5)*.7,check=mesh(scene,new T.BoxGeometry(track.width/10,.025,.7),(i+j)%2?white:iron,start.x+Math.cos(startYaw)*x+Math.sin(startYaw)*z,start.y+.08,start.z-Math.sin(startYaw)*x+Math.cos(startYaw)*z);check.rotation.y=startYaw;
 }
 for(const ramp of course.ramps??[]){
  for(let j=-2;j<=0;j++){
   const index=(ramp.index+j+route.length)%route.length,p=route[index],q=route[(index+1)%route.length],yaw=Math.atan2(q.x-p.x,q.z-p.z);
   const stripe=mesh(scene,new T.BoxGeometry(track.width*.85,.035,.3),paint,p.x,p.y+.09,p.z);stripe.rotation.y=yaw;
   for(const side of [-1,1]){const a=anchor(index,side,half+.6);mesh(scene,new T.ConeGeometry(.3,1.5,6),paint,a.x,p.y+.75,a.z);}
  }
  const a=anchor(ramp.index,1,half+2),board=sign('JUMP','AIR TRICK / SPACE',palette.accent);board.position.set(a.x,a.p.y+2.5,a.z);board.rotation.y=a.yaw+Math.PI;scene.add(board);
 }
 if(coast){
  // Resort buildings cluster along the dry start, leaving the reef approach open.
  for(let i=0;i<route.length;i+=3){const a=anchor(i,1,half+9);if(a.p.y<-.15)continue;if(distanceToRoute(a.x,a.z,route)<half+5)continue;
   const base=terrainY(a.x,a.z);
   if(i%6===0)house(a.x,base,a.z,a.yaw+Math.PI/2);
   else {kitAt('PalmTree',a.x,base,a.z,1.4,a.yaw);const umbrella=mesh(scene,new T.ConeGeometry(2,.8,10),i%2?paint:roof,a.x,base+2.3,a.z);umbrella.rotation.y=.2;mesh(scene,new T.CylinderGeometry(.06,.06,2.2,6),wood,a.x,base+1.1,a.z);}
  }
  for(let i=0;i<route.length;i++){
   const p=route[i];if(p.y>=waterLevel-.3)continue;
   if(i%3===0){const arch=mesh(scene,new T.TorusGeometry(half+1.8,.15,7,28,Math.PI),white,p.x,p.y+.4,p.z);arch.rotation.y=anchor(i,1,0).yaw;}
   for(const side of [-1,1]){
    const a=anchor(i,side,half+2+(i%3)*2),base=terrainY(a.x,a.z);
    const coral=kitAt(i%2?'CoralFan':'CoralCluster',a.x,base,a.z,.8+(i%3)*.3,a.yaw);
    if(!coral){const group=new T.Group();group.position.set(a.x,base,a.z);scene.add(group);for(let branch=0;branch<5;branch++){const stem=mesh(group,new T.CylinderGeometry(.08,.2,1+branch*.22,6),coralMaterials[i%3],(branch-2)*.35,.5+branch*.11,Math.sin(branch)*.3);stem.rotation.z=(branch-2)*.18;}}
    kitAt('Seaweed',a.x+1,terrainY(a.x+1,a.z+1),a.z+1,1+(i%2)*.5);
    if(i%3===0)kitAt('ReefRock',a.x,base-.2,a.z,.8,a.yaw);
    if(i%2===0){const object=kitAt('Fish',a.x,p.y+2,a.z,.7,a.yaw)??mesh(scene,new T.SphereGeometry(.35,10,6),paint,a.x,p.y+2,a.z);object.scale.z*=1.6;animated.push(object);fish.push({object,origin:object.position.clone(),phase:i*.8+side});}
   }
  }
 }else if(forest){
  // Village houses, orchard rows, fenced fields and hilltop windmills create districts.
  for(let i=0;i<route.length;i+=3){
   const a=anchor(i,i%2?1:-1,half+9);if(distanceToRoute(a.x,a.z,route)<half+4)continue;
   const base=terrainY(a.x,a.z);
   if(i%9===0)house(a.x,base,a.z,a.yaw+Math.PI/2,i%18===0);
   else kitAt(i%2?'BroadleafTree':'PineTree',a.x,base,a.z,1.4+(i%3)*.25,a.yaw);
   fence(a.x,base,a.z,a.yaw,6);
   if(i%12===0){
    const millY=terrainY(a.x+7,a.z+4),windmill=kitAt('Windmill',a.x+7,millY,a.z+4,1);
    const authoredSails=windmill?.getObjectByName('WindmillSails');if(authoredSails){animated.push(windmill!);fans.push(authoredSails);}
    if(!windmill){mesh(scene,new T.CylinderGeometry(.9,1.7,7,10),cream,a.x+7,millY+3.5,a.z+4);const sails=new T.Group();sails.position.set(a.x+7,millY+5.6,a.z+5);scene.add(sails);for(let b=0;b<4;b++){const blade=mesh(sails,new T.BoxGeometry(.45,3,.12),white,0,1.5,0);blade.geometry.translate(0,1.3,0);blade.position.y=0;blade.rotation.z=b*Math.PI/2;}animated.push(sails);fans.push(sails);}
   }
  }
  const fieldMat=material(0xb89b4e),crop=material(0x9eab49);
  for(let row=0;row<9;row++)for(let col=0;col<8;col++){
   const x=-24+col*5,z=-18+row*4;if(distanceToRoute(x,z,route)<half+5)continue;
   const y=terrainY(x,z)+.06;
   mesh(scene,new T.BoxGeometry(4,.1,3),fieldMat,x,y,z);
   for(let n=0;n<4;n++)mesh(scene,new T.BoxGeometry(.12,.35,2.8),crop,x-1.5+n,y+.2,z);
  }
 }else{
  for(let i=0;i<30;i++){
   const a=i*2.399,r=24+(i*13%62),x=Math.sin(a)*r,z=Math.cos(a)*r;if(distanceToRoute(x,z,route)<half+7)continue;
   kitAt('NeonTower',x,groundY,z,1+(i%5)*.45,a);
  }
  for(let i=0;i<route.length;i+=8){
   const a=anchor(i,-1,half+5);const board=sign(['OTTO','ARCADE','NOVA','CLASH'][i/8%4|0],'MIDNIGHT EXPRESS',i%16?0xff82ca:0x69ecf3);board.scale.set(3,2,1);board.position.set(a.x,a.p.y+5,a.z);board.rotation.y=a.yaw;scene.add(board);
  }
 }
 const pickups=track.pickups.map(index=>{const p=route[index%route.length],object=kitAt('Pickup',p.x,p.y+.7,p.z,1)??mesh(scene,new T.OctahedronGeometry(.55),glow,p.x,p.y+.7,p.z);object.userData.baseY=p.y+.7;return object;});
 let water:T.Mesh|undefined;
 if(coast){
  const waterMat=surfaceMaterial('water',0x329cae,40);waterMat.transparent=true;waterMat.opacity=.4;waterMat.depthWrite=false;waterMat.side=T.DoubleSide;
  water=mesh(scene,new T.PlaneGeometry(340,340),waterMat,0,waterLevel,0);water.rotation.x=-Math.PI/2;water.castShadow=false;animated.push(water);
 }
 const underwaterColor=new T.Color(0x0b7582),underwaterSky=new T.Color(0x095b70),normalSky=new T.Color(palette.sky);
 const sky=scene.getObjectByName('Sky'),fog=scene.fog instanceof T.Fog?scene.fog:null,normalFog=fog?.color.clone(),normalNear=fog?.near??80,normalFar=fog?.far??220;
 let time=0,submerged=0;
 return {pickups,animated,update(state:GameState,dt:number,player=0,cameraY?:number){
  time+=dt;const underwater=coast&&(cameraY??state.players[player].y+3)<waterLevel;
  submerged=T.MathUtils.lerp(submerged,underwater?1:0,1-Math.exp(-dt*5));
  if(sky)sky.visible=submerged<.9;
  if(fog&&normalFog){fog.color.copy(normalFog).lerp(underwaterColor,submerged);fog.near=T.MathUtils.lerp(normalNear,2,submerged);fog.far=T.MathUtils.lerp(normalFar,42,submerged);}
  if(coast)(scene.background as T.Color).copy(normalSky).lerp(underwaterSky,submerged);
  for(const f of fish){f.object.position.set(f.origin.x+Math.sin(time*.7+f.phase)*2,f.origin.y+Math.sin(time*1.4+f.phase)*.3,f.origin.z+Math.cos(time*.7+f.phase)*2);f.object.rotation.y=time*.7+f.phase+Math.PI/2;}
  for(const fan of fans)fan.rotation.z+=dt*.6;
  if(water){const mat=water.material as T.MeshStandardMaterial;if(mat.map){mat.map.offset.x=time*.008;mat.map.offset.y=Math.sin(time*.07)*.05;}}
 }};
}
