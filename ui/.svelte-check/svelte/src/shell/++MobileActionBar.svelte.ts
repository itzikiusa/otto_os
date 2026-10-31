///<reference types="svelte" />
;// On-screen action bar for phone users who can't reach ⌘-key chords.
// Each button invokes the SAME function the keyboard shortcut calls (passed in
// as props from App.svelte). Rendered only on phone (viewport.isPhone gate in
// App.svelte); desktop is completely unaffected.
//
// Actions exposed:
//   palette    – ⌘K  open/close command palette (primary nav on mobile)
//   newSession – ⌘T  open the New Session modal
//   closeTab   – ⌘W  close the active tab/session
//   find       – ⌘F  open find bar (terminal find if focused, else page find)
//   broadcast  – ⌘⇧B broadcast a message to all sessions (shown when sessions exist)

import Icon from '../lib/components/Icon.svelte';
;

  interface Props {
    onpalette: () => void;
    onnewSession: () => void;
    oncloseTab: () => void;
    onfind: () => void;
    /** Only rendered when the caller deems it relevant (e.g. sessions exist). */
    showBroadcast?: boolean;
    onbroadcast?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  


  let {
    onpalette,
    onnewSession,
    oncloseTab,
    onfind,
    showBroadcast = false,
    onbroadcast,
  }: Props = $props();
;
async () => {


 { svelteHTML.createElement("div", {      "class":`action-bar`,"role":`toolbar`,"aria-label":`Quick actions`,...__sveltets_2_empty({"data-find-neutral":true}),});
   { svelteHTML.createElement("button", {           "class":`ab-btn`,"onclick":onpalette,"title":`Open command palette`,"aria-keyshortcuts":`Meta+K`,"aria-label":`Open command palette`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`command`,"size":20,}});}
     { svelteHTML.createElement("span", { "class":`ab-label`,});  }
   }

   { svelteHTML.createElement("button", {           "class":`ab-btn`,"onclick":onnewSession,"title":`New session`,"aria-keyshortcuts":`Meta+T`,"aria-label":`New session`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":20,}});}
     { svelteHTML.createElement("span", { "class":`ab-label`,});  }
   }

   { svelteHTML.createElement("button", {           "class":`ab-btn`,"onclick":oncloseTab,"title":`Close current tab`,"aria-keyshortcuts":`Meta+W`,"aria-label":`Close current tab`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":20,}});}
     { svelteHTML.createElement("span", { "class":`ab-label`,});  }
   }

   { svelteHTML.createElement("button", {           "class":`ab-btn`,"onclick":onfind,"title":`Find in terminal or page`,"aria-keyshortcuts":`Meta+F`,"aria-label":`Find in terminal or page`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":20,}});}
     { svelteHTML.createElement("span", { "class":`ab-label`,});  }
   }

  if(showBroadcast && onbroadcast){
     { svelteHTML.createElement("button", {           "class":`ab-btn`,"onclick":onbroadcast,"title":`Broadcast message to all sessions`,"aria-keyshortcuts":`Meta+Shift+B`,"aria-label":`Broadcast message to all sessions`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":20,}});}
       { svelteHTML.createElement("span", { "class":`ab-label`,});  }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MobileActionBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MobileActionBar__SvelteComponent_ = ReturnType<typeof MobileActionBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default MobileActionBar__SvelteComponent_;