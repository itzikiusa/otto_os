import * as T from 'three';
import { GLTFLoader, type GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';
import { followVehicle,vehicleWheels } from './presentation';
import { batchScenery } from './static-batch';
import { buildEnvironment } from './environment';
import { arenaFor } from './maps';
import { segmentBox } from './shooter';
import type {GameConfig, GameEvent, GameState} from './types';

export interface GameScene {
  canvas:HTMLCanvasElement;
  render(state:GameState,player:number,firstPerson:boolean,dt:number,aiming:boolean):void;
  dispose():void;
  reticle():{x:number;y:number};
  stats():{frames:number;drawCalls:number;triangles:number;frameMs:number;camera:{x:number;y:number;z:number};cameraDirection:{x:number;y:number;z:number}};
}
interface Actor { object:T.Object3D; mixer:T.AnimationMixer; clips:Map<string,T.AnimationAction>; current:string; wheels:T.Object3D[] }
function disposeTree(root:T.Object3D):void {
  const geometries=new Set<T.BufferGeometry>(),materials=new Set<T.Material>(),textures=new Set<T.Texture>();
  root.traverse(o=>{if(o instanceof T.Mesh||o instanceof T.Line||o instanceof T.Points){geometries.add(o.geometry);for(const m of Array.isArray(o.material)?o.material:[o.material]){materials.add(m);for(const v of Object.values(m))if(v instanceof T.Texture)textures.add(v);}}});
  for(const g of geometries)g.dispose();for(const m of materials)m.dispose();for(const t of textures)t.dispose();
}
/** Loaded only when a person opens a match, with abort-safe teardown. */
export async function createScene(container:HTMLElement,config:GameConfig,skin:'azure'|'ember',signal:AbortSignal):Promise<GameScene> {
  const loader=new GLTFLoader(),prefix='/room-games/';
  const files=['environment-kit.glb',`${config.kind==='shooter'?'fighter':'kart'}-${skin}.glb`,`${config.kind==='shooter'?'fighter':'kart'}-${skin==='azure'?'ember':'azure'}.glb`,...(config.kind==='shooter'?['rifle.glb']:[])];
  const loaded:GLTF[]=[];
  try {for(const file of files){if(signal.aborted)throw new Error('Game loading canceled');loaded.push(await loader.loadAsync(prefix+file));}} catch(e){for(const asset of loaded)disposeTree(asset.scene);throw new Error(`Game models could not load. Retry to download them again. ${e instanceof Error?e.message:''}`);}
  if(signal.aborted){for(const a of loaded)disposeTree(a.scene);throw new Error('Game loading canceled');}
  const scene=new T.Scene(),camera=new T.PerspectiveCamera(65,1,.08,450);
  let renderer:T.WebGLRenderer;
  try {renderer=new T.WebGLRenderer({antialias:true,powerPreference:'high-performance'});}catch(e){for(const a of loaded)disposeTree(a.scene);throw new Error(`3D graphics are unavailable. Enable hardware acceleration and reopen the game. ${String(e)}`);}
  renderer.setPixelRatio(Math.min(window.devicePixelRatio,1.5));renderer.shadowMap.enabled=true;renderer.shadowMap.type=T.PCFShadowMap;renderer.toneMapping=T.ACESFilmicToneMapping;renderer.toneMappingExposure=.95;
  const pmrem=new T.PMREMGenerator(renderer),room=new RoomEnvironment(),environment=pmrem.fromScene(room,.04);scene.environment=environment.texture;scene.environmentIntensity=config.map==='neon'?.3:config.map==='station'?.45:.6;room.dispose();pmrem.dispose();
  const world=buildEnvironment(scene,loaded[0].scene,config);
  batchScenery(scene,world.pickups);
  const actors:Actor[]=loaded.slice(1,3).map(gltf=>{
    const object=gltf.scene;object.traverse(o=>{if(o instanceof T.Mesh){o.castShadow=true;o.receiveShadow=true;}});scene.add(object);
    const mixer=new T.AnimationMixer(object),clips=new Map(gltf.animations.map(c=>[c.name,mixer.clipAction(c)]));
    return {object,mixer,clips,current:'',wheels:vehicleWheels(object)};
  });
  scene.add(camera);
  const weapon=loaded[3]?.scene;
  if(weapon){camera.add(weapon);weapon.scale.setScalar(.65);weapon.position.set(.23,-.28,-.62);weapon.rotation.y=Math.PI;weapon.traverse(o=>{if(o instanceof T.Mesh){o.castShadow=false;o.receiveShadow=false;}});}
  const canvas=renderer.domElement;canvas.tabIndex=0;canvas.setAttribute('aria-label',config.kind==='shooter'?'Arena Duel game. Click to aim, WASD to move.':'Circuit Clash game. Click then use WASD to drive.');container.appendChild(canvas);
  const resize=()=>{const w=Math.max(1,container.clientWidth),h=Math.max(1,container.clientHeight);renderer.setSize(w,h);camera.aspect=w/h;camera.updateProjectionMatrix();};
  const observer=new ResizeObserver(resize);observer.observe(container);resize();
  let disposed=false,lastEvent=0,frames=0,frameMs=0;
  const effects:{object:T.Object3D;life:number}[]=[];
  const eye=new T.Vector3(),desired=new T.Vector3(),target=new T.Vector3();let positioned=false,actorsPositioned=false;const cameraAim=new T.Vector3();let reticle={x:50,y:50};
  function event(e:GameEvent):void {
    if(e.type==='shot'&&e.end){const geometry=new T.BufferGeometry().setFromPoints([new T.Vector3(e.x,e.y+1.45,e.z),new T.Vector3(e.end.x,e.end.y,e.end.z)]);const line=new T.Line(geometry,new T.LineBasicMaterial({color:e.player===0?0xa1efff:0xffb681,transparent:true,opacity:.8}));scene.add(line);effects.push({object:line,life:.09});}
    if(e.type==='hit'||e.type==='kill'||e.type==='boost'){const ring=new T.Mesh(new T.RingGeometry(.2,.65,20),new T.MeshBasicMaterial({color:e.type==='hit'?0xffd0a3:0x7eeeff,transparent:true,opacity:.7,side:T.DoubleSide}));ring.position.set(e.x,e.y+1,e.z);ring.lookAt(camera.position);scene.add(ring);effects.push({object:ring,life:.3});}
  }
  return {canvas,
    render(state,player,firstPerson,dt,aiming){
      if(disposed)return;const start=performance.now();frames++;
      if(actorsPositioned&&config.kind==='kart'&&actors[player].object.position.distanceTo(new T.Vector3(state.players[player].x,state.players[player].y,state.players[player].z))>12)positioned=false;
      state.players.forEach((p,i)=>{
        const actor=actors[i];
        const pose=config.kind==='kart'&&actorsPositioned?followVehicle({...actor.object.position,yaw:actor.object.rotation.y},p,dt):p;
        actor.object.position.set(pose.x,pose.y,pose.z);actor.object.rotation.y=pose.yaw;
        actor.object.visible=!(config.kind==='shooter'&&i===player&&firstPerson&&p.hp>0);
        const clip=p.hp<=0?'Death':p.cooldown>.10?'Shoot':p.moving?'Run':'Idle';
        if(clip!==actor.current){const old=actor.clips.get(actor.current),next=actor.clips.get(clip);old?.fadeOut(.15);next?.reset().fadeIn(.15).play();if(next&&clip==='Death'){next.setLoop(T.LoopOnce,1);next.clampWhenFinished=true;}actor.current=clip;}
        actor.mixer.update(Math.min(dt,.05));for(const wheel of actor.wheels)wheel.rotation.x+=p.speed*dt*2.2;
        if(config.kind==='kart'){actor.object.rotation.z=T.MathUtils.lerp(actor.object.rotation.z,-p.steering*Math.min(.07,Math.abs(p.speed)*.003),1-Math.exp(-dt*9));for(const wheel of actor.wheels.slice(0,2))wheel.rotation.y=-p.steering*.32;}
      });
      actorsPositioned=true;
      const statePlayer=state.players[player], visual=actors[player].object;
      const p=config.kind==='kart'?{...statePlayer,x:visual.position.x,y:visual.position.y,z:visual.position.z,yaw:visual.rotation.y}:statePlayer;const forward=new T.Vector3(Math.sin(p.yaw)*Math.cos(p.pitch),Math.sin(p.pitch),Math.cos(p.yaw)*Math.cos(p.pitch));
      eye.set(p.x,p.y+(config.kind==='shooter'?1.55:1.05),p.z);
      if(config.kind==='shooter'&&firstPerson&&p.hp>0){camera.position.copy(eye);target.copy(eye).addScaledVector(forward,20);camera.lookAt(target);positioned=true;}
      else {
        const yaw=p.yaw;const follow=config.kind==='shooter'?(aiming?2.8:4.6):6.5;
        desired.set(eye.x-Math.sin(yaw)*follow,eye.y+(config.kind==='shooter'?1.05:2.4),eye.z-Math.cos(yaw)*follow);
        if(config.kind==='shooter'){
          let fraction=1;for(const b of arenaFor(config.map).cover){const t=segmentBox(eye,desired,{...b,width:b.width+.35,depth:b.depth+.35,height:b.height+.3});if(t!==null)fraction=Math.min(fraction,Math.max(.08,t-.06));}
          desired.lerpVectors(eye,desired,fraction);
          const n=arenaFor(config.map).halfSize-.35;desired.x=T.MathUtils.clamp(desired.x,-n,n);desired.z=T.MathUtils.clamp(desired.z,-n,n);
        }
        if(!positioned){camera.position.copy(desired);}else camera.position.lerp(desired,1-Math.exp(-dt*10));
        if(config.kind==='shooter'){for(const box of arenaFor(config.map).cover){const hit=segmentBox(eye,camera.position,box,.18);if(hit!==null)camera.position.lerpVectors(eye,camera.position,Math.max(.06,hit-.05));}}
        target.copy(eye).addScaledVector(forward,config.kind==='shooter'?12:5);
        if(!positioned||config.kind==='shooter')cameraAim.copy(target);else cameraAim.lerp(target,1-Math.exp(-dt*10));
        camera.lookAt(cameraAim);positioned=true;
      }
      if(config.kind==='shooter'){
        const end=eye.clone().addScaledVector(forward,90);let fraction=1;
        for(const box of arenaFor(config.map).cover){const hit=segmentBox(eye,end,box);if(hit!==null)fraction=Math.min(fraction,hit);}
        const enemy=state.players[1-player];if(enemy.hp>0){const hit=segmentBox(eye,end,{x:enemy.x,y:enemy.y,z:enemy.z,width:.9,height:1.85,depth:.9});if(hit!==null)fraction=Math.min(fraction,hit);}
        camera.updateMatrixWorld();const screen=eye.clone().lerp(end,fraction).project(camera);reticle={x:(screen.x+1)*50,y:(1-screen.y)*50};
      }
      const fov=config.kind==='shooter'?(aiming?48:68):62+Math.min(6,Math.abs(p.speed)*.18);camera.fov=T.MathUtils.lerp(camera.fov,fov,1-Math.exp(-dt*5));camera.updateProjectionMatrix();
      if(weapon){weapon.visible=firstPerson&&config.kind==='shooter'&&p.hp>0;weapon.position.z=-.62+(p.cooldown>.1?.07:0);weapon.position.y=-.28+(p.moving?Math.sin(state.elapsed*10)*.015:0);weapon.rotation.z=p.reloadTime>0?-.35:0;}
      for(const e of state.events)if(e.id>lastEvent){event(e);lastEvent=e.id;}
      for(let i=effects.length-1;i>=0;i--){const fx=effects[i];fx.life-=dt;if(fx.life<=0){scene.remove(fx.object);disposeTree(fx.object);effects.splice(i,1);}else if(fx.object instanceof T.Mesh)fx.object.scale.multiplyScalar(1+dt*3);}
      world.pickups.forEach((o,i)=>{o.visible=(state.pickupTimers[i]??0)<=0;o.rotation.y+=dt;o.position.y=.7+Math.sin(state.elapsed*2+i)*.15;});
      renderer.render(scene,camera);frameMs=frameMs*.95+(performance.now()-start)*.05;
    },
    reticle:()=>reticle,
    stats:()=>({frames,drawCalls:renderer.info.render.calls,triangles:renderer.info.render.triangles,frameMs,camera:{x:camera.position.x,y:camera.position.y,z:camera.position.z},cameraDirection:camera.getWorldDirection(new T.Vector3())}),
    dispose(){if(disposed)return;disposed=true;observer.disconnect();actors.forEach(a=>{a.mixer.stopAllAction();a.mixer.uncacheRoot(a.object);});disposeTree(scene);disposeTree(loaded[0].scene);environment.dispose();renderer.dispose();renderer.forceContextLoss();canvas.remove();},
  };
}
