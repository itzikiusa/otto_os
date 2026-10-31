///<reference types="svelte" />
;// Host for the assistant bar panel (`#/bar`, the desktop shell's `otto-bar`
// window: transparent, non-activating, native HUD vibrancy behind the page).
//
// The bar itself is the shared FloatingBar.svelte (layout.md §7), pulled in
// with a dynamic import so the main window's bundle doesn't carry it. If that
// chunk fails to load (an update replaced the assets under a long-lived
// window), the pill says so inline with Retry and "Open Otto" — Esc still
// hides it.
//
// Contract with FloatingBar (ui/src/lib/desktop.ts):
//   • size the window with `bar.resize(contentHeight, {radius?})` — it grows
//     upward from the pill, capped at 560 px and the screen;
//   • `Esc` → `bar.hide()`; "Open in Otto" → `openInOtto(route)`;
//   • focus the input on `onDesktopEvent('otto://bar-shown', …)`;
//   • hold-to-talk arrives as `otto://assistant-voice` `{pressed}`.

import { onMount, type Component } from 'svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { bar, onDesktopEvent, openInOtto, useGlassWindow } from '../../lib/desktop';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let Bar: Component | null = $state(null);
  let loading = $state(false);
  let loadError: string | null = $state(null);
  let pillEl: HTMLDivElement | undefined = $state();

  async function load(): Promise<void> {
    loading = true;
    try {
      Bar = (await import('../../lib/components/FloatingBar.svelte')).default as Component;
      loadError = null;
    } catch {
      loadError = 'Otto may have updated while this window was open.';
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    useGlassWindow();
    void load();
  });

  // Error pill only: size + focus + Esc. FloatingBar owns all of this once loaded.
  $effect(() => {
    if (!loadError || Bar) return;
    void bar.resize(56).catch(() => {});
    let unlisten: (() => void) | null = null;
    void onDesktopEvent('otto://bar-shown', () => pillEl?.querySelector('button')?.focus()).then((fn) => (unlisten = fn));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') void bar.hide().catch(() => {});
    };
    window.addEventListener('keydown', onKey);
    return () => {
      unlisten?.();
      window.removeEventListener('keydown', onKey);
    };
  });
;
async () => {

if(Bar){
   { const $$_raB0C = __sveltets_2_ensureComponent(Bar); new $$_raB0C({ target: __sveltets_2_any(), props: {}});}
} else if (loadError){
   { const $$_div0 = svelteHTML.createElement("div", {      "class":`bar-fallback`,"role":`dialog`,"aria-label":`Otto assistant`,});pillEl = $$_div0;
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`the assistant bar`,"error":loadError,"empty":true,"variant":`compact`,loading,"onretry":() => void load(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => void openInOtto().catch(() => {}),});  }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
     LoadState}
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BarHost__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BarHost__SvelteComponent_ = ReturnType<typeof BarHost__SvelteComponent_>;
/*Ωignore_endΩ*/export default BarHost__SvelteComponent_;