///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// Pin provider + model for the whole thread (the model chip), or go back to
// the routing rules. Explicit always wins over the router (plan §2.4).
import Modal from '../../lib/components/Modal.svelte';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { assistant, describeError } from '../../lib/stores/assistant.svelte';
import { toasts } from '../../lib/toast.svelte';
import { PROVIDER_NAME, type Provider } from '../../lib/assistant/model';
import type { AssistantThread } from '../../lib/api/types';
;

  interface Props {
    thread: AssistantThread;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { thread, onclose }: Props = $props();

  const PROVIDERS: Provider[] = ['claude', 'codex'];
  // Seeded once from the thread; the sheet edits a local draft.
  let provider = $state<Provider>((() => (thread.provider === 'codex' ? 'codex' : 'claude'))());
  let model = $state((() => (thread.route_pinned ? (thread.model ?? '') : ''))());
  let busy = $state(false);
  let error = $state('');

  async function pin(): Promise<void> {
    busy = true;
    error = '';
    try {
      await assistant.route(thread.id, provider, model.trim() || null);
      onclose();
    } catch (e) {
      error = `Couldn’t pin the model. ${describeError(e)}`;
    } finally {
      busy = false;
    }
  }
  async function unpin(): Promise<void> {
    busy = true;
    try {
      await assistant.route(thread.id, null);
      onclose();
    } catch (e) {
      toasts.error('Couldn’t switch back to routing rules', describeError(e));
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Model for this thread`,"width":460,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(thread.route_pinned){
       { svelteHTML.createElement("button", {     "class":`btn ghost reset`,"onclick":() => void unpin(),"disabled":busy,});   }
    }
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":() => void pin(),"disabled":busy,});busy ? 'Pinning…' : 'Pin to thread'; }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("p", { "class":`lead`,});
       thread.title;            { svelteHTML.createElement("span", { "class":`mono`,});  } 
     { svelteHTML.createElement("span", { "class":`mono`,});  }     
   }
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("span", {   "class":`lbl`,"id":`pin-provider`,});  }
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-labelledby":`pin-provider`,});
         for(let p of __sveltets_2_ensureArray(PROVIDERS)){p;
         { svelteHTML.createElement("button", {        "type":`button`,"aria-pressed":provider === p,"onclick":() => {
            provider = p;
            model = '';
          },});provider === p;
           { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {    "provider":p,"size":12,}});}
          PROVIDER_NAME[p];
         }
      }
     }
   }
  provider; {
     { const $$_rekciPledoM1C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM1C({ target: __sveltets_2_any(), props: {       provider,"value":model,"onchange":(m) => (model = m),"hint":`Empty uses the provider’s default model.`,}});}
  }
  if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`err`,});error; }}}
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PinModelSheet__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PinModelSheet__SvelteComponent_ = ReturnType<typeof PinModelSheet__SvelteComponent_>;
/*Ωignore_endΩ*/export default PinModelSheet__SvelteComponent_;