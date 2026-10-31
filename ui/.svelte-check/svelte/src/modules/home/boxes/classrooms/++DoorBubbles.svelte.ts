///<reference types="svelte" />
;// Otto School — the ✋ / ❗ badges over corridor doors (ClassroomsBox's
// overlay). Decorative (the overlay is aria-hidden; the "needs you" queue
// and the list carry the same facts as real buttons). `bubble` registers
// each badge with the box, which places it at its door's projection.

import type { Action } from 'svelte/action';
import type { Room } from '../../school/model';
;

  interface Props {
    /** Corridor doors with someone waiting (or in error). */
    rooms: Room[];
    bubble: Action<HTMLElement, string>;
  };function $$render() {

  
  
  
  
  
  

  let { rooms, bubble }: Props = $props();
;
async () => {

   for(let r of __sveltets_2_ensureArray(rooms)){r.id;
   {const $$action_0 = __sveltets_2_ensureAction(bubble(svelteHTML.mapElementTag('div'),(`door:${r.id}`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {    "class":`bubble`,});r.counts.needsYou === 0;
    r.counts.needsYou > 0 ? `✋ ${r.counts.needsYou}` : `❗ ${r.counts.error}`;
   }}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DoorBubbles__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DoorBubbles__SvelteComponent_ = ReturnType<typeof DoorBubbles__SvelteComponent_>;
/*Ωignore_endΩ*/export default DoorBubbles__SvelteComponent_;