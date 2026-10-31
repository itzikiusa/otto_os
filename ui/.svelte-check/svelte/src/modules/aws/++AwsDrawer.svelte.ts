///<reference types="svelte" />
;// Right-side detail drawer for the AWS service views (EC2, RDS) — the same
// look as the Kubernetes ResourceDrawer: header (state pill + name + id,
// close), tab strip, scrollable body. The chrome (desktop column / phone
// sheet, ✕, Esc, modal registration, focus) is the shared DockedDrawer;
// this adds the kind/name/id/status header and the tab strip (←/→ move).

import type { BadgeTone } from '../../lib/status';
import { sentenceCase } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import type { Snippet } from 'svelte';
import { onTabKey } from '../../lib/tabKeys';
import DockedDrawer from '../../lib/components/DockedDrawer.svelte';
;

  interface Props {
    /** Small uppercase kind label ("instance", "db instance"). */
    kind: string;
    name: string;
    /** Secondary id shown after the name (instance id, endpoint…). */
    id?: string;
    /** Status text (the raw AWS state; shown sentence-cased) + its Badge tone. */
    status?: string;
    statusTone?: BadgeTone;
    tabs: { id: string; label: string }[];
    tab: string;
    ontab: (id: string) => void;
    onclose: () => void;
    children: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { kind, name, id = '', status = '', statusTone = 'neutral', tabs, tab, ontab, onclose, children }: Props =
    $props();

;
async () => {

 { const $$_rewarDdekcoD0C = __sveltets_2_ensureComponent(DockedDrawer); const $$_rewarDdekcoD0 = new $$_rewarDdekcoD0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"open":true,"title":`${kind} details`,onclose,"testid":`aws-drawer`,head:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", { "class":`dr-title`,});
       { svelteHTML.createElement("span", { "class":`dr-kind`,});kind; }
       { svelteHTML.createElement("span", {   "class":`dr-name`,"title":name,});name; }
      if(id && id !== name){ { svelteHTML.createElement("span", {   "class":`dr-id mono`,"title":id,});id; }}
      if(status){ { const $$_egdaB1C = __sveltets_2_ensureComponent(Badge); new $$_egdaB1C({ target: __sveltets_2_any(), props: {      "tone":statusTone,"label":sentenceCase(status),"testid":`aws-drawer-status`,}});}}
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {head} = $$_rewarDdekcoD0.$$prop_def;/*Ωignore_endΩ*/
  
   { svelteHTML.createElement("div", { "class":`dr-tabs`,});
    { svelteHTML.createElement("div", {     "class":`segmented`,"role":`tablist`,"aria-label":`Detail tabs`,});
       for(let t of __sveltets_2_ensureArray(tabs)){t.id;
       { svelteHTML.createElement("button", {                "role":`tab`,"id":`dr-tab-${t.id}`,"aria-controls":`dr-panel`,"aria-selected":tab === t.id,"tabindex":tab === t.id ? 0 : -1,"onclick":() => ontab(t.id),"onkeydown":onTabKey,});tab === t.id;t.label; }
    }
    }
   }
   { svelteHTML.createElement("div", {       "class":`dr-body`,"id":`dr-panel`,"role":`tabpanel`,"aria-labelledby":`dr-tab-${tab}`,});
    ;__sveltets_2_ensureSnippet(children());
   }
 DockedDrawer}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AwsDrawer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AwsDrawer__SvelteComponent_ = ReturnType<typeof AwsDrawer__SvelteComponent_>;
/*Ωignore_endΩ*/export default AwsDrawer__SvelteComponent_;