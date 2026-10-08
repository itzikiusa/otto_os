import type {Weapon} from './types.ts';
export interface WeaponDefinition {name:string;capacity:number;reload:number;interval:number;damage:number;pellets:number;spread:number;range:number}
export const WEAPONS:Record<Weapon,WeaponDefinition>={
 rifle:{name:'Pulse rifle',capacity:24,reload:1.4,interval:.16,damage:25,pellets:1,spread:0,range:90},
 scatter:{name:'Scatter blaster',capacity:6,reload:1.8,interval:.72,damage:14,pellets:7,spread:.13,range:24},
 rail:{name:'Rail lance',capacity:4,reload:2.2,interval:1.15,damage:70,pellets:1,spread:0,range:120},
};
export const WEAPON_ORDER:Weapon[]=['rifle','scatter','rail'];
