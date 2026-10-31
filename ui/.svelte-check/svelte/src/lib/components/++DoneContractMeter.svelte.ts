///<reference types="svelte" />
;// The "done contract" (R8): an explainable readiness score rendered as a ring
// plus an itemized checklist. Each item shows satisfied ✓ / missing ✗, its
// label, a required/optional tag, and the human-readable detail text.

import Icon from './Icon.svelte';
import Badge from './Badge.svelte';
import type { DoneContract } from '../api/types';
;

  interface Props {
    contract: DoneContract;
  };;

  type Tone = 'ok' | 'warn' | 'bad';;function $$render() {

  
  
  
  
  
  

  let { contract }: Props = $props();

  // `satisfied` includes optional successes; the required checklist must not.
  const requiredSatisfied = $derived(contract.items.filter(item => item.required && item.satisfied).length);
  const score = $derived(Math.max(0, Math.min(100, Math.round(contract.score))));

  const tone = $derived<Tone>(score >= 80 ? 'ok' : score >= 50 ? 'warn' : 'bad');

  // Ring geometry: r=26 → circumference ≈ 163.36; the dash offset draws the arc.
  const R = 26;
  const CIRC = 2 * Math.PI * R;
  const dash = $derived((score / 100) * CIRC);
;
async () => {

 { svelteHTML.createElement("section", {   "class":`meter`,"aria-label":`Done contract`,});
   { svelteHTML.createElement("div", { "class":`ring-wrap ${tone}`,});
     { svelteHTML.createElement("svg", {       "viewBox":`0 0 64 64`,"class":`ring`,"role":`img`,"aria-label":`Done score ${score} of 100`,});
       { svelteHTML.createElement("circle", {            "class":`track`,"cx":`32`,"cy":`32`,"r":R,"fill":`none`,"stroke-width":`7`,});}
       { svelteHTML.createElement("circle", {                   "class":`value`,"cx":`32`,"cy":`32`,"r":R,"fill":`none`,"stroke-width":`7`,"stroke-linecap":`round`,"stroke-dasharray":`${dash} ${CIRC}`,"transform":`rotate(-90 32 32)`,});}
     }
     { svelteHTML.createElement("div", { "class":`ring-label`,});
       { svelteHTML.createElement("span", { "class":`num`,});score; }
       { svelteHTML.createElement("span", { "class":`den`,});  }
     }
   }

   { svelteHTML.createElement("div", { "class":`contract`,});
     { svelteHTML.createElement("div", { "class":`contract-head`,});
       { svelteHTML.createElement("span", { "class":`title`,});  }
       { svelteHTML.createElement("span", { "class":`dim`,});requiredSatisfied; contract.required;   }
     }
    if(contract.items.length === 0){
       { svelteHTML.createElement("p", { "class":`dim empty`,});   }
    }else{
       { svelteHTML.createElement("ul", { "class":`items`,});
           for(let it of __sveltets_2_ensureArray(contract.items)){it.key;
           { svelteHTML.createElement("li", {   "class":`item`,});it.satisfied;!it.satisfied;
             { svelteHTML.createElement("span", {   "class":`mark`,"aria-hidden":`true`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":it.satisfied ? 'check' : 'x',"size":12,}});}
             }
             { svelteHTML.createElement("div", { "class":`item-body`,});
               { svelteHTML.createElement("div", { "class":`item-top`,});
                 { svelteHTML.createElement("span", { "class":`item-label`,});it.label; }
                 { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {      "tone":it.required ? 'accent' : 'neutral',"variant":`outline`,"label":it.required ? 'Required' : 'Optional',}});}
               }
              if(it.detail){
                 { svelteHTML.createElement("span", { "class":`item-detail`,});it.detail; }
              }
             }
           }
        }
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DoneContractMeter__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DoneContractMeter__SvelteComponent_ = ReturnType<typeof DoneContractMeter__SvelteComponent_>;
/*Ωignore_endΩ*/export default DoneContractMeter__SvelteComponent_;