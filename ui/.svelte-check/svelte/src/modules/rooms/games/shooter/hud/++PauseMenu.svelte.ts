///<reference types="svelte" />
;// Storm Royale pause menu (Esc / Menu): the full key reference plus Resume
// and Leave. It covers the Menu button, so focus moves onto Resume.

import { tick } from 'svelte';
;

  interface Props { networked: boolean; onresume: () => void; onexit: () => void };function $$render() {

  
  
  

  let { networked, onresume, onexit }: Props = $props();
  let resumeButton = $state<HTMLButtonElement>();
  $effect(() => { const b = resumeButton; if (b) void tick().then(() => b.focus()); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`br-cover`,}); { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("p", {});networked ? 'The match continues for everyone else while this menu is open.' : 'The island waits for you.'; }
   { svelteHTML.createElement("p", { "class":`br-keys`,});                                                                                                                                                                               }
   { const $$_button1 = svelteHTML.createElement("button", {    "class":`btn primary`,"onclick":onresume,});resumeButton = $$_button1;  } { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onexit,});  } }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PauseMenu__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PauseMenu__SvelteComponent_ = ReturnType<typeof PauseMenu__SvelteComponent_>;
/*Ωignore_endΩ*/export default PauseMenu__SvelteComponent_;