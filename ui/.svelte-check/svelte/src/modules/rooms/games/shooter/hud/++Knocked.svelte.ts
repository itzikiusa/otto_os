///<reference types="svelte" />
;// Storm Royale "Knocked" overlay (ShooterGame's HUD): the bleed-out bar and,
// while a teammate holds E, the revive progress. Visual only — ShooterGame's
// screen-reader status announces the knock.

import { DOWN_HP, REVIVE_TIME, type Fighter } from '../sim';
;

  interface Props { me: Fighter };function $$render() {

  
  
  
  

  let { me }: Props = $props();
;
async () => {

 { svelteHTML.createElement("div", { "class":`br-knocked`,});
   { svelteHTML.createElement("h2", {});  }
   { svelteHTML.createElement("p", {});me.revive > 0 ? 'A teammate is reviving you — hold on' : 'Crawl to a teammate; they can revive you (E)'; }
   { svelteHTML.createElement("div", {   "class":`br-reload`,"aria-label":`Bleeding out`,}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${(me.hp / DOWN_HP) * 100}%`); } { svelteHTML.createElement("em", {});  } }
  if(me.revive > 0){ { svelteHTML.createElement("div", {   "class":`br-reload use`,"aria-label":`Revive progress`,}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${Math.min(1, me.revive / REVIVE_TIME) * 100}%`); } { svelteHTML.createElement("em", {});  } }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Knocked__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Knocked__SvelteComponent_ = ReturnType<typeof Knocked__SvelteComponent_>;
/*Ωignore_endΩ*/export default Knocked__SvelteComponent_;