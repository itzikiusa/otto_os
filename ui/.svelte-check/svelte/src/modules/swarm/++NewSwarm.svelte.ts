///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Create a swarm — from a preset (5 templates) or blank.
import { onMount } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { toastError } from '../../lib/toastError';
import { swarm } from '../../lib/stores/swarm.svelte';
import { toasts } from '../../lib/toast.svelte';
;

  interface Props {
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { onclose }: Props = $props();

  let name = $state('');
  let selected = $state<string>(''); // preset slug or '' for blank
  let busy = $state(false);

  onMount(() => {
    void swarm.loadPresets();
  });

  async function create() {
    if (!name.trim() || busy) return;
    busy = true;
    try {
      await swarm.createSwarm(name.trim(), selected || undefined);
      toasts.success('Swarm created');
      onclose();
    } catch (e) {
      toastError('Couldn’t create the swarm', e);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`New swarm`,"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":create,"disabled":!name.trim() || busy,"title":name.trim() ? undefined : 'Name the swarm first',});busy ? 'Creating…' : 'Create'; }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`swarm-name`,});  }
     { svelteHTML.createElement("input", {          "dir":`auto`,"id":`swarm-name`,"class":`input`,"placeholder":`e.g. Platform Team`,"bind:value":name,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }

   { svelteHTML.createElement("p", { "class":`section-title`,});    }
   { svelteHTML.createElement("div", {     "class":`presets`,"role":`group`,"aria-label":`Preset`,});
     { svelteHTML.createElement("button", {      "class":`preset`,"aria-pressed":selected === '',"onclick":() => (selected = ''),});selected === '';
       { svelteHTML.createElement("div", { "class":`p-name`,});  }
       { svelteHTML.createElement("div", { "class":`p-desc dim`,});       }
     }
    if(swarm.presetsLoading || swarm.presetsError){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             "what":`presets`,"variant":`compact`,"loading":swarm.presetsLoading,"error":swarm.presetsError,"empty":swarm.presets.length === 0,"onretry":() => void swarm.loadPresets(),}});}
    }
       for(let p of __sveltets_2_ensureArray(swarm.presets)){p.slug;
       { svelteHTML.createElement("button", {      "class":`preset`,"aria-pressed":selected === p.slug,"onclick":() => (selected = p.slug),});selected === p.slug;
         { svelteHTML.createElement("div", { "class":`p-name`,});p.name;  { svelteHTML.createElement("span", { "class":`dim`,}); plural(p.agents.length, 'agent'); } }
         { svelteHTML.createElement("div", { "class":`p-desc dim`,});p.description; }
       }
    }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NewSwarm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NewSwarm__SvelteComponent_ = ReturnType<typeof NewSwarm__SvelteComponent_>;
/*Ωignore_endΩ*/export default NewSwarm__SvelteComponent_;