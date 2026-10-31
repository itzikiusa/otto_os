///<reference types="svelte" />
;// Otto School — the selected kid's card (ClassroomsBox, room / screen
// view): its state lines and every action as a button — open, look at the
// screen, send the headmaster over, detention, kick out, release.

import Icon from '../../../../lib/components/Icon.svelte';
import ProviderIcon from '../../../../lib/components/ProviderIcon.svelte';
import { relTime } from '../../../mission-control/lib';
import { kidLines, type Kid } from '../../school/model';
import type { SchoolHandle, SchoolView } from '../../school/scene';
;

  interface Props {
    selKid: Kid;
    view: SchoolView;
    handle: SchoolHandle | null;
    /** The card element (the box keeps it in view). */
    el?: HTMLElement | null;
    onclose: () => void;
    open: (k: Kid) => Promise<void>;
    free: (k: Kid) => Promise<void>;
    checkOn: (k: Kid) => void;
    detention: (k: Kid) => Promise<void>;
    kick: (k: Kid) => Promise<void>;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { selKid, view, handle, el = $bindable(null), onclose, open, free, checkOn, detention, kick }: Props = $props()/*Ωignore_startΩ*/;el;/*Ωignore_endΩ*/;
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {      "class":`kid-card`,"role":`group`,"aria-label":`${selKid.title} — details`,});el = $$_div0;
   { svelteHTML.createElement("div", { "class":`card-head`,});
     { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":selKid.provider,"size":16,}});}
     { svelteHTML.createElement("b", { "class":`ellipsis`,});selKid.title; }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onclose,"aria-label":`Close details`,"title":`Close details`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
   }
      for(let line of __sveltets_2_ensureArray((kidLines(selKid, relTime(selKid.lastActiveAt))))){let i = 1;i;
     { svelteHTML.createElement("div", {  "class":`card-line`,});i === 0 && selKid.pose === 'needs-you';line; }
  }
   { svelteHTML.createElement("div", { "class":`card-actions`,});
    if(selKid.detention){
       { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":!selKid.canManage,"onclick":() => selKid && void free(selKid),});   }
    }else{
       { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => void open(selKid),});  }
      if(view.kind === 'screen'){
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void handle?.backToRoom(),});    }
      }else{
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => selKid && void handle?.lookAtScreen(selKid.id),});   }
      }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => selKid && checkOn(selKid),"title":`Send the headmaster over`,});  }
      if(selKid.canManage){
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => selKid && void detention(selKid),"title":`Archive — resumable, with Undo`,});  }
         { svelteHTML.createElement("button", {     "class":`btn small danger`,"onclick":() => selKid && void kick(selKid),"title":`Delete this session and its history`,});  }
      }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('el'), slots: {}, events: {} }}
const KidCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type KidCard__SvelteComponent_ = ReturnType<typeof KidCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default KidCard__SvelteComponent_;