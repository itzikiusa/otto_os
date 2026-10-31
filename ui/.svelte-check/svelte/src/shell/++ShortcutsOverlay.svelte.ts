///<reference types="svelte" />
;// Keyboard-shortcut cheat sheet, triggered by `?` (handled in App.svelte).
// A shared Modal: Esc / backdrop / ✕ close it, focus is trapped, and it
// registers as an overlay (the live browser's native webview hides under
// it). The binding list is derived from the KEYMAP in lib/keys.ts so it
// stays in lockstep with the actual chords.

import Modal from '../lib/components/Modal.svelte';
import { KEYMAP } from '../lib/keys';
import { shortcutChips } from '../lib/shortcutChips';
;

  interface Props {
    open: boolean;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { open, onclose }: Props = $props();
;
async () => {

if(open){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Keyboard shortcuts`,"width":720,onclose,}});
     { svelteHTML.createElement("div", { "class":`sc-grid`,});
         for(let group of __sveltets_2_ensureArray(KEYMAP)){group.category;
         { svelteHTML.createElement("section", { "class":`sc-group`,});
           { svelteHTML.createElement("h3", { "class":`sc-cat`,});group.category; }
             for(let b of __sveltets_2_ensureArray(group.bindings)){b.keys + b.label;
             { svelteHTML.createElement("div", { "class":`sc-row`,});
               { svelteHTML.createElement("span", { "class":`sc-label`,});b.label; }
               { svelteHTML.createElement("span", { "class":`sc-keys`,});
                    for(let c of __sveltets_2_ensureArray(shortcutChips(b.keys))){let i = 1;i;if(c.kind === 'sep'){ { svelteHTML.createElement("span", {  "class":`sc-sep`,});c.text; }}else{ { svelteHTML.createElement("kbd", {});c.text; }}}
               }
             }
          }
         }
      }
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ShortcutsOverlay__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ShortcutsOverlay__SvelteComponent_ = ReturnType<typeof ShortcutsOverlay__SvelteComponent_>;
/*Ωignore_endΩ*/export default ShortcutsOverlay__SvelteComponent_;