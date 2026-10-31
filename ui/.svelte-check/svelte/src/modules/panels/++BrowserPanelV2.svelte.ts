///<reference types="svelte" />
;// Right-panel Browser tab, "v2": the Browser module (reader/live tabs,
// persisted marks, ask bar) embedded beside the active agent session. The
// ask bar targets that session, so marking an element here and asking
// about it lands in the pane the user is already working in. Shares the
// `browser` store (tabs/marks) with the Browser page — they are never
// mounted at the same time (the panel only exists on the Agents module),
// so the two never fight over a live tab's native webview.

import BrowserView from '../browser/BrowserView.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  

  const target = $derived(ws.activeSession?.kind === 'agent' ? ws.activeSessionId : null);
;
async () => {

 { svelteHTML.createElement("div", { "class":`v2`,});
   { const $$_weiVresworB1C = __sveltets_2_ensureComponent(BrowserView); new $$_weiVresworB1C({ target: __sveltets_2_any(), props: {  "embedded":true,"targetSessionId":target,}});}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrowserPanelV2__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrowserPanelV2__SvelteComponent_ = ReturnType<typeof BrowserPanelV2__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrowserPanelV2__SvelteComponent_;