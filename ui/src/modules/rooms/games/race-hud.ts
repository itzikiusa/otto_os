import {trackFor} from './maps.ts';
import type {GameState,TrackMap} from './types.ts';
export function raceClock(seconds:number):string {
 const hundredths=Math.max(0,Math.floor(seconds*100));
 return `${Math.floor(hundredths/6000)}:${String(Math.floor(hundredths/100)%60).padStart(2,'0')}.${String(hundredths%100).padStart(2,'0')}`;
}
export function racePosition(state:GameState,id:number):1|2 {
 if(state.phase==='finished'&&state.winner!==null)return state.winner===id?1:2;
 const track=trackFor(state.config.map),n=track.route.length;
 const progress=state.players.map(p=>{
  const previous=(p.checkpoint+n-1)%n,a=track.route[previous],b=track.route[p.checkpoint];
  const dx=b.x-a.x,dz=b.z-a.z;
  const t=Math.max(0,Math.min(1,((p.x-a.x)*dx+(p.z-a.z)*dz)/Math.max(.01,dx*dx+dz*dz)));
  return p.lap*n+previous+t;
 });
 return progress[id]>=progress[1-id]?1:2;
}
export function courseOutline(track:TrackMap):string{return [...track.route,track.route[0]].map(p=>`${p.x},${p.z}`).join(' ');}
export function courseBounds(track:TrackMap):string {
 const xs=track.route.map(p=>p.x),zs=track.route.map(p=>p.z),minX=Math.min(...xs)-12,minZ=Math.min(...zs)-12;
 return `${minX} ${minZ} ${Math.max(...xs)-minX+12} ${Math.max(...zs)-minZ+12}`;
}
