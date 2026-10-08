import type { ArenaMap, CoverBox, TrackMap, Vec3 } from './types.ts';
const box = (x:number,z:number,width:number,depth:number,height=3):CoverBox => ({x,y:0,z,width,depth,height});
const point = (x:number,z:number):Vec3 => ({x,y:0,z});
export const ARENAS: Record<string,ArenaMap> = {
 station: {id:'station',name:'Orbital Station',halfSize:24,spawns:[point(-17,-17),point(17,17),point(-17,17),point(17,-17)],cover:[box(0,0,8,4,3.4),box(-11,-7,4,7),box(11,7,4,7),box(-10,12,6,3,1.3),box(10,-12,6,3,1.3),box(0,-17,3,3,2),box(0,17,3,3,2)]},
 foundry: {id:'foundry',name:'Ember Foundry',halfSize:25,spawns:[point(-19,-18),point(19,18),point(-19,18),point(19,-18)],cover:[box(-6,0,4,17,4),box(6,0,4,17,4),box(0,13,9,3,1.4),box(0,-13,9,3,1.4),box(-17,0,4,5,2.6),box(17,0,4,5,2.6)]},
 dunes: {id:'dunes',name:'Sunken Dunes',halfSize:28,spawns:[point(-21,-20),point(21,20),point(-21,20),point(21,-20)],cover:[box(0,0,6,7,4),box(-12,-10,8,4,2.5),box(12,10,8,4,2.5),box(-14,11,5,8,3),box(14,-11,5,8,3),box(0,-19,4,3,1.2),box(0,19,4,3,1.2)]},
};
function track(id:string,name:string,width:number,points:number[][]):TrackMap {
 return {id,name,width,route:points.map(([x,z])=>point(x,z)),pickups:[2,5,8]};
}
export const TRACKS: Record<string,TrackMap> = {
 coast:track('coast','Coral Coast',10,[[0,-44],[26,-40],[43,-23],[47,2],[36,29],[14,42],[-13,40],[-30,24],[-42,5],[-39,-19],[-23,-39]]),
 forest:track('forest','Fernwood Rally',9,[[0,-52],[24,-45],[34,-25],[18,-7],[31,12],[28,38],[6,50],[-20,41],[-37,20],[-27,-1],[-40,-24],[-26,-45]]),
 neon:track('neon','Neon Overdrive',10,[[0,-48],[27,-48],[45,-30],[45,-4],[29,13],[43,31],[25,47],[-1,47],[-25,34],[-44,13],[-44,-17],[-27,-42]]),
};
export function arenaFor(id:string):ArenaMap { return ARENAS[id] ?? ARENAS.station; }
export function trackFor(id:string):TrackMap { return TRACKS[id] ?? TRACKS.coast; }
export const arenaMap = arenaFor;
export const trackMap = trackFor;
