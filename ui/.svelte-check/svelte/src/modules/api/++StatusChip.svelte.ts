///<reference types="svelte" />
;// HTTP status as a soft-tinted pill: "200 OK", tone by class, with the class
// in words in the tooltip (colour is never the only signal).

import { statusTone, statusWord } from '../../lib/api/apiVars';
;

  interface Props {
    status: number | null | undefined;
    text?: string | null;
    small?: boolean;
  };function $$render() {

  
  
  

  let { status, text = null, small = false }: Props = $props();
;
async () => {

 { svelteHTML.createElement("span", {   "class":`schip ${statusTone(status)}`,"title":statusWord(status),});small;
  status ?? 'No response';if(text){ text;}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StatusChip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StatusChip__SvelteComponent_ = ReturnType<typeof StatusChip__SvelteComponent_>;
/*Ωignore_endΩ*/export default StatusChip__SvelteComponent_;