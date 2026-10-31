///<reference types="svelte" />
;// A row of proof badges. Renders nothing when the list is empty.

import ProofBadge from './ProofBadge.svelte';
;
  interface Props {
    badges: string[];
  };function $$render() {

  
  

  let { badges }: Props = $props();
;
async () => {

if(badges.length > 0){
   { svelteHTML.createElement("span", { "class":`proof-badges`,});
       for(let b of __sveltets_2_ensureArray(badges)){b;
       { const $$_egdaBfoorP1C = __sveltets_2_ensureComponent(ProofBadge); new $$_egdaBfoorP1C({ target: __sveltets_2_any(), props: {  "badge":b,}});}
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProofBadges__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProofBadges__SvelteComponent_ = ReturnType<typeof ProofBadges__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProofBadges__SvelteComponent_;