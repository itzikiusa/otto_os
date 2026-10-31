///<reference types="svelte" />
;// A studio's identity: a small filled tile (studio colour + glyph) and,
// optionally, its name. The colour is decoration — the name or the tile's
// `title` always says which studio it is.

import Icon, { asIcon } from '../../lib/components/Icon.svelte';
import { studioColorVar, studioInfo } from './model';
;

  interface Props {
    studio: string;
    /** Tile edge: 16 (inline rows), 20 (cards), 28 (studio tiles). */
    size?: 16 | 20 | 28;
    showName?: boolean;
  };function $$render() {

  
  
  
  
  

  let { studio, size = 16, showName = false }: Props = $props();
  const info = $derived(studioInfo(studio));
  const glyph = $derived(size >= 28 ? 16 : 12);
;
async () => {

 { svelteHTML.createElement("span", {   "class":`studio-badge`,"title":showName ? undefined : info.name,});
   { svelteHTML.createElement("span", {     "class":`tile`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, `${size}px`);__sveltets_2_ensureType(String, Number, studioColorVar(studio));
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":asIcon(info.icon, 'box'),"size":glyph,}});}
   }
  if(showName){ { svelteHTML.createElement("span", { "class":`name`,});info.name; }}else{ { svelteHTML.createElement("span", { "class":`sr-only`,});info.name; }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StudioBadge__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StudioBadge__SvelteComponent_ = ReturnType<typeof StudioBadge__SvelteComponent_>;
/*Ωignore_endΩ*/export default StudioBadge__SvelteComponent_;