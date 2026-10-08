import type { CoverBox, GameEvent, GameState, Vec3 } from './types.ts';
export const clamp=(n:number,min:number,max:number)=>Math.max(min,Math.min(max,n));
export const angleDelta=(target:number,current:number)=>Math.atan2(Math.sin(target-current),Math.cos(target-current));
export const distance=(a:Vec3,b:Vec3)=>Math.hypot(a.x-b.x,a.z-b.z);
export function random(s:GameState):number { s.rng=(Math.imul(s.rng,1664525)+1013904223)>>>0;return s.rng/4294967296; }
export function emit(s:GameState,type:GameEvent['type'],player:number,extra:Partial<GameEvent>={}):void {const p=s.players[player];if(s.events.length>=128)s.events.shift();s.events.push({id:++s.eventSequence,type,player,x:p.x,y:p.y,z:p.z,...extra});}
/** Slab intersection returns entry fraction of segment (also used for camera obstruction). */
export function segmentBox(from:Vec3,to:Vec3,box:CoverBox,pad=0):number|null {
 let near=0,far=1;
 for(const [a,b,min,max] of [[from.x,to.x,box.x-box.width/2-pad,box.x+box.width/2+pad],[from.y,to.y,box.y-pad,box.y+box.height+pad],[from.z,to.z,box.z-box.depth/2-pad,box.z+box.depth/2+pad]]) {
 const delta=b-a;if(Math.abs(delta)<1e-9){if(a<min||a>max)return null;continue;}
 const t1=(min-a)/delta,t2=(max-a)/delta;near=Math.max(near,Math.min(t1,t2));far=Math.min(far,Math.max(t1,t2));if(near>far)return null;
 }
 return near;
}
export function lineOfSight(from:Vec3,to:Vec3,cover:CoverBox[],pad=0):boolean {return !cover.some(b=>segmentBox(from,to,b,pad)!==null);}
