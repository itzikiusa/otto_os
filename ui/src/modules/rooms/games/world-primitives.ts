import * as T from 'three';
import type {Vec3} from './types';
import {roadStripVertices} from './track-course.ts';

export function material(color: number, metalness = 0, roughness = .8): T.MeshStandardMaterial { return new T.MeshStandardMaterial({color, metalness, roughness}); }
export function mesh(parent:T.Object3D, geometry:T.BufferGeometry, mat:T.Material, x:number,y:number,z:number):T.Mesh {
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
  const n = route.length,vertices=roadStripVertices(route,left,right);let travelled=0;
  for (let i=0;i<=n;i++) {
    const p=route[i%n], prev=route[(i+n-1)%n];
    if(i>0)travelled+=Math.hypot(p.x-prev.x,p.z-prev.z);
    for (let side=0;side<2;side++) { const vertex=vertices[i*2+side];positions.push(vertex.x,vertex.y+y,vertex.z);uv.push(side,travelled/8); }
    if(i<n) {const k=i*2; indices.push(k,k+1,k+2,k+1,k+3,k+2);}
  }
  const geometry=new T.BufferGeometry(); geometry.setAttribute('position',new T.Float32BufferAttribute(positions,3)); geometry.setAttribute('uv',new T.Float32BufferAttribute(uv,2)); geometry.setIndex(indices); geometry.computeVertexNormals(); return geometry;
}
export function distanceToRoute(x:number,z:number, route:Vec3[]):number {
  let best=Infinity;
  route.forEach((p,i)=> {const q=route[(i+1)%route.length],dx=q.x-p.x,dz=q.z-p.z,t=Math.max(0,Math.min(1,((x-p.x)*dx+(z-p.z)*dz)/(dx*dx+dz*dz))); best=Math.min(best,Math.hypot(x-p.x-dx*t,z-p.z-dz*t));}); return best;
}
export function sign(title:string,subtitle:string,color:number):T.Mesh {
 const canvas=document.createElement('canvas');canvas.width=512;canvas.height=256;
 const ctx=canvas.getContext('2d')!;ctx.fillStyle='#101e2a';ctx.fillRect(0,0,512,256);
 ctx.fillStyle=`#${color.toString(16).padStart(6,'0')}`;ctx.fillRect(0,0,12,256);
 ctx.fillStyle='#eef5ef';ctx.font='bold 115px system-ui';ctx.fillText(title,32,145);
 ctx.fillStyle='#a5bac5';ctx.font='22px system-ui';ctx.fillText(subtitle,35,205);
 const texture=new T.CanvasTexture(canvas);texture.colorSpace=T.SRGBColorSpace;texture.anisotropy=4;
 return new T.Mesh(new T.PlaneGeometry(2,1),new T.MeshBasicMaterial({map:texture,side:T.DoubleSide,toneMapped:false}));
}
/** Keep a grounded decoration's entire world AABB outside the square play area. */
export function placeBeyondArena(object:T.Object3D,halfSize:number,clearance=1):void {
 const bounds=new T.Box3().setFromObject(object);
 if(Math.abs(object.position.x)>=Math.abs(object.position.z)){
  const side=object.position.x<0?-1:1;
  object.position.x+=side>0?Math.max(0,halfSize+clearance-bounds.min.x):Math.min(0,-halfSize-clearance-bounds.max.x);
 }else{
  const side=object.position.z<0?-1:1;
  object.position.z+=side>0?Math.max(0,halfSize+clearance-bounds.min.z):Math.min(0,-halfSize-clearance-bounds.max.z);
 }
}
