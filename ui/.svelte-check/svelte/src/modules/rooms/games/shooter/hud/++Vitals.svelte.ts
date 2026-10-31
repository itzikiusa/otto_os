///<reference types="svelte" />
;// Storm Royale vitals panel: shield and health bars, grenades, killstreaks and
// tactical-sprint stamina, and the building materials (ShooterGame's HUD).

import { TAC_TIME, type Fighter } from '../sim';
import { MATERIAL_COLOURS, MATERIAL_NAMES, NADES, STREAKS } from '../weapons';
import type { ShooterMode } from '../catalog';
import { hex } from './minimap';
;

  interface Props { me: Fighter; mode: ShooterMode; uavUp: boolean; building: boolean; material: number };function $$render() {

  
  
  
  
  
  

  let { me, mode, uavUp, building, material }: Props = $props();
;
async () => {

 { svelteHTML.createElement("div", {   "class":`br-vitals`,"aria-label":`Health and shield`,});
   { svelteHTML.createElement("div", {           "class":`br-bar shield`,"role":`meter`,"aria-label":`Shield`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":Math.round(me.shield),}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${me.shield}%`); } { svelteHTML.createElement("b", {}); { svelteHTML.createElement("svg", {   "viewBox":`0 0 24 24`,"aria-hidden":`true`,}); { svelteHTML.createElement("path", {  "d":`M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5z`,});} } Math.round(me.shield); } }
   { svelteHTML.createElement("div", {           "class":`br-bar health`,"role":`meter`,"aria-label":`Health`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":Math.round(me.hp),}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${me.hp}%`); } { svelteHTML.createElement("b", {}); { svelteHTML.createElement("svg", {   "viewBox":`0 0 24 24`,"aria-hidden":`true`,}); { svelteHTML.createElement("path", {  "d":`M10 3h4v7h7v4h-7v7h-4v-7H3v-4h7z`,});} } Math.round(me.hp); } }
   { svelteHTML.createElement("div", {   "class":`br-gear`,"aria-label":`Grenades and killstreaks`,});
     { svelteHTML.createElement("span", {   "title":`${NADES[me.lethal].name} (G)`,});me.lethalN === 0; { svelteHTML.createElement("kbd", {});  }NADES[me.lethal].name;  { svelteHTML.createElement("b", {});me.lethalN; } }
     { svelteHTML.createElement("span", {   "title":`${NADES[me.tacKind].name} (T)`,});me.tacN === 0; { svelteHTML.createElement("kbd", {});  }NADES[me.tacKind].name;  { svelteHTML.createElement("b", {});me.tacN; } }
        for(let st of __sveltets_2_ensureArray(STREAKS)){let k = 1;st.id; { svelteHTML.createElement("span", {    "class":`br-streak`,"title":(me.ready & (1 << k)) > 0 ? `${st.name} ready — press X` : `${st.name}: ${mode === 'zombies' ? st.kills * 4 : st.kills} eliminations in a row`,});(me.ready & (1 << k)) > 0;if((me.ready & (1 << k)) > 0){ { svelteHTML.createElement("kbd", {});  }}st.name;if(!((me.ready & (1 << k)) > 0)){ { svelteHTML.createElement("small", {}); Math.min(Math.floor(me.streak), st.kills); st.kills; }} }}
    if(uavUp){ { svelteHTML.createElement("span", {   "class":`br-streak ready`,"title":`Your team’s UAV shows every enemy on the map`,});  }}
    if(me.tactical || me.tacT < TAC_TIME){ { svelteHTML.createElement("span", {   "class":`br-tac`,"title":`Tactical sprint (double-tap Shift)`,}); { svelteHTML.createElement("i", { });__sveltets_2_ensureType(String, Number, `${(me.tacT / TAC_TIME) * 100}%`); } }}
   }
   { svelteHTML.createElement("div", {   "class":`br-mats`,"aria-label":`Building materials`,});
        for(let name of __sveltets_2_ensureArray(MATERIAL_NAMES)){let m = 1;name; { svelteHTML.createElement("span", {    "title":name,});building && m === material;__sveltets_2_ensureType(String, Number, hex(MATERIAL_COLOURS[m])); { svelteHTML.createElement("i", { "aria-hidden":`true`,}); }name;  { svelteHTML.createElement("b", {});me.mats[m]; } }}
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Vitals__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Vitals__SvelteComponent_ = ReturnType<typeof Vitals__SvelteComponent_>;
/*Ωignore_endΩ*/export default Vitals__SvelteComponent_;