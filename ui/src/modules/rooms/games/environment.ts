import * as T from 'three';
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
  const n = route.length;
  for (let i=0;i<=n;i++) {
    const p=route[i%n], prev=route[(i+n-1)%n], next=route[(i+1)%n];
    const a=new T.Vector2(p.x-prev.x,p.z-prev.z).normalize(), b=new T.Vector2(next.x-p.x,next.z-p.z).normalize();
    const normal = new T.Vector2(-(a.y+b.y),a.x+b.x).normalize();
    const correction = 1 / Math.max(.45, normal.dot(new T.Vector2(-b.y,b.x)));
    for (const offset of [left,right]) { positions.push(p.x+normal.x*offset*correction,y,p.z+normal.y*offset*correction); uv.push(offset, i); }
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
  if(config.map!=='coast')mesh(scene,new T.PlaneGeometry(600,600),material(palette.ground),0,-.04,0).rotation.x=-Math.PI/2;
  const hemi=new T.HemisphereLight(palette.sky,0x62514e,2.0); scene.add(hemi);
  const sun=new T.DirectionalLight(palette.sun,3.2); sun.position.set(-30,55,20); sun.castShadow=true;
  sun.shadow.mapSize.set(2048,2048); sun.shadow.camera.left=-65; sun.shadow.camera.right=65; sun.shadow.camera.top=65; sun.shadow.camera.bottom=-65; sun.shadow.camera.far=150; sun.shadow.normalBias=.06; scene.add(sun);
  if(config.kind==='shooter') {
    const arena=arenaFor(config.map), n=arena.halfSize;
    if(config.map!=='dunes') {
      const grid=new T.GridHelper(n*2,Math.round(n*2/3),palette.accent,palette.ground); grid.position.y=.015; scene.add(grid);
      for(let i=-n;i<n;i+=6) for(const side of [-1,1]) {
        prop(kit,scene,'StationWall',i,side*n,1,side<0?0:Math.PI,{width:6,height:4,depth:.8});
        prop(kit,scene,'StationWall',side*n,i,1,Math.PI/2,{width:6,height:4,depth:.8});
      }
    } else {
      for(let i=0;i<28;i++) {const a=i/28*Math.PI*2; prop(kit,scene,'DesertRock',Math.sin(a)*(n+4),Math.cos(a)*(n+4),2+(i%3),a);}
    }
    arena.cover.forEach((cover,i)=> {
      // Collision surfaces stay rectangular even when decorative art has gaps.
      const backing=mesh(scene,new T.BoxGeometry(cover.width,cover.height,cover.depth),material(config.map==='dunes'?0xb98c57:0x475364,.25),cover.x,cover.y+cover.height/2,cover.z);
      backing.name='CollisionCover';
      const name=config.map==='dunes'?'DesertRock':config.map==='foundry'?'FoundryFurnace':i%2?'CargoCrate':'Reactor';
      prop(kit,scene,name,cover.x,cover.z,1,0,{width:cover.width+.04,height:cover.height+.06,depth:cover.depth+.04});
      if(config.map!=='dunes') {const glow=mesh(scene,new T.BoxGeometry(cover.width*.8,.06,.05),new T.MeshBasicMaterial({color:palette.accent}),cover.x,cover.height*.7,cover.z+cover.depth/2+.04);glow.castShadow=false;}
    });
    arena.spawns.forEach((p,i)=> {const ring=mesh(scene,new T.RingGeometry(1.1,1.35,40),new T.MeshBasicMaterial({color:i%2?0xff9767:0x66d8ef,side:T.DoubleSide}),p.x,.025,p.z);ring.rotation.x=-Math.PI/2;});
    for(let i=0;i<8;i++){const a=i/8*Math.PI*2;prop(kit,scene,config.map==='dunes'?'DesertArch':'StationColumn',Math.sin(a)*(n+1),Math.cos(a)*(n+1),1.4,a);}
    if(config.map==='station') {
      const stars=new T.BufferGeometry(),positions=[];for(let i=0;i<180;i++){const a=i*2.399;positions.push(Math.sin(a)*130,25+(i*17%100),Math.cos(a)*130);}stars.setAttribute('position',new T.Float32BufferAttribute(positions,3));scene.add(new T.Points(stars,new T.PointsMaterial({color:0xc5e5ff,size:.35})));
      mesh(scene,new T.SphereGeometry(18,32,20),material(0x6b93bc,.05,.75),70,45,-110);
    }
    return {pickups:[]};
  }
  const track=trackFor(config.map), half=track.width/2;
  const road=mesh(scene,roadStrip(track.route,-half,half,.04),material(palette.road,.08,.8),0,0,0); road.castShadow=false;
  for(const side of [-1,1]) {
    mesh(scene,roadStrip(track.route,side*(half+.05),side*(half+.6),.045),material(palette.accent),0,0,0);
    mesh(scene,roadStrip(track.route,side*(half-.35),side*(half-.2),.05),material(0xf2eee3),0,0,0);
  }
  const start=track.route[0],next=track.route[1],yaw=Math.atan2(next.x-start.x,next.z-start.z);
  const gantry=prop(kit,scene,'RaceGantry',start.x,start.z,1,yaw); if(gantry) gantry.scale.multiplyScalar(1.3);
  for(let i=0;i<10;i++)for(let j=0;j<2;j++) {const x=(i-4.5)*track.width/10,z=(j-.5)*.8;const check=mesh(scene,new T.PlaneGeometry(track.width/10,.8),material((i+j)%2?0xeff2ea:0x273141),start.x+Math.cos(yaw)*x+Math.sin(yaw)*z,.065,start.z-Math.sin(yaw)*x+Math.cos(yaw)*z);check.rotation.set(-Math.PI/2,0,yaw);}
  for(let i=0;i<90;i++) {
    const a=i*2.399,r=18+((i*31)%70),x=Math.sin(a)*r,z=Math.cos(a)*r;
    if(distanceToRoute(x,z,track.route)<half+3) continue;
    const name=config.map==='forest'?(i%3?'PineTree':'BroadleafTree'):config.map==='coast'?(i%3?'PalmTree':'CoastalRock'):'NeonTower';
    prop(kit,scene,name,x,z,.8+(i%4)*.35,a);
  }
  track.route.forEach((p,i)=>{const q=track.route[(i+1)%track.route.length],dx=q.x-p.x,dz=q.z-p.z,len=Math.hypot(dx,dz);for(let j=0;j<Math.floor(len/7);j++){const t=(j+.5)/Math.floor(len/7);prop(kit,scene,'TrackBarrier',p.x+dx*t-dz/len*(half+1),p.z+dz*t+dx/len*(half+1),1,Math.atan2(dx,dz));}});
  if(config.map==='coast'){const water=mesh(scene,new T.PlaneGeometry(600,600),material(0x369cac,.5,.22),0,-.18,0);water.rotation.x=-Math.PI/2; mesh(scene,new T.CylinderGeometry(75,82,.15,64),material(palette.ground),0,-.08,0);}
  const pickups=track.pickups.map(index=>{const p=track.route[index%track.route.length];return prop(kit,scene,'Pickup',p.x,p.z,1) ?? new T.Group();});
  return {pickups};
}
