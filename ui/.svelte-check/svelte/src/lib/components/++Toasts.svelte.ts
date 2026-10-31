///<reference types="svelte" />
;
import { toasts, type Toast } from '../toast.svelte';
import { barStore } from '../stores/bar.svelte';
import Icon, { type IconName } from './Icon.svelte';
function $$render() {

  
  
  

  // The stripe carries the tone, and the glyph repeats it (colour is never
  // the only signal).
  const ICON: Record<Toast['level'], IconName> = {
    info: 'info',
    success: 'check',
    warn: 'warning',
    error: 'warning',
  };
;
async () => {


 { svelteHTML.createElement("div", {      "class":`toasts`,"aria-live":`polite`,"aria-relevant":`additions text`,});__sveltets_2_ensureType(String, Number, `${barStore.toastLift} `);
     for(let t of __sveltets_2_ensureArray(toasts.toasts)){t.id;
    
     { svelteHTML.createElement("div", {               "class":`toast ${t.level}`,"role":t.level === 'error' ? 'alert' : undefined,"aria-atomic":`true`,"onmouseenter":() => toasts.pause(t.id),"onmouseleave":() => toasts.resume(t.id),"onfocusin":() => toasts.pause(t.id),"onfocusout":() => toasts.resume(t.id),});
       { svelteHTML.createElement("div", { "class":`toast-stripe`,}); }
       { svelteHTML.createElement("span", {   "class":`toast-icon`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":ICON[t.level],"size":14,}});} }
       { svelteHTML.createElement("div", { "class":`toast-content`,});
         { svelteHTML.createElement("div", { "class":`toast-title`,});
          t.title;
          if(t.count > 1){ { svelteHTML.createElement("span", {   "class":`toast-count`,"title":`Happened ${t.count} times`,}); t.count; }}
         }
        if(t.body){ { svelteHTML.createElement("div", { "class":`toast-body`,});t.body; }}
        if(t.action){
           { svelteHTML.createElement("button", {   "class":`btn small toast-action`,"onclick":() => void toasts.runAction(t.id),});t.action.label; }
        }
       }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => toasts.dismiss(t.id),"aria-label":`Dismiss`,"title":`Dismiss`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
       }
     }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const Toasts__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type Toasts__SvelteComponent_ = InstanceType<typeof Toasts__SvelteComponent_>;
/*Ωignore_endΩ*/export default Toasts__SvelteComponent_;