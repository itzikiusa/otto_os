///<reference types="svelte" />
;// "Couldn’t load your permissions" — an IN-FLOW strip at the foot of the
// shell, above the status bar (ux-design-2-07). It used to float over the
// unified toolbar at the toast layer with no close, covering the back button
// and title for as long as the daemon kept failing. Dismiss holds until the
// next failed read (`auth.capabilitiesNoticeDismissed`).

import Icon from '../lib/components/Icon.svelte';
import LoadState from '../lib/components/LoadState.svelte';
import { auth } from '../lib/stores/auth.svelte';
import { focusAppContent } from '../lib/announce.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  

  let retrying = $state(false);
  async function retry(): Promise<void> {
    if (retrying) return;
    retrying = true;
    try {
      await auth.retryCapabilities();
    } finally {
      retrying = false;
    }
  }

  /** Dismiss removes the whole strip, the focused button with it: hand focus
   *  to the page content instead of letting it fall to <body>. */
  function dismiss(): void {
    auth.dismissCapabilitiesNotice();
    void focusAppContent();
  }
;
async () => {


if(auth.capabilitiesError && !auth.isRoot && !auth.capabilitiesNoticeDismissed){
   { svelteHTML.createElement("div", {   "class":`caps-banner`,...__sveltets_2_empty({"data-testid":`caps-banner`}),});
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`your permissions`,"variant":`compact`,"empty":true,"error":`Some pages may be hidden.`,"loading":retrying,"onretry":retry,errorActions:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":dismiss,"title":`Dismiss until the next failed retry`,"aria-label":`Dismiss permissions notice`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
     LoadState}
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CapsBanner__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CapsBanner__SvelteComponent_ = ReturnType<typeof CapsBanner__SvelteComponent_>;
/*Ωignore_endΩ*/export default CapsBanner__SvelteComponent_;