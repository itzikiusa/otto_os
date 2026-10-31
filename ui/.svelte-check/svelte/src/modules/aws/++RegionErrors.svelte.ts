///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
// Inline note for an "All enabled regions" list: the regions that failed
// (denied, opt-in disabled, throttled…) while the rest still rendered.
import Icon from '../../lib/components/Icon.svelte';
import type { AwsRegionError } from '../../lib/api/types';
;

  interface Props {
    errors: AwsRegionError[];
  };function $$render() {

  
  
  
  
  
  

  let { errors }: Props = $props();
  let open = $state(false);
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const count = $derived(errors.length);
  $effect(() => { if (count) announce(`${plural(count, 'region')} couldn’t be listed — the rows below come from the others.`); });
;
async () => {

if(errors.length){
   { svelteHTML.createElement("div", { "class":`rerr`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}
     { svelteHTML.createElement("span", {});plural(errors.length, 'region');            }
     { svelteHTML.createElement("button", {     "class":`link`,"onclick":() => (open = !open),"aria-expanded":open,});open ? 'Hide' : 'Details'; }
    if(open){
       { svelteHTML.createElement("ul", {});
           for(let e of __sveltets_2_ensureArray(errors)){e.region;
           { svelteHTML.createElement("li", {}); { svelteHTML.createElement("span", { "class":`mono`,});e.region; }  e.message; }
        }
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RegionErrors__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RegionErrors__SvelteComponent_ = ReturnType<typeof RegionErrors__SvelteComponent_>;
/*Ωignore_endΩ*/export default RegionErrors__SvelteComponent_;