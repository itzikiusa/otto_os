///<reference types="svelte" />
;
import { api } from '../../lib/api/client';
import { toastError } from '../../lib/toastError';
import type { HandoverDelivery, Session } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { handoverStateLabel } from '../../lib/labels';

;type $$ComponentProps =  { session: Session; readonly?: boolean };function $$render() {

  
  
  
  
  
  

  let { session, readonly = false }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  const delivery = $derived(session.meta?.handover as HandoverDelivery | undefined);
  let busy = $state(false);
  async function act(action: 'retry' | 'acknowledge') {
    if (!delivery || busy || readonly) return;
    const targetId = session.id;
    const deliveryId = delivery.id;
    busy = true;
    try {
      const updated = await api.post<Session>(`/sessions/${targetId}/handover/${action}`, { delivery_id: deliveryId });
      ws.sessions = ws.sessions.map((s) => s.id === targetId ? updated : s);
      toasts.success(action === 'retry' ? 'Handover retry started' : 'Receipt confirmed');
    } catch (e) {
      toastError('Couldn’t update the handover', e);
    } finally { busy = false; }
  }
;
async () => {

if(delivery){
   { svelteHTML.createElement("details", { "class":`handover-delivery`,});
     { svelteHTML.createElement("summary", {});  handoverStateLabel(delivery.state).toLowerCase(); }
    if(delivery.error){ { svelteHTML.createElement("p", { "role":`alert`,});delivery.error; }}
    if(delivery.focus){ { svelteHTML.createElement("p", {}); { svelteHTML.createElement("strong", {});  } delivery.focus; }}
    if(delivery.brief){ { svelteHTML.createElement("pre", {});delivery.brief; }}else{ { svelteHTML.createElement("p", {});    }}
    if(delivery.state === 'sent'){
       { svelteHTML.createElement("p", {});          
        delivery.archive_source ? 'The source will be archived only after confirmation.' : ''; }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":readonly || busy,"onclick":() => act('acknowledge'),});  }
    } else if (delivery.state !== 'acknowledged'){
       { svelteHTML.createElement("p", {});               }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":readonly || busy,"onclick":() => act('retry'),});  }
    }
   }
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HandoverDeliveryPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HandoverDeliveryPanel__SvelteComponent_ = ReturnType<typeof HandoverDeliveryPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default HandoverDeliveryPanel__SvelteComponent_;