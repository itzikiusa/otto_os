import * as T from 'three';
import {RoundedBoxGeometry} from 'three/examples/jsm/geometries/RoundedBoxGeometry.js';
import {surfaceMaterial,skyDome} from './surfaces.ts';
import {arenaFor, trackFor} from './maps.ts';
import type {GameConfig, Vec3} from './types';

export const PALETTES: Record<string, {sky:number; ground:number; road:number; accent:number; sun:number; fog:number}> = {
  station: {sky:0x131e38,ground:0x344356,road:0x3d485e,accent:0x67daee,sun:0xc2dcff,fog:120},
  foundry: {sky:0x352b38,ground:0x484047,road:0x453744,accent:0xff925c,sun:0xffc694,fog:110},
  dunes: {sky:0xbfdce5,ground:0xc89d65,road:0xac824f,accent:0x37c5cb,sun:0xffecd1,fog:170},
  coast: {sky:0x8fd8ee,ground:0xe8ce98,road:0x444f60,accent:0x38d3ca,sun:0xfff1cf,fog:220},
  forest: {sky:0xb9d8cb,ground:0x668b62,road:0x65564c,accent:0xffd26f,sun:0xffedc7,fog:190},
  neon: {sky:0x151e3e,ground:0x202b44,road:0x303b54,accent:0xbb86ff,sun:0xabc4ff,fog:200},
};
export function material(color: number, metalness = 0, roughness = .8): T.MeshStandardMaterial { return new T.MeshStandardMaterial({color, metalness, roughness}); }
function mesh(parent:T.Object3D, geometry:T.BufferGeometry, mat:T.Material, x:number,y:number,z:number):T.Mesh {
  const object = new T.Mesh(geometry,mat); object.position.set(x,y,z); object.castShadow = true; object.receiveShadow = true; parent.add(object); return object;
}
/** Clone one authored kit root; optional exact collision-box dimensions. */
export function prop(kit:T.Object3D, parent:T.Object3D, name:string, x:number,z:number, scale=1, yaw=0, size?:{width:number;height:number;depth:number}):T.Object3D | null {
  const source = kit.getObjectByName(name); if (!source) return null;
  const root = source.clone(true); root.position.set(0,0,0); root.rotation.set(0,0,0); root.scale.setScalar(1);
  const bounds = new T.Box3().setFromObject(root); const extent = bounds.getSize(new T.Vector3());
  const holder = new T.Group(); holder.add(root);
  root.position.set(-(bounds.min.x+bounds.max.x)/2, -bounds.min.y, -(bounds.min.z+bounds.max.z)/2);
  holder.scale.set(size ? size.width / Math.max(.01,extent.x) : scale, size ? size.height / Math.max(.01,extent.y) : scale, size ? size.depth / Math.max(.01,extent.z) : scale);
  holder.position.set(x,0,z); holder.rotation.y = yaw;
  holder.traverse(o => { if (o instanceof T.Mesh) { o.castShadow = true; o.receiveShadow = true; } });
  parent.add(holder); return holder;
}
export function roadStrip(route:Vec3[], left:number, right:number, y:number):T.BufferGeometry {
  [left,right]=[Math.min(left,right),Math.max(left,right)];
  const positions:number[] = [], uv:number[] = [], indices:number[] = [];
  const n = route.length;let travelled=0;
  for (let i=0;i<=n;i++) {
    const p=route[i%n], prev=route[(i+n-1)%n], next=route[(i+1)%n];
    if(i>0)travelled+=Math.hypot(p.x-prev.x,p.z-prev.z);
    const a=new T.Vector2(p.x-prev.x,p.z-prev.z).normalize(), b=new T.Vector2(next.x-p.x,next.z-p.z).normalize();
    const normal = new T.Vector2(-(a.y+b.y),a.x+b.x).normalize();
    const correction = 1 / Math.max(.45, normal.dot(new T.Vector2(-b.y,b.x)));
    for (const offset of [left,right]) { positions.push(p.x+normal.x*offset*correction,y,p.z+normal.y*offset*correction); uv.push((offset-left)/Math.max(.01,right-left), travelled/8); }
    if(i<n) {const k=i*2; indices.push(k,k+1,k+2,k+1,k+3,k+2);}
  }
  const geometry=new T.BufferGeometry(); geometry.setAttribute('position',new T.Float32BufferAttribute(positions,3)); geometry.setAttribute('uv',new T.Float32BufferAttribute(uv,2)); geometry.setIndex(indices); geometry.computeVertexNormals(); return geometry;
}
function distanceToRoute(x:number,z:number, route:Vec3[]):number {
  let best=Infinity;
  route.forEach((p,i)=> {const q=route[(i+1)%route.length],dx=q.x-p.x,dz=q.z-p.z,t=Math.max(0,Math.min(1,((x-p.x)*dx+(z-p.z)*dz)/(dx*dx+dz*dz))); best=Math.min(best,Math.hypot(x-p.x-dx*t,z-p.z-dz*t));}); return best;
}
export function buildEnvironment(scene:T.Scene, kit:T.Object3D, config:GameConfig):{pickups:T.Object3D[]} {
  const palette=PALETTES[config.map] ?? PALETTES.station;
  scene.background=new T.Color(palette.sky); scene.fog=new T.Fog(palette.sky,palette.fog*.4,palette.fog);
  const night=config.map==='station'||config.map==='neon'||config.map==='foundry';
  skyDome(scene,night?palette.sky:config.map==='dunes'?0x6290b5:0x2671a5,night?0x1b334b:config.map==='coast'?0x8ad3ee:config.map==='forest'?0xa8cdd7:0xe6cead,night);
  const floor=surfaceMaterial(config.kind==='shooter'?(config.map==='dunes'?'sand':'metal'):(config.map==='forest'?'grass':'asphalt'),palette.ground,config.kind==='shooter'?80:45);
  if(config.map!=='coast'){
    const ground=mesh(scene,new T.PlaneGeometry(300,300),floor,0,-.04,0);ground.rotation.x=-Math.PI/2;ground.castShadow=false;
  }
  const trim=material(night?0x1b2734:0x34434a,.55,.46), accent=material(palette.accent,.25,.5);
  const glowMat=new T.MeshBasicMaterial({color:palette.accent});
  const hemi=new T.HemisphereLight(palette.sky,0x302b25,night?.6:.85); scene.add(hemi);
  const sun=new T.DirectionalLight(palette.sun,night?1.5:2.3); sun.position.set(-30,55,20); sun.castShadow=true;
  sun.shadow.mapSize.set(2048,2048); sun.shadow.camera.left=-65; sun.shadow.camera.right=65; sun.shadow.camera.top=65; sun.shadow.camera.bottom=-65; sun.shadow.camera.far=150; sun.shadow.normalBias=.06; scene.add(sun);
  if(config.kind==='shooter') {
    const arena=arenaFor(config.map), n=arena.halfSize;
    if(config.map!=='dunes') {
      // Recessed approach lanes, embedded guide lights and perimeter architecture.
      for(const side of [-1,1]){
        const lane=mesh(scene,new T.PlaneGeometry(3.5,n*2),material(0x172731,.3,.7),side*17,.008,0);lane.rotation.x=-Math.PI/2;lane.castShadow=false;
        for(let z=-n+3;z<n;z+=6){const light=mesh(scene,new T.BoxGeometry(.09,.02,2),glowMat,side*19,.02,z);light.castShadow=false;}
        for(let z=-n;z<=n;z+=8){mesh(scene,new T.BoxGeometry(.45,8,.55),trim,side*(n+.8),4,z);mesh(scene,new T.BoxGeometry(4,.3,.6),trim,side*(n-.7),7.8,z);}
        for(let x=-n+4;x<n;x+=8){mesh(scene,new T.BoxGeometry(5,.25,.8),trim,x,6,n+1);prop(kit,scene,'PipeRack',x,-n-.7,1.6);}
      }
      for(let i=-n;i<n;i+=6) for(const side of [-1,1]) {
        prop(kit,scene,'StationWall',i,side*n,1,side<0?0:Math.PI,{width:6,height:4,depth:.8});
        prop(kit,scene,'StationWall',side*n,i,1,Math.PI/2,{width:6,height:4,depth:.8});
      }
    } else {
      for(let i=0;i<28;i++) {const a=i/28*Math.PI*2; prop(kit,scene,'DesertRock',Math.sin(a)*(n+4),Math.cos(a)*(n+4),2+(i%3),a);}
    }
    const coverSurface=surfaceMaterial(config.map==='dunes'?'stone':'metal',config.map==='dunes'?0xb0844f:config.map==='foundry'?0x635146:0x294252,1);
    arena.cover.forEach((cover,i)=> {
      if(config.map==='dunes'){
        // Sandstone ruin blocks make the entire collision volume visible.
        const block=mesh(scene,new T.BoxGeometry(cover.width,cover.height,cover.depth),coverSurface,cover.x,cover.y+cover.height/2,cover.z);block.name='CollisionCover';
        prop(kit,scene,'DesertRock',cover.x,cover.z,1,0,{width:cover.width,height:cover.height,depth:cover.depth});
      }else{
        const shell=mesh(scene,new RoundedBoxGeometry(cover.width,cover.height,cover.depth,2,.15),coverSurface,cover.x,cover.y+cover.height/2,cover.z);shell.name='CollisionCover';
        // Armor ribs and vent arrays make collision volumes read as equipment.
        for(const side of [-1,1]){
          mesh(scene,new T.BoxGeometry(cover.width+.04,.18,cover.depth+.04),trim,cover.x,.14,cover.z);
          for(let x=-cover.width/2+.3;x<cover.width/2;x+=1.25){
            mesh(scene,new T.BoxGeometry(.08,cover.height*.8,.08),trim,cover.x+x,cover.height/2,cover.z+side*(cover.depth/2+.015));
          }
          mesh(scene,new T.BoxGeometry(cover.width*.7,.055,.045),glowMat,cover.x,cover.height*.78,cover.z+side*(cover.depth/2+.04));
          for(let v=0;v<4;v++)mesh(scene,new T.BoxGeometry(cover.width*.48,.07,.035),trim,cover.x,.55+v*.15,cover.z+side*(cover.depth/2+.045));
        }
        const panel=sign(`0${i+1}`,config.map==='foundry'?'THERMAL / CAUTION':'ORBITAL / SECTOR',palette.accent);
        panel.position.set(cover.x,cover.height*.56,cover.z-cover.depth/2-.06);panel.rotation.y=Math.PI;scene.add(panel);
      }
    });
    arena.spawns.forEach((p,i)=> {const ring=mesh(scene,new T.RingGeometry(1.1,1.35,40),new T.MeshBasicMaterial({color:i%2?0xff9767:0x66d8ef,side:T.DoubleSide}),p.x,.025,p.z);ring.rotation.x=-Math.PI/2;});
    for(let i=0;i<8;i++){const a=i/8*Math.PI*2;prop(kit,scene,config.map==='dunes'?'DesertArch':'StationColumn',Math.sin(a)*(n+1),Math.cos(a)*(n+1),1.4,a);}
    if(config.map==='station') {
      const stars=new T.BufferGeometry(),positions=[];for(let i=0;i<180;i++){const a=i*2.399;positions.push(Math.sin(a)*130,25+(i*17%100),Math.cos(a)*130);}stars.setAttribute('position',new T.Float32BufferAttribute(positions,3));scene.add(new T.Points(stars,new T.PointsMaterial({color:0xc5e5ff,size:.35,fog:false})));
      const planetMaterial=material(0x6b93bc,.05,.75);planetMaterial.fog=false;mesh(scene,new T.SphereGeometry(18,32,20),planetMaterial,50,48,100);
    }
    return {pickups:[]};
  }
  const track=trackFor(config.map), half=track.width/2;
  const road=mesh(scene,roadStrip(track.route,-half,half,.04),surfaceMaterial('asphalt',config.map==='neon'?0x152337:0x353d43),0,0,0); road.castShadow=false;
  for(const side of [-1,1]) {
    mesh(scene,roadStrip(track.route,side*(half+.05),side*(half+.6),.045),material(palette.accent),0,0,0);
    mesh(scene,roadStrip(track.route,side*(half-.35),side*(half-.2),.05),material(0xf2eee3),0,0,0);
  }
  const start=track.route[0],next=track.route[1],yaw=Math.atan2(next.x-start.x,next.z-start.z);
  const gantry=prop(kit,scene,'RaceGantry',start.x,start.z,1,yaw); if(gantry) gantry.scale.multiplyScalar(1.3);
  for(let i=0;i<10;i++)for(let j=0;j<2;j++) {const x=(i-4.5)*track.width/10,z=(j-.5)*.8;const check=mesh(scene,new T.PlaneGeometry(track.width/10,.8),material((i+j)%2?0xeff2ea:0x273141),start.x+Math.cos(yaw)*x+Math.sin(yaw)*z,.065,start.z-Math.sin(yaw)*x+Math.cos(yaw)*z);check.rotation.set(-Math.PI/2,0,yaw);}
  for(let i=0;i<90;i++) {
    const a=i*2.399,r=18+((i*31)%70),x=Math.sin(a)*r,z=Math.cos(a)*r;
    if(distanceToRoute(x,z,track.route)<half+3||(config.map==='coast'&&Math.hypot(x,z)>59)) continue;
    const name=config.map==='forest'?(i%3?'PineTree':'BroadleafTree'):config.map==='coast'?(i%3?'PalmTree':'CoastalRock'):'NeonTower';
    prop(kit,scene,name,x,z,.8+(i%4)*.35,a);
  }
  track.route.forEach((p,i)=>{const q=track.route[(i+1)%track.route.length],dx=q.x-p.x,dz=q.z-p.z,len=Math.hypot(dx,dz);for(let j=0;j<Math.floor(len/7);j++){const t=(j+.5)/Math.floor(len/7);prop(kit,scene,'TrackBarrier',p.x+dx*t-dz/len*(half+1),p.z+dz*t+dx/len*(half+1),1,Math.atan2(dx,dz)+Math.PI/2);}});
  if(config.map==='coast'){const water=mesh(scene,new T.PlaneGeometry(600,600),surfaceMaterial('water',0x187eac,35),0,-.18,0);water.rotation.x=-Math.PI/2; mesh(scene,new T.CylinderGeometry(62,69,.15,64),surfaceMaterial('sand',palette.ground,20),0,-.08,0);}
  dressCircuit(scene,kit,track.route,half,palette.accent,config.map);
  const pickups=track.pickups.map(index=>{const p=track.route[index%track.route.length];return prop(kit,scene,'Pickup',p.x,p.z,1) ?? new T.Group();});
  return {pickups};
}

function sign(title:string,subtitle:string,color:number):T.Mesh {
 const canvas=document.createElement('canvas');canvas.width=512;canvas.height=256;
 const ctx=canvas.getContext('2d')!;ctx.fillStyle='#101e2a';ctx.fillRect(0,0,512,256);
 ctx.fillStyle=`#${color.toString(16).padStart(6,'0')}`;ctx.fillRect(0,0,12,256);
 ctx.fillStyle='#eef5ef';ctx.font='bold 115px system-ui';ctx.fillText(title,32,145);
 ctx.fillStyle='#a5bac5';ctx.font='22px system-ui';ctx.fillText(subtitle,35,205);
 const texture=new T.CanvasTexture(canvas);texture.colorSpace=T.SRGBColorSpace;texture.anisotropy=4;
 return new T.Mesh(new T.PlaneGeometry(2,1),new T.MeshBasicMaterial({map:texture,side:T.DoubleSide,toneMapped:false}));
}

/** Trackside architecture, markings and distant terrain establish a place and scale. */
function dressCircuit(scene:T.Scene,kit:T.Object3D,route:Vec3[],half:number,color:number,map:string):void {
 const white=material(0xe5e4d4),dark=material(0x17232f,.35),paint=material(color,.15),lamp=new T.MeshBasicMaterial({color});
 // Segmented curb paint and road dashes follow the exact simulation polyline.
 route.forEach((p,i)=>{
  const q=route[(i+1)%route.length],dx=q.x-p.x,dz=q.z-p.z,length=Math.hypot(dx,dz),yaw=Math.atan2(dx,dz);
  for(let d=2;d<length-2;d+=3){
   const x=p.x+dx*d/length,z=p.z+dz*d/length;
   for(const side of [-1,1]){
    const curb=mesh(scene,new T.BoxGeometry(.5,.035,1.35),Math.floor(d/3)%2?white:paint,x+dz/length*(half+.3)*side,.06,z-dx/length*(half+.3)*side);curb.rotation.y=yaw;curb.castShadow=false;
   }
   if((d-2)%6===0){const dash=mesh(scene,new T.PlaneGeometry(.12,1.2),white,x,.053,z);dash.rotation.set(-Math.PI/2,0,yaw);dash.castShadow=false;}
  }
  const next=route[(i+2)%route.length],turn=Math.sign(dx*(next.z-q.z)-dz*(next.x-q.x));
  if(i%2===0){
   const x=q.x+dz/length*(half+2)*turn,z=q.z-dx/length*(half+2)*turn;
   const board=sign(turn>0?'‹‹':'››',map==='neon'?'OVERNIGHT CIRCUIT':'CIRCUIT CLASH',color);board.position.set(x,2,z);board.rotation.y=yaw+Math.PI;board.scale.setScalar(1.6);scene.add(board);
   mesh(scene,new T.CylinderGeometry(.07,.09,2,6),dark,x,1,z);
  }
  if(map==='neon'||map==='forest'){
   const x=p.x+dz/length*(half+2),z=p.z-dx/length*(half+2);
   mesh(scene,new T.CylinderGeometry(.07,.14,6,7),dark,x,3,z);
   mesh(scene,new T.BoxGeometry(2.4,.15,.45),lamp,x,6,z);
   if(map==='neon'&&i%3===0){const light=new T.PointLight(color,8,16,2);light.position.set(x,5,z);scene.add(light);}
  }
 });
 const start=route[0],next=route[1],angle=Math.atan2(next.x-start.x,next.z-start.z);
 const board=sign('CLASH','START / FINISH',color);board.position.set(start.x,4.65,start.z);board.rotation.y=angle+Math.PI;board.scale.set(2.2,1,1);scene.add(board);
 // A low spectator terrace and colored canopy at the start give the circuit a focal point.
 const stand=new T.Group();stand.position.set(start.x+Math.cos(angle)*14,0,start.z-Math.sin(angle)*14);stand.rotation.y=angle;scene.add(stand);
 for(let row=0;row<4;row++)mesh(stand,new T.BoxGeometry(8,.3,1),row%2?paint:dark,0,.3+row*.38,row*.9);
 for(const x of [-4,4])for(const z of [-1,4])mesh(stand,new T.CylinderGeometry(.09,.09,4,6),dark,x,2,z);
 const canopy=mesh(stand,new T.BoxGeometry(9,.25,6),paint,0,4,1.4);canopy.rotation.z=.08;
 // Mountains/islands stay outside the raceable area; nearby grass remains flat.
 for(let i=0;i<22;i++){
  const a=i/22*Math.PI*2,r=(map==='coast'?170:115)+(i%3)*12,x=Math.sin(a)*r,z=Math.cos(a)*r;
  if(map==='coast'&&i%3!==0)continue;
  if(map==='neon'){
   prop(kit,scene,'NeonTower',x,z,1.8+(i%4)*.65,a);
  }else{
   const mountain=new T.Mesh(new T.IcosahedronGeometry(1,2),material(map==='coast'?0x4c786a:0x405d52));mountain.position.set(x,2,z);mountain.scale.set(18+(i%3)*4,map==='coast'?4+(i%3)*2:7+(i%4)*3,17);mountain.rotation.y=a;mountain.receiveShadow=true;scene.add(mountain);
  }
 }
 if(map==='forest'){
  // Grass clumps flank the road without obscuring its playable edges.
  const grass=material(0x496940);
  for(let i=0;i<150;i++){
   const a=i*2.399,r=15+(i*17%60),x=Math.sin(a)*r,z=Math.cos(a)*r;
   if(distanceToRoute(x,z,route)<half+1.3)continue;
   const tuft=mesh(scene,new T.ConeGeometry(.6,.8,5),grass,x,.3,z);tuft.rotation.y=a;
  }
 }
}
