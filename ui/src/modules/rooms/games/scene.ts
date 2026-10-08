import * as T from 'three';
import { GLTFLoader, type GLTF } from 'three/examples/jsm/loaders/GLTFLoader.js';
import {EffectComposer} from 'three/examples/jsm/postprocessing/EffectComposer.js';
import {RenderPass} from 'three/examples/jsm/postprocessing/RenderPass.js';
import {UnrealBloomPass} from 'three/examples/jsm/postprocessing/UnrealBloomPass.js';
import {OutputPass} from 'three/examples/jsm/postprocessing/OutputPass.js';
import {GameEffects} from './game-effects';
import {DRIVERS,driverFor} from './roster';
import {sampleTrack} from './track-course';
import {trackFor} from './maps';
import { RoomEnvironment } from 'three/examples/jsm/environments/RoomEnvironment.js';
import { followVehicle,vehicleWheels } from './presentation';
import { batchScenery } from './static-batch';
import { buildEnvironment } from './environment';
import { arenaFor } from './maps';
import { segmentBox } from './shooter';
import type {GameConfig, GameState} from './types';

export interface GameScene {
  canvas:HTMLCanvasElement;
  render(state:GameState,player:number,firstPerson:boolean,dt:number,aiming:boolean):void;
  dispose():void;
  reticle():{x:number;y:number};
  stats():{frames:number;drawCalls:number;triangles:number;frameMs:number;camera:{x:number;y:number;z:number};cameraDirection:{x:number;y:number;z:number}};
}
interface Actor { object:T.Object3D; mixer:T.AnimationMixer; clips:Map<string,T.AnimationAction>; current:string; wheels:T.Object3D[];drivers:Map<string,T.Object3D>;head:T.Object3D|null }
function disposeTree(root:T.Object3D):void {
  const geometries=new Set<T.BufferGeometry>(),materials=new Set<T.Material>(),textures=new Set<T.Texture>();
  root.traverse(o=>{if(o instanceof T.Mesh||o instanceof T.Line||o instanceof T.Points){geometries.add(o.geometry);for(const m of Array.isArray(o.material)?o.material:[o.material]){materials.add(m);for(const v of Object.values(m))if(v instanceof T.Texture)textures.add(v);}}});
  for(const g of geometries)g.dispose();for(const m of materials)m.dispose();for(const t of textures)t.dispose();
}
/** Loaded only when a person opens a match, with abort-safe teardown. */
export async function createScene(container:HTMLElement,config:GameConfig,skin:'azure'|'ember',signal:AbortSignal):Promise<GameScene> {
  const loader=new GLTFLoader(),prefix='/room-games/';
  const files=['environment-kit.glb',`${config.kind==='shooter'?'fighter':'kart'}-${skin}.glb`,`${config.kind==='shooter'?'fighter':'kart'}-${skin==='azure'?'ember':'azure'}.glb`,...(config.kind==='shooter'?['rifle.glb']:['drivers.glb']),'adventure-kit.glb'];
  const loaded:GLTF[]=[];
  try {for(const file of files){if(signal.aborted)throw new Error('Game loading canceled');loaded.push(await loader.loadAsync(prefix+file));}} catch(e){for(const asset of loaded)disposeTree(asset.scene);throw new Error(`Game models could not load. Retry to download them again. ${e instanceof Error?e.message:''}`);}
  if(signal.aborted){for(const a of loaded)disposeTree(a.scene);throw new Error('Game loading canceled');}
  const scene=new T.Scene(),camera=new T.PerspectiveCamera(65,1,.08,450);
  let renderer:T.WebGLRenderer;
  try {renderer=new T.WebGLRenderer({antialias:true,powerPreference:'high-performance'});}catch(e){for(const a of loaded)disposeTree(a.scene);throw new Error(`3D graphics are unavailable. Enable hardware acceleration and reopen the game. ${String(e)}`);}
  renderer.setPixelRatio(Math.min(window.devicePixelRatio,1.5));renderer.shadowMap.enabled=true;renderer.shadowMap.type=T.PCFSoftShadowMap;renderer.toneMapping=T.ACESFilmicToneMapping;renderer.toneMappingExposure=.95;
  const pmrem=new T.PMREMGenerator(renderer),room=new RoomEnvironment(),environment=pmrem.fromScene(room,.04);scene.environment=environment.texture;scene.environmentIntensity=config.map==='neon'?.3:config.map==='station'?.45:.6;room.dispose();pmrem.dispose();
  loaded[0].scene.add(loaded[files.indexOf('adventure-kit.glb')].scene);
  const world=buildEnvironment(scene,loaded[0].scene,config);
  batchScenery(scene,[...world.pickups,...world.animated]);
  const driverLibrary=config.kind==='kart'?loaded[files.indexOf('drivers.glb')].scene:null;
  const actors:Actor[]=loaded.slice(1,3).map(gltf=>{
    const object=gltf.scene;if(config.kind==='kart')object.rotation.order='YXZ';object.traverse(o=>{if(o instanceof T.Mesh){o.castShadow=true;o.receiveShadow=true;}});scene.add(object);
    const mixer=new T.AnimationMixer(object),clips=new Map(gltf.animations.map(c=>[c.name,mixer.clipAction(c)]));
    const drivers=new Map<string,T.Object3D>();
    if(driverLibrary){const socket=object.getObjectByName('DriverSocket')??object;for(const d of DRIVERS){const source=driverLibrary.getObjectByName(d.root);if(source){const clone=source.clone(true);clone.traverse(o=>{if(o instanceof T.Mesh){o.castShadow=true;o.receiveShadow=true;}});clone.position.set(0,0,0);socket.add(clone);clone.visible=false;drivers.set(d.id,clone);}}}
    return {object,mixer,clips,current:'',wheels:vehicleWheels(object),drivers,head:object.getObjectByName('Head')??null};
  });
  scene.add(camera);
  const weapon=config.kind==='shooter'?loaded[files.indexOf('rifle.glb')].scene:undefined;
  if(weapon){camera.add(weapon);weapon.scale.setScalar(.65);weapon.position.set(.23,-.28,-.62);weapon.rotation.y=Math.PI;weapon.traverse(o=>{if(o instanceof T.Mesh){o.castShadow=false;o.receiveShadow=false;}});}
  const composer=new EffectComposer(renderer);composer.addPass(new RenderPass(scene,camera));const bloom=new UnrealBloomPass(new T.Vector2(1,1),.22,.45,1.05);composer.addPass(bloom);const output=new OutputPass();composer.addPass(output);renderer.info.autoReset=false;
  const fx=new GameEffects(scene);
  const muzzle=new T.PointLight(0x9fefff,0,4);scene.add(muzzle);let recoil=0;
  const canvas=renderer.domElement;canvas.tabIndex=0;canvas.setAttribute('aria-label',config.kind==='shooter'?'Arena Duel game. Click to aim, WASD to move.':'Circuit Clash game. Click then use WASD to drive.');container.appendChild(canvas);
  const resize=()=>{const w=Math.max(1,container.clientWidth),h=Math.max(1,container.clientHeight);renderer.setSize(w,h);composer.setSize(w,h);camera.aspect=w/h;camera.updateProjectionMatrix();};
  const observer=new ResizeObserver(resize);observer.observe(container);resize();
  let disposed=false,lastEvent=0,frames=0,frameMs=0;
  const eye=new T.Vector3(),desired=new T.Vector3(),target=new T.Vector3();let positioned=false,actorsPositioned=false;const cameraAim=new T.Vector3();let reticle={x:50,y:50};
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
        if(config.kind==='kart'){
          actor.drivers.forEach((model,key)=>{model.visible=key===p.character;if(model.visible){const head=model.getObjectByName(`${driverFor(key).species==='Robot'?'Robot':driverFor(key).species}Head`);if(head)head.rotation.y=T.MathUtils.lerp(head.rotation.y,-p.steering*.3,1-Math.exp(-dt*6));}});
          const surface=sampleTrack(trackFor(config.map),p.x,p.z);
          const slope=p.grounded?-Math.atan(surface.slope*(surface.tangent.x*Math.sin(p.yaw)+surface.tangent.z*Math.cos(p.yaw))):Math.min(.2,-p.velocityY*.025);
          actor.object.rotation.x=T.MathUtils.lerp(actor.object.rotation.x,slope,1-Math.exp(-dt*8));
          actor.object.rotation.z=T.MathUtils.lerp(actor.object.rotation.z,-p.steering*Math.min(.07,Math.abs(p.speed)*.003),1-Math.exp(-dt*9));for(const wheel of actor.wheels.slice(0,2))wheel.rotation.y=-p.steering*.32;if(p.trick&&!p.grounded)actor.object.rotation.z=Math.sin(Math.min(1,p.airTime/.8)*Math.PI*2)*.35;}
      });
      actorsPositioned=true;
      const statePlayer=state.players[player], visual=actors[player].object;
      const p=config.kind==='kart'?{...statePlayer,x:visual.position.x,y:visual.position.y,z:visual.position.z,yaw:visual.rotation.y}:statePlayer;const forward=new T.Vector3(Math.sin(p.yaw)*Math.cos(p.pitch),Math.sin(p.pitch),Math.cos(p.yaw)*Math.cos(p.pitch));
      eye.set(p.x,p.y+(config.kind==='shooter'?1.55:1.05),p.z);
      if(config.kind==='shooter'&&firstPerson&&p.hp>0){camera.position.copy(eye);target.copy(eye).addScaledVector(forward,20);camera.lookAt(target);positioned=true;}
      else {
        const yaw=p.yaw;const follow=config.kind==='shooter'?(aiming?2.8:4.6):6.5+(p.boost>0?.65:0);
        desired.set(eye.x-Math.sin(yaw)*follow+(config.kind==='shooter'?Math.cos(yaw)*.65:0),eye.y+(config.kind==='shooter'?1.05:p.underwater?2:2.4),eye.z-Math.cos(yaw)*follow-(config.kind==='shooter'?Math.sin(yaw)*.65:0));
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
      const fov=config.kind==='shooter'?(aiming?48:68):64+Math.min(7,Math.abs(p.speed)*.18)+(p.boost>0?5:0);camera.fov=T.MathUtils.lerp(camera.fov,fov,1-Math.exp(-dt*5));camera.updateProjectionMatrix();
      if(weapon){weapon.visible=firstPerson&&config.kind==='shooter'&&p.hp>0;weapon.position.z=-.62+recoil*2;weapon.position.x=aiming?.1:.23;weapon.position.y=-.28+(p.moving?Math.sin(state.elapsed*10)*.012:0);weapon.rotation.z=T.MathUtils.lerp(weapon.rotation.z,p.reloadTime>0?-.55:0,1-Math.exp(-dt*12));weapon.scale.set(.65,p.weapon==='scatter'?.78:.65,p.weapon==='rail'?.85:.65);}
      for(const e of state.events)if(e.id>lastEvent){fx.event(e);if(e.type==='shot'&&e.player===player){recoil=p.weapon==='scatter'?.09:p.weapon==='rail'?.065:.035;muzzle.position.set(p.x+forward.x*.7,p.y+1.4,p.z+forward.z*.7);muzzle.intensity=5;}lastEvent=e.id;}
      recoil*=Math.exp(-dt*18);muzzle.intensity*=Math.exp(-dt*28);
      if(config.kind==='shooter'&&p.hp>0)camera.rotateX(recoil);
      if(config.kind==='shooter'){
        const end=eye.clone().addScaledVector(forward,90);let fraction=1;
        for(const box of arenaFor(config.map).cover){const hit=segmentBox(eye,end,box);if(hit!==null)fraction=Math.min(fraction,hit);}
        const enemy=state.players[1-player];if(enemy.hp>0){const hit=segmentBox(eye,end,{x:enemy.x,y:enemy.y,z:enemy.z,width:.9,height:1.85,depth:.9});if(hit!==null)fraction=Math.min(fraction,hit);}
        camera.updateMatrixWorld();const screen=eye.clone().lerp(end,fraction).project(camera);reticle={x:(screen.x+1)*50,y:(1-screen.y)*50};
      }
      fx.update(state,dt);world.update?.(state,dt,player,camera.position.y);
      world.pickups.forEach((o,i)=>{o.visible=(state.pickupTimers[i]??0)<=0;o.rotation.y+=dt;o.position.y=(o.userData.baseY??.7)+Math.sin(state.elapsed*2+i)*.15;});
      renderer.info.reset();composer.render();frameMs=frameMs*.95+(performance.now()-start)*.05;
    },
    reticle:()=>reticle,
    stats:()=>({frames,drawCalls:renderer.info.render.calls,triangles:renderer.info.render.triangles,frameMs,camera:{x:camera.position.x,y:camera.position.y,z:camera.position.z},cameraDirection:camera.getWorldDirection(new T.Vector3())}),
    dispose(){if(disposed)return;disposed=true;observer.disconnect();fx.dispose();bloom.dispose();output.dispose();composer.dispose();actors.forEach(a=>{a.mixer.stopAllAction();a.mixer.uncacheRoot(a.object);});disposeTree(scene);disposeTree(loaded[0].scene);environment.dispose();renderer.dispose();renderer.forceContextLoss();canvas.remove();},
  };
}
