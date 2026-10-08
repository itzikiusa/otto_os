import * as T from 'three';
import type {GameEvent,GameState} from './types';
interface Particle {p:T.Vector3;v:T.Vector3;life:number;max:number;size:number;color:T.Color}
/** One bounded instanced particle batch, shared by sparks, dust, bubbles and exhaust. */
export class GameEffects {
 private particles:Particle[]=[];
 private readonly geometry=new T.SphereGeometry(1,8,6);
 private readonly material=new T.MeshBasicMaterial({transparent:true,opacity:.65,depthWrite:false});
 private readonly mesh=new T.InstancedMesh(this.geometry,this.material,260);
 private readonly dummy=new T.Object3D();private readonly fade=new T.Color();
 private lines:{line:T.Line;life:number}[]=[];
 private shields:T.Mesh[]=[];private seekers:T.Mesh[]=[];
 private scene:T.Scene;private trail=0;
 constructor(scene:T.Scene){
  this.scene=scene;this.mesh.frustumCulled=false;this.mesh.count=0;scene.add(this.mesh);
  for(let i=0;i<2;i++){const m=new T.Mesh(new T.SphereGeometry(1.6,20,12),new T.MeshBasicMaterial({color:0x75ddff,transparent:true,opacity:.14,wireframe:true,depthWrite:false}));m.visible=false;scene.add(m);this.shields.push(m);}
  for(let i=0;i<8;i++){const m=new T.Mesh(new T.ConeGeometry(.2,.8,8),new T.MeshStandardMaterial({color:0xffaa44,emissive:0xff6600,emissiveIntensity:2}));m.visible=false;scene.add(m);this.seekers.push(m);}
 }
 private spray(x:number,y:number,z:number,color:number,count:number,power:number,size=.1):void{
  for(let i=0;i<count&&this.particles.length<260;i++){const life=.3+Math.random()*.5;this.particles.push({p:new T.Vector3(x,y,z),v:new T.Vector3((Math.random()-.5)*power,Math.random()*power,(Math.random()-.5)*power),life,max:life,size,color:new T.Color(color)});}
 }
 event(e:GameEvent):void{
  if(e.type==='shot'&&e.end){
   const line=new T.Line(new T.BufferGeometry().setFromPoints([new T.Vector3(e.x,e.y+1.45,e.z),new T.Vector3(e.end.x,e.end.y,e.end.z)]),new T.LineBasicMaterial({color:e.player===0?0x92ecff:0xffb269,transparent:true,opacity:.9}));
   this.scene.add(line);this.lines.push({line,life:.085});this.spray(e.end.x,e.end.y,e.end.z,0xffd69d,5,3,.045);
  }
  if(e.type==='hit'||e.type==='kill')this.spray(e.x,e.y+1,e.z,e.type==='kill'?0x72d9ff:0xffbb63,e.type==='kill'?28:12,5);
  if(e.type==='land')this.spray(e.x,e.y+.1,e.z,0xd2be9f,16,3,.13);
  if(e.type==='trick'||e.type==='pickup'||e.type==='shield')this.spray(e.x,e.y+1,e.z,0xffe382,24,4,.1);
 }
 update(state:GameState,dt:number):void{
  this.trail+=dt;
  if(this.trail>.035){this.trail=0;for(const p of state.players){
   if(state.config.kind==='kart'&&Math.abs(p.speed)>3){
    const x=p.x-Math.sin(p.yaw),z=p.z-Math.cos(p.yaw);
    if(p.underwater)this.spray(x,p.y+.7,z,0xa7efff,2,1,.065);
    else if(p.boost>0)this.spray(x,p.y+.4,z,0x79dfff,3,1,.09);
    else if(p.drifting)this.spray(x,p.y+.1,z,p.driftCharge>.8?0xffbd46:0x83d7ff,3,2,.07);
    else if(p.offTrack)this.spray(x,p.y+.15,z,0xb5a58b,1,.7,.12);
   }
  }}
  for(let i=this.particles.length-1;i>=0;i--){const p=this.particles[i];p.life-=dt;if(p.life<=0){this.particles.splice(i,1);continue;}p.p.addScaledVector(p.v,dt);p.v.y-=dt*2;}
  this.mesh.count=this.particles.length;this.particles.forEach((p,i)=>{this.dummy.position.copy(p.p);this.dummy.scale.setScalar(p.size*(.3+p.life/p.max));this.dummy.updateMatrix();this.mesh.setMatrixAt(i,this.dummy.matrix);this.fade.copy(p.color).multiplyScalar(.4+.6*p.life/p.max);this.mesh.setColorAt(i,this.fade);});
  this.mesh.instanceMatrix.needsUpdate=true;if(this.mesh.instanceColor)this.mesh.instanceColor.needsUpdate=true;
  for(let i=this.lines.length-1;i>=0;i--){const fx=this.lines[i];fx.life-=dt;if(fx.life<=0){this.scene.remove(fx.line);fx.line.geometry.dispose();(fx.line.material as T.Material).dispose();this.lines.splice(i,1);}}
  this.shields.forEach((m,i)=>{const p=state.players[i];m.visible=p.shield>0&&p.hp>0;m.position.set(p.x,p.y+.9,p.z);m.rotation.y+=dt*.4;m.scale.setScalar(state.config.kind==='kart'?1: .75);});
  this.seekers.forEach((m,i)=>{const p=state.projectiles[i];m.visible=!!p;if(p){m.position.set(p.x,p.y,p.z);const target=state.players[p.target];m.quaternion.setFromUnitVectors(new T.Vector3(0,1,0),new T.Vector3(target.x-p.x,target.y+.7-p.y,target.z-p.z).normalize());}});
 }
 dispose():void{for(const fx of this.lines){this.scene.remove(fx.line);fx.line.geometry.dispose();(fx.line.material as T.Material).dispose();}this.lines=[];this.particles=[];}
}
