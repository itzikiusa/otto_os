///<reference types="svelte" />
;// The side pane: a movable local native webview on desktop, or an iframe
// at `?embed=1#/<route>` — its own router and stores (lib/sidePane.ts). It
// has no bar of its own: its Swap / Open in main pane / Close controls sit
// at the end of the page's own top row, inside the frame (PaneControls).
// The frame is built once per pane and navigated in place afterwards, so
// changing the pane's module never reloads it. Until it reports ready, a
// cover shaped like the page (a header row + skeleton) stands in, with a
// Close button; a pane that never boots says so, with Retry.

import { untrack } from 'svelte';
import { nativePane, nativePaneAvailable } from '../lib/nativePane';
import { paneBounds, LatestPaneLayout, type PaneBounds } from '../lib/nativePanePolicy';
import { ctxMenu } from '../lib/contextmenu.svelte';
import Icon from '../lib/components/Icon.svelte';
import Skeleton from '../lib/components/Skeleton.svelte';
import { sidePane } from '../lib/stores/sidePane.svelte';
import { ui, isTauri } from '../lib/stores/ui.svelte';
;

  interface Props {
    label: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { label }: Props = $props();

  let surface: HTMLElement | undefined = $state();
  let scheduleLayout: (() => void) | undefined;
  $effect(() => {
    const el = surface;
    void sidePane.generation;
    if (!el || !nativePaneAvailable) return;
    return untrack(() => {
      let bounds: PaneBounds = paneBounds(el.getBoundingClientRect(), window.innerWidth, window.innerHeight, ui.zoom) ?? { x: 0, y: 0, width: 1, height: 1 };
      const stop = sidePane.attachNative(bounds);
      let raf = 0;
      let restoreFocus = false;
      let occluded = false;
      const layouts = new LatestPaneLayout<{ bounds: PaneBounds; visible: boolean; restore: boolean }>(async (next) => {
        await nativePane.layout(next.bounds, next.visible);
        if (next.visible && next.restore) {
          await nativePane.restoreAttachedFocus(() => sidePane.showing && !ui.overlayOpen && !ctxMenu.open && !sidePane.dragging);
          restoreFocus = false;
        }
      });
      const schedule = (): void => {
        if (raf) return;
        raf = requestAnimationFrame(() => {
          raf = 0;
          bounds = paneBounds(el.getBoundingClientRect(), window.innerWidth, window.innerHeight, ui.zoom) ?? bounds;
          const blocked = ui.overlayOpen || ctxMenu.open || sidePane.dragging;
          if (blocked && !occluded) restoreFocus = sidePane.focused;
          const visible = sidePane.showing && sidePane.status === 'ready' && !blocked;
          const restore = visible && restoreFocus;
          occluded = blocked;
          layouts.push({ bounds, visible, restore });
        });
      };
      scheduleLayout = schedule;
      const observer = new ResizeObserver(schedule);
      observer.observe(el);
      window.addEventListener('resize', schedule);
      window.addEventListener('scroll', schedule, true);
      schedule();
      return () => {
        scheduleLayout = undefined;
        cancelAnimationFrame(raf);
        observer.disconnect();
        window.removeEventListener('resize', schedule);
        window.removeEventListener('scroll', schedule, true);
        layouts.stop();
        stop();
      };
    });
  });
  $effect(() => {
    void sidePane.showing; void sidePane.status; void sidePane.nativeState;
    void sidePane.placement; void sidePane.dragging; void ui.zoom; void ui.overlayOpen; void ctxMenu.open;
    scheduleLayout?.();
  });

  let frame: HTMLIFrameElement | undefined = $state();
  $effect(() => {
    const f = frame;
    if (!f) return;
    sidePane.attach(f);
    return () => sidePane.detach(f);
  });

  // The cover's header row sits where the page's will: under the traffic
  // lights when this pane leads and the sidebar is the narrow Rail.
  const padTraffic = $derived(isTauri && sidePane.placement === 'leading' && !ui.railExpanded);
;
async () => {

 { const $$_section0 = svelteHTML.createElement("section", {          "class":`side-pane`,"aria-label":`${label} (side pane)`,...__sveltets_2_empty({"data-testid":`side-pane`}),...__sveltets_2_empty({"data-module":sidePane.key}),});surface = $$_section0;nativePaneAvailable && !sidePane.showing;
  if(!nativePaneAvailable){
  sidePane.generation; {
     { const $$_iframe1 = svelteHTML.createElement("iframe", {          "src":sidePane.frameSrc(),"title":`${label} (side pane)`,...__sveltets_2_empty({"data-testid":`side-pane-frame`}),});frame = $$_iframe1;sidePane.status === 'ready';sidePane.dragging; }
  }
  }
  if(sidePane.status !== 'ready'){
     { svelteHTML.createElement("div", {   "class":`sp-cover`,...__sveltets_2_empty({"data-testid":`side-pane-cover`}),});
       { svelteHTML.createElement("div", {  "class":`sp-head chrome-material`,});padTraffic;
         { svelteHTML.createElement("span", { "class":`sp-title`,});label; }
         { svelteHTML.createElement("span", { "class":`sp-grow`,}); }
         { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => sidePane.close(),"aria-label":`Close side pane`,"title":`Close side pane`,"aria-keyshortcuts":`Meta+\\`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
         }
       }
      if(sidePane.status === 'loading'){
        
         { svelteHTML.createElement("div", { "class":`sp-skeleton`,});
           { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {     "rows":6,"height":28,label,}});}
         }
      }else{
         { svelteHTML.createElement("div", {   "class":`sp-error`,"role":`alert`,});
           { svelteHTML.createElement("span", { "class":`sp-error-icon`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":24,}});} }
           { svelteHTML.createElement("h3", {});  label; }
           { svelteHTML.createElement("p", {});            }
           { svelteHTML.createElement("div", { "class":`sp-error-actions`,});
             { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => sidePane.retry(),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} 
             }
             { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => sidePane.close(),});  }
           }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SidePane__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SidePane__SvelteComponent_ = ReturnType<typeof SidePane__SvelteComponent_>;
/*Ωignore_endΩ*/export default SidePane__SvelteComponent_;