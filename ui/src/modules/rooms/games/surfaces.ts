import * as T from 'three';

type Surface = 'asphalt'|'metal'|'sand'|'grass'|'stone'|'water';
/** Small, deterministic local textures: no image downloads or external licenses. */
export function surfaceMaterial(kind:Surface,color:number,repeat=1):T.MeshStandardMaterial {
 const canvas=document.createElement('canvas');canvas.width=canvas.height=512;
 const ctx=canvas.getContext('2d')!;
 let seed=91213;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed/4294967296;};
 const pixels=ctx.createImageData(512,512);
 for(let i=0;i<pixels.data.length;i+=4){const n=Math.floor(155+random()*70);pixels.data.set([n,n,n,255],i);}
 ctx.putImageData(pixels,0,0);
 if(kind==='stone'){
  ctx.strokeStyle='rgba(38,25,14,.6)';ctx.lineWidth=5;
  for(let y=0;y<512;y+=85){ctx.beginPath();ctx.moveTo(0,y);ctx.lineTo(512,y);ctx.stroke();
   for(let x=(Math.floor(y/85)%2)*85;x<512;x+=170){ctx.beginPath();ctx.moveTo(x,y);ctx.lineTo(x,y+85);ctx.stroke();}}
  ctx.strokeStyle='rgba(252,230,188,.55)';ctx.lineWidth=2;
  for(let y=7;y<512;y+=85){ctx.beginPath();ctx.moveTo(0,y);ctx.lineTo(512,y);ctx.stroke();}
 }else if(kind==='water'){
  ctx.fillStyle='#b5dcde';ctx.fillRect(0,0,512,512);ctx.strokeStyle='rgba(245,255,255,.6)';ctx.lineWidth=2;
  for(let y=0;y<512;y+=11){ctx.beginPath();for(let x=0;x<=512;x+=4)ctx.lineTo(x,y+Math.sin(x/35+y)*3);ctx.stroke();}
 }else if(kind==='metal'){
  ctx.fillStyle='rgba(10,15,20,.72)';ctx.fillRect(0,0,512,6);ctx.fillRect(0,0,6,512);
  ctx.strokeStyle='rgba(255,255,255,.38)';ctx.lineWidth=2;ctx.strokeRect(10,10,492,492);
  ctx.strokeStyle='rgba(25,30,35,.22)';ctx.lineWidth=1;
  for(let n=0;n<90;n++){const x=random()*512,y=random()*512;ctx.beginPath();ctx.moveTo(x,y);ctx.lineTo(x+random()*45,y+random()*3);ctx.stroke();}
  for(const x of [22,490])for(const y of [22,490]){ctx.fillStyle='rgba(20,25,30,.8)';ctx.beginPath();ctx.arc(x,y,4,0,Math.PI*2);ctx.fill();ctx.fillStyle='rgba(255,255,255,.5)';ctx.fillRect(x-2,y-3,3,1);}
 }else if(kind==='sand'){
  ctx.strokeStyle='rgba(245,238,208,.2)';ctx.lineWidth=3;
  for(let y=0;y<512;y+=14){ctx.beginPath();for(let x=0;x<=512;x+=8)ctx.lineTo(x,y+Math.sin(x/45+y/23)*4);ctx.stroke();}
 }else if(kind==='grass'){
  ctx.strokeStyle='rgba(25,48,14,.3)';ctx.lineWidth=1;
  for(let i=0;i<5000;i++){const x=random()*512,y=random()*512;ctx.beginPath();ctx.moveTo(x,y);ctx.lineTo(x+2,y-3-random()*6);ctx.stroke();}
 }
 const texture=new T.CanvasTexture(canvas);texture.colorSpace=T.SRGBColorSpace;texture.wrapS=texture.wrapT=T.RepeatWrapping;texture.repeat.set(repeat,repeat);texture.anisotropy=4;
 return new T.MeshStandardMaterial({color,map:texture,bumpMap:texture,bumpScale:kind==='metal'?.028:.035,roughness:kind==='metal'?.58:kind==='water'?.3:.92,metalness:kind==='metal'?.4:kind==='water'?.35:.04});
}

export function skyDome(scene:T.Scene,top:number,horizon:number,night=false):void {
 const material=new T.ShaderMaterial({
  side:T.BackSide,depthWrite:false,
  uniforms:{top:{value:new T.Color(top)},horizon:{value:new T.Color(horizon)},night:{value:night?1:0}},
  vertexShader:'varying vec3 direction; void main(){ direction=position; gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0); }',
  fragmentShader:`varying vec3 direction; uniform vec3 top; uniform vec3 horizon; uniform float night;
   void main(){vec3 d=normalize(direction);float h=clamp(d.y*1.6,0.0,1.0);
   vec3 c=mix(horizon,top,pow(h,.65));float sun=pow(max(0.0,dot(d,normalize(vec3(-.6,.35,-.5)))),650.0);
   c+=vec3(1.0,.72,.38)*sun*(1.0-night)*1.3;
   gl_FragColor=vec4(c,1.0);
   #include <tonemapping_fragment>
   #include <colorspace_fragment>
  }`,
 });
 const dome=new T.Mesh(new T.SphereGeometry(240,32,16),material);dome.name='Sky';dome.frustumCulled=false;scene.add(dome);
}
