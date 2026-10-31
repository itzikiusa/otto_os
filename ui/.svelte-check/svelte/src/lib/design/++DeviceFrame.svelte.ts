///<reference types="svelte" />
;
  export type DeviceKind = 'none' | 'iphone' | 'ipad' | 'desktop';
  /** Chooser entries (design §4.2): iPhone / iPad / desktop / none. */
  export const DEVICES: { id: DeviceKind; label: string }[] = [
    { id: 'none', label: 'Fit' },
    { id: 'iphone', label: 'iPhone' },
    { id: 'ipad', label: 'iPad' },
    { id: 'desktop', label: 'Desktop' },
  ];
;;// DeviceFrame — the "device frame" chooser's chrome around an HTML screen
// (design §4.2): iPhone / iPad / desktop / none. Purely presentational: it
// fixes the frame's CSS size so the sandboxed iframe inside renders at a
// realistic viewport width, and paints a bezel. The `scheme` only tints the
// bezel/backdrop — the mockup's own light/dark handling is up to its CSS
// (`color-scheme` is forwarded to the iframe via the host's class).


import type { Snippet } from 'svelte';
;

  interface Props {
    device: DeviceKind;
    scheme?: 'light' | 'dark';
    children: Snippet;
  };function $$render() {

  
  
  
  
  
  
  

  let { device, scheme = 'light', children }: Props = $props();

  /** CSS px viewport sizes (logical points). */
  const SIZES: Record<Exclude<DeviceKind, 'none'>, { w: number; h: number; label: string }> = {
    iphone: { w: 390, h: 844, label: 'iPhone 14 · 390×844' },
    ipad: { w: 820, h: 1180, label: 'iPad · 820×1180' },
    desktop: { w: 1280, h: 800, label: 'Desktop · 1280×800' },
  };
  const size = $derived(device === 'none' ? null : SIZES[device]);
;
async () => {



if(size){
   { svelteHTML.createElement("div", {  "class":`frame-stage`,});scheme === 'dark';
     { svelteHTML.createElement("div", {       "class":`device ${device}`,"style":`--dev-w:${size.w}px;--dev-h:${size.h}px`,"title":size.label,});
      if(device === 'iphone'){ { svelteHTML.createElement("span", { "class":`notch`,}); }}
       { svelteHTML.createElement("div", { "class":`device-screen`,});;__sveltets_2_ensureSnippet(children()); }
     }
     { svelteHTML.createElement("span", { "class":`frame-label`,});size.label; }
   }
}else{
   { svelteHTML.createElement("div", {  "class":`frame-fit`,});scheme === 'dark';;__sveltets_2_ensureSnippet(children()); }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DeviceFrame__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DeviceFrame__SvelteComponent_ = ReturnType<typeof DeviceFrame__SvelteComponent_>;
/*Ωignore_endΩ*/export default DeviceFrame__SvelteComponent_;