///<reference types="svelte" />
;// The vault's page-preview popover (state in previewStore.svelte.ts). Clamped
// into the viewport: hangs under the anchor, flips above it only when that
// fits, otherwise pins to the bottom edge; height capped + scrollable.

import { LINK_PREVIEW_ID, linkPreview } from './previewStore.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
function $$render() {

  
  
  
  
  

  let el = $state<HTMLElement | undefined>();
  let h = $state(120);
  $effect(() => {
    void linkPreview.current?.state;
    if (el) h = el.offsetHeight;
  });
  const style = $derived.by(() => {
    const p = linkPreview.current;
    if (!p) return '';
    const vw = window.innerWidth, vh = window.innerHeight;
    const w = Math.min(320, vw - 16);
    const left = Math.max(8, Math.min(p.x, vw - w - 8));
    const maxH = Math.max(80, Math.min(360, vh - 16));
    const height = Math.min(h, maxH);
    let top = p.y;
    if (top + height > vh - 8) top = p.top - 6 - height >= 8 ? p.top - 6 - height : Math.max(8, vh - height - 8);
    return `left:${left}px;top:${top}px;width:${w}px;max-height:${maxH}px`;
  });
;
async () => {


 { svelteHTML.createElement("svelte:window", {  "onkeydown":(e) => { if (e.key === 'Escape' && linkPreview.current) linkPreview.hide(); },});}

if(linkPreview.current){
  const p = linkPreview.current;
   { const $$_div0 = svelteHTML.createElement("div", {         "class":`lp`,"id":LINK_PREVIEW_ID,"role":`tooltip`,style,...__sveltets_2_empty({"data-testid":`vault-link-preview`}),});el = $$_div0;
     { svelteHTML.createElement("div", { "class":`lp-title`,});p.title;if(p.type){ { svelteHTML.createElement("span", { "class":`lp-type`,});p.type; }} }
     { svelteHTML.createElement("div", { "class":`lp-path`,});p.path; }
    if(p.state === 'loading'){ { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":2,"height":12,"label":`the preview`,}});}
    } else if (p.state === 'error'){ { svelteHTML.createElement("p", { "class":`lp-dim`,});  }
    } else if (p.text){ { svelteHTML.createElement("p", {});p.text; }
    }else{ { svelteHTML.createElement("p", { "class":`lp-dim`,});  }}
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LinkPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LinkPreview__SvelteComponent_ = ReturnType<typeof LinkPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default LinkPreview__SvelteComponent_;