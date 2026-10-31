///<reference types="svelte" />
;// Review status as the shared Badge with a dot (colour + word, never colour
// alone). With `onclick` it is the artifact header's status menu button —
// the same `.chip` look, as a real <button>.

import Badge from '../../lib/components/Badge.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { badgeTone, designStatus } from '../../lib/status';
import type { DesignStatus } from '../../lib/api/types';
;

  interface Props {
    status: DesignStatus | string;
    onclick?: (e: MouseEvent) => void;
    /** Extra words after the label ("· v8"). */
    suffix?: string;
  };function $$render() {

  
  
  
  
  
  
  

  let { status, onclick, suffix = '' }: Props = $props();
  const info = $derived(designStatus(status));
  const tone = $derived(badgeTone(info.tone));
;
async () => {

if(onclick){
   { svelteHTML.createElement("button", {        "class":`chip ${tone} as-btn`,onclick,"aria-haspopup":`menu`,"title":`Change status`,...__sveltets_2_empty({"data-testid":`design-status`}),});
     { svelteHTML.createElement("span", {   "class":`dot`,"aria-hidden":`true`,}); }info.label;suffix;
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
   }
}else{
    { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {   tone,"label":`${info.label}${suffix}`,"dot":true,}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StatusPill__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StatusPill__SvelteComponent_ = ReturnType<typeof StatusPill__SvelteComponent_>;
/*Ωignore_endΩ*/export default StatusPill__SvelteComponent_;