///<reference types="svelte" />
;// Otto School — the 3D stage's "needs you" queue (ClassroomsBox): the
// school-wide sessions that need you or failed, as real buttons that walk
// to the kid (or open it); with nothing waiting, the corridor offers a jump
// into the busiest classroom instead.

import Icon from '../../../../lib/components/Icon.svelte';
import ProviderIcon from '../../../../lib/components/ProviderIcon.svelte';
import { roomSummary, type Kid, type Room } from '../../school/model';
import type { SchoolHandle, SchoolView } from '../../school/scene';
;

  interface Props {
    queue: Kid[];
    queueLabel: string;
    /** Expanded (kept by the box while the queue comes and goes). */
    queueOpen: boolean;
    view: SchoolView;
    busiest: Room | null;
    handle: SchoolHandle | null;
    open: (k: Kid) => Promise<void>;
    goTo: (k: Kid) => Promise<void>;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { queue, queueLabel, queueOpen = $bindable(), view, busiest, handle, open, goTo }: Props = $props()/*Ωignore_startΩ*/;queueOpen;/*Ωignore_endΩ*/;

  const QUEUE_SHOWN = 5;
;
async () => {

if(queue.length > 0){
  
   { svelteHTML.createElement("section", {    "class":`queue`,"aria-label":`Sessions that need you`,});view.kind === 'corridor';
     { svelteHTML.createElement("button", {     "class":`queue-head`,"aria-expanded":queueOpen,"onclick":() => (queueOpen = !queueOpen),});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`hand`,"size":12,}});}
       { svelteHTML.createElement("span", {});queueLabel; }
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":queueOpen ? 'chevronUp' : 'chevronDown',"size":12,}});}
     }
    if(queueOpen){
       { svelteHTML.createElement("ul", {});
           for(let k of __sveltets_2_ensureArray((queue.slice(0, QUEUE_SHOWN)))){k.id;
           { svelteHTML.createElement("li", {  "class":`qrow`,});k.pose === 'error';
             { svelteHTML.createElement("button", {     "class":`qmain`,"onclick":() => void goTo(k),"title":`Go to ${k.title} in ${k.workspaceName}`,});
               { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {    "provider":k.provider,"size":12,}});}
               { svelteHTML.createElement("span", { "class":`ellipsis qt`,});k.title; }
               { svelteHTML.createElement("small", { "class":`ellipsis`,});k.workspaceName; }
             }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void open(k),"aria-label":`Open ${k.title}`,"title":`Open ${k.title}`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});} }
           }
        }
       }
      if(queue.length > QUEUE_SHOWN){ { svelteHTML.createElement("p", { "class":`qmore`,}); queue.length - QUEUE_SHOWN;     }}
    }
   }
} else if (view.kind === 'corridor' && busiest){
   { svelteHTML.createElement("button", {     "class":`jump`,"onclick":() => busiest && void handle?.enterRoom(busiest.id),"title":`Walk into the busiest classroom`,});
     { svelteHTML.createElement("span", { "class":`ellipsis`,});busiest.name; } { svelteHTML.createElement("small", {});roomSummary(busiest); } { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('queueOpen'), slots: {}, events: {} }}
const NeedsYouQueue__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NeedsYouQueue__SvelteComponent_ = ReturnType<typeof NeedsYouQueue__SvelteComponent_>;
/*Ωignore_endΩ*/export default NeedsYouQueue__SvelteComponent_;