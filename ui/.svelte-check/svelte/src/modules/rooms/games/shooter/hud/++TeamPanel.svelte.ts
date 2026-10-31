///<reference types="svelte" />
;// Storm Royale team panel: one row per teammate with health + shield, or the
// bleed-out bar while knocked (team modes; ShooterGame decides when to show it).

import { DOWN_HP, type Fighter } from '../sim';
import { teamColour } from '../catalog';
import { hex } from './minimap';
;

  interface Props { mates: Fighter[]; team: number; modeName: string };function $$render() {

  
  
  
  
  

  let { mates, team, modeName }: Props = $props();
;
async () => {

 { svelteHTML.createElement("ul", {    "class":`br-team`,"aria-label":`Your ${modeName.toLowerCase().replace(/s$/, '')} team`,});__sveltets_2_ensureType(String, Number, hex(teamColour(team)));
     for(let m of __sveltets_2_ensureArray(mates)){m.id;
     { svelteHTML.createElement("li", {  });m.mode === 'down';m.mode === 'dead';
       { svelteHTML.createElement("i", { "aria-hidden":`true`,}); } { svelteHTML.createElement("span", {});m.name; }
      if(m.mode === 'dead'){ { svelteHTML.createElement("em", {});  }
      } else if (m.mode === 'down'){ { svelteHTML.createElement("em", {});m.revive > 0 ? 'Reviving…' : 'Knocked'; } { svelteHTML.createElement("b", {           "class":`bleed`,"role":`meter`,"aria-label":`${m.name} bleeding out`,"aria-valuemin":0,"aria-valuemax":DOWN_HP,"aria-valuenow":Math.round(m.hp),}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${(m.hp / DOWN_HP) * 100}%`); } }
      }else{ { svelteHTML.createElement("b", {         "role":`meter`,"aria-label":`${m.name} health and shield`,"aria-valuemin":0,"aria-valuemax":200,"aria-valuenow":Math.round(m.hp + m.shield),}); { svelteHTML.createElement("span", {  "class":`hp`,});__sveltets_2_ensureType(String, Number, `${m.hp / 2}%`); } { svelteHTML.createElement("span", {  "class":`sh`,});__sveltets_2_ensureType(String, Number, `${m.shield / 2}%`); } }}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TeamPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TeamPanel__SvelteComponent_ = ReturnType<typeof TeamPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default TeamPanel__SvelteComponent_;