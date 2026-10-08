import type {TrackMap,Vec3} from './types.ts';
/** Closed Catmull-Rom centerline shared by collision, AI and the road mesh. */
export function sampleCourse(points:Vec3[],steps=4):Vec3[] {
 const result:Vec3[]=[];
 for(let i=0;i<points.length;i++)for(let n=0;n<steps;n++){
  const a=points[(i+points.length-1)%points.length],b=points[i],c=points[(i+1)%points.length],d=points[(i+2)%points.length],t=n/steps;
  const at=(k:keyof Vec3)=>.5*((2*b[k])+(-a[k]+c[k])*t+(2*a[k]-5*b[k]+4*c[k]-d[k])*t*t+(-a[k]+3*b[k]-3*c[k]+d[k])*t*t*t);
  result.push({x:at('x'),y:at('y'),z:at('z')});
 }
 return result;
}
/** Paired mitered edges, including the closing pair, shared by mesh and physics. */
export function roadStripVertices(route:Vec3[],left:number,right:number):Vec3[] {
 [left,right]=[Math.min(left,right),Math.max(left,right)];
 const vertices:Vec3[]=[],n=route.length;
 for(let i=0;i<=n;i++){
  const p=route[i%n],prev=route[(i+n-1)%n],next=route[(i+1)%n];
  const before=Math.hypot(p.x-prev.x,p.z-prev.z)||1,after=Math.hypot(next.x-p.x,next.z-p.z)||1;
  const ax=(p.x-prev.x)/before,az=(p.z-prev.z)/before,bx=(next.x-p.x)/after,bz=(next.z-p.z)/after;
  const normalLength=Math.hypot(az+bz,ax+bx)||1,nx=-(az+bz)/normalLength,nz=(ax+bx)/normalLength;
  const correction=1/Math.max(.45,-nx*bz+nz*bx);
  for(const offset of [left,right])vertices.push({x:p.x+nx*offset*correction,y:p.y,z:p.z+nz*offset*correction});
 }
 return vertices;
}
interface RoadTriangle {a:Vec3;b:Vec3;c:Vec3;minX:number;maxX:number;minZ:number;maxZ:number;denominator:number}
const roadTriangles=new WeakMap<Vec3[],Map<number,RoadTriangle[]>>();
/** Height on the actual strip triangles; null means there is no road below this XZ. */
export function sampleRoadHeight(track:TrackMap,x:number,z:number,halfWidth=track.width/2):number|null {
 let widths=roadTriangles.get(track.route);if(!widths){widths=new Map();roadTriangles.set(track.route,widths);}
 let triangles=widths.get(halfWidth);
 if(!triangles){
  triangles=[];const vertices=roadStripVertices(track.route,-halfWidth,halfWidth);
  for(let i=0;i<track.route.length;i++)for(const indices of [[i*2,i*2+1,i*2+2],[i*2+1,i*2+3,i*2+2]]){
   const [a,b,c]=indices.map(index=>vertices[index]);
   triangles.push({a,b,c,minX:Math.min(a.x,b.x,c.x),maxX:Math.max(a.x,b.x,c.x),minZ:Math.min(a.z,b.z,c.z),maxZ:Math.max(a.z,b.z,c.z),denominator:(b.z-c.z)*(a.x-c.x)+(c.x-b.x)*(a.z-c.z)});
  }
  widths.set(halfWidth,triangles);
 }
 let height:number|null=null;
 for(const {a,b,c,minX,maxX,minZ,maxZ,denominator} of triangles){
  if(x<minX-1e-7||x>maxX+1e-7||z<minZ-1e-7||z>maxZ+1e-7||Math.abs(denominator)<1e-9)continue;
  const u=((b.z-c.z)*(x-c.x)+(c.x-b.x)*(z-c.z))/denominator,v=((c.z-a.z)*(x-c.x)+(a.x-c.x)*(z-c.z))/denominator,w=1-u-v;
  if(Math.min(u,v,w)<-1e-7)continue;
  const y=u*a.y+v*b.y+w*c.y;height=height===null?y:Math.max(height,y);
 }
 return height;
}
export interface TrackSample {point:Vec3;distance:number;segment:number;t:number;tangent:Vec3;slope:number;progress:number;underwater:boolean}
/** XZ projection deliberately ignores flight height, retaining the road under an airborne kart. */
export function sampleTrack(track:TrackMap,x:number,z:number):TrackSample {
 let best:TrackSample={point:track.route[0],distance:Infinity,segment:0,t:0,tangent:{x:0,y:0,z:1},slope:0,progress:0,underwater:false};
 for(let i=0;i<track.route.length;i++){
  const a=track.route[i],b=track.route[(i+1)%track.route.length],dx=b.x-a.x,dz=b.z-a.z,length=Math.hypot(dx,dz);
  const t=Math.max(0,Math.min(1,((x-a.x)*dx+(z-a.z)*dz)/Math.max(1e-6,length*length))),px=a.x+dx*t,pz=a.z+dz*t,distance=Math.hypot(px-x,pz-z);
  if(distance>=best.distance)continue;
  const y=a.y+(b.y-a.y)*t,slope=(b.y-a.y)/Math.max(.001,length);
  best={point:{x:px,y,z:pz},distance,segment:i,t,tangent:{x:dx/length,y:slope,z:dz/length},slope,progress:(i+t)/track.route.length,underwater:track.waterLevel!==null&&y+.5<track.waterLevel};
 }
 return best;
}
