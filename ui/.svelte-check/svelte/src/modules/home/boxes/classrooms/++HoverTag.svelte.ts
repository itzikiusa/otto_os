///<reference types="svelte" />
;// Otto School — the hover tooltip over the 3D stage (ClassroomsBox): a
// kid's title, provider and state with a crisp peek at its live terminal,
// a door's room summary, or the headmaster. The box positions it (it owns
// the projection), so the element is bound back.

import { now } from '../../../../lib/stores/now.svelte';
import { relTime } from '../../../mission-control/lib';
import { isAsleep, kidActivity, providerLabel, roomSummary, type Kid, type Room } from '../../school/model';
import type { SchoolPick } from '../../school/scene';
;

  interface Props {
    hover: SchoolPick | null;
    hoverKid: Kid | null;
    hoverRoom: Room | null;
    /** The hovered kid's last terminal rows (null = no peek). */
    peek: string[] | null;
    el?: HTMLElement | null;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { hover, hoverKid, hoverRoom, peek, el = $bindable(null) }: Props = $props()/*Ωignore_startΩ*/;el;/*Ωignore_endΩ*/;
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {   "class":`hover-tag`,});!!peek;el = $$_div0;
  if(hoverKid){
     { svelteHTML.createElement("b", {});hoverKid.title; }
     { svelteHTML.createElement("small", {});providerLabel(hoverKid.provider);  kidActivity(hoverKid, relTime(hoverKid.lastActiveAt), isAsleep(hoverKid, now())); }
    if(peek){
       { svelteHTML.createElement("pre", { "class":`peek`,});peek.length ? peek.join('\n') : 'No output yet'; }
    }
  } else if (hoverRoom){
     { svelteHTML.createElement("b", {});hoverRoom.name; } { svelteHTML.createElement("small", {});roomSummary(hoverRoom);      }
  } else if (hover?.kind === 'head'){
     { svelteHTML.createElement("b", {});  } { svelteHTML.createElement("small", {});  }
  } else if (hover?.kind === 'door'){
     { svelteHTML.createElement("b", {});    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('el'), slots: {}, events: {} }}
const HoverTag__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HoverTag__SvelteComponent_ = ReturnType<typeof HoverTag__SvelteComponent_>;
/*Ωignore_endΩ*/export default HoverTag__SvelteComponent_;