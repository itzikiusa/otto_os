import * as T from 'three';
import {RoundedBoxGeometry} from 'three/examples/jsm/geometries/RoundedBoxGeometry.js';
import {surfaceMaterial,skyDome} from './surfaces.ts';
import {arenaFor} from './maps.ts';
import {material,mesh,prop,sign,placeBeyondArena} from './world-primitives.ts';
import {buildCircuitWorld} from './circuit-world.ts';
import {dressArena} from './arena-world.ts';
export {roadStrip,prop,material} from './world-primitives.ts';
import type {GameConfig,GameState} from './types';

export const PALETTES: Record<string, {sky:number; ground:number; road:number; accent:number; sun:number; fog:number}> = {
  station: {sky:0x131e38,ground:0x344356,road:0x3d485e,accent:0x67daee,sun:0xc2dcff,fog:120},
  foundry: {sky:0x352b38,ground:0x484047,road:0x453744,accent:0xff925c,sun:0xffc694,fog:110},
  dunes: {sky:0xbfdce5,ground:0xc89d65,road:0xac824f,accent:0x37c5cb,sun:0xffecd1,fog:170},
  coast: {sky:0x8fd8ee,ground:0xe8ce98,road:0x444f60,accent:0x38d3ca,sun:0xfff1cf,fog:220},
  forest: {sky:0xb9d8cb,ground:0x668b62,road:0x65564c,accent:0xffd26f,sun:0xffedc7,fog:190},
  neon: {sky:0x151e3e,ground:0x202b44,road:0x303b54,accent:0xbb86ff,sun:0xabc4ff,fog:200},
};
export function buildEnvironment(scene:T.Scene, kit:T.Object3D, config:GameConfig):{pickups:T.Object3D[];animated:T.Object3D[];update?:(state:GameState,dt:number,player?:number,cameraY?:number)=>void} {
  const palette=PALETTES[config.map] ?? PALETTES.station;
  scene.background=new T.Color(palette.sky); scene.fog=new T.Fog(palette.sky,palette.fog*.4,palette.fog);
  const night=config.map==='station'||config.map==='neon'||config.map==='foundry';
  skyDome(scene,night?palette.sky:config.map==='dunes'?0x6290b5:0x2671a5,night?0x1b334b:config.map==='coast'?0x8ad3ee:config.map==='forest'?0xa8cdd7:0xe6cead,night);
  if(config.kind==='shooter'){
    const floor=surfaceMaterial(config.map==='dunes'?'sand':'metal',palette.ground,80);
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
      for(let i=0;i<28;i++) {const a=i/28*Math.PI*2,r=(n+4)/Math.max(Math.abs(Math.sin(a)),Math.abs(Math.cos(a)));const rock=prop(kit,scene,'DesertRock',Math.sin(a)*r,Math.cos(a)*r,2+(i%3),a);if(rock)placeBeyondArena(rock,n,2);}
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
    for(let i=0;i<8;i++){const a=i/8*Math.PI*2,r=(n+4)/Math.max(Math.abs(Math.sin(a)),Math.abs(Math.cos(a)));const decoration=prop(kit,scene,config.map==='dunes'?'DesertArch':'StationColumn',Math.sin(a)*r,Math.cos(a)*r,1.4,a);if(decoration)placeBeyondArena(decoration,n,2);}
    if(config.map==='station') {
      const stars=new T.BufferGeometry(),positions=[];for(let i=0;i<180;i++){const a=i*2.399;positions.push(Math.sin(a)*130,25+(i*17%100),Math.cos(a)*130);}stars.setAttribute('position',new T.Float32BufferAttribute(positions,3));scene.add(new T.Points(stars,new T.PointsMaterial({color:0xc5e5ff,size:.35,fog:false})));
      const planetMaterial=material(0x6b93bc,.05,.75);planetMaterial.fog=false;mesh(scene,new T.SphereGeometry(18,32,20),planetMaterial,50,48,100);
    }
    return dressArena(scene,arena,config,palette.accent);
  }
  return buildCircuitWorld(scene,kit,config,palette);
}
