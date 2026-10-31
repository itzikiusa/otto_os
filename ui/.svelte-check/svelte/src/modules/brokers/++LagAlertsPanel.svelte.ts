///<reference types="svelte" />
;
import Badge from '../../lib/components/Badge.svelte';
// Lag alert configuration UI. Lists configured alerts for a cluster,
// showing breach status when the last metrics sweep detected a threshold
// crossing. Allows creating and deleting alerts.
import { api } from '../../lib/api/client';
import { toastError } from '../../lib/toastError';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { BrokerCluster } from '../../lib/api/types';
import type { LagAlert } from './types';
;

  interface Props {
    cluster: BrokerCluster;
  };function $$render() {

  
  
  
  

  
  
  
  
  
  
  
  

  let { cluster }: Props = $props();

  let alerts = $state<LagAlert[]>([]);
  let loading = $state(true);
  /** Failed load — inline with Retry, never "No alerts configured.". */
  let loadError = $state<string | null>(null);

  // New alert form fields.
  let newTopic = $state('');
  let newGroup = $state('');
  let newThreshold = $state(1000);
  let creating = $state(false);
  // Why "Add" is disabled, in words (a disabled button must say why).
  const addBlock = $derived(
    !newTopic.trim() || !newGroup.trim()
      ? 'Enter a topic and a consumer group first'
      : !(Number(newThreshold) > 0)
        ? 'Threshold must be greater than 0'
        : null,
  );

  $effect(() => {
    void cluster.id;
    alerts = [];
    loadAlerts();
  });

  function loadAlerts(): void {
    loading = true;
    loadError = null;
    api
      .get<LagAlert[]>(`/brokers/clusters/${cluster.id}/lag-alerts`)
      .then((a) => (alerts = a))
      .catch((e) => (loadError = loadErrorText(e)))
      .finally(() => (loading = false));
  }

  async function createAlert() {
    if (addBlock) return;
    creating = true;
    try {
      const a = await api.post<LagAlert>(`/brokers/clusters/${cluster.id}/lag-alerts`, {
        topic: newTopic.trim(),
        group_name: newGroup.trim(),
        threshold: newThreshold,
      });
      alerts = [...alerts, a];
      newTopic = '';
      newGroup = '';
      newThreshold = 1000;
      toasts.success('Alert created');
    } catch (e) {
      toastError('Couldn’t create the alert', e);
    } finally {
      creating = false;
    }
  }

  async function deleteAlert(alert: LagAlert) {
    const ok = await confirmer.ask(
      `Delete the lag alert for "${alert.topic}" / "${alert.group_name}"? It stops being evaluated.`,
      { title: 'Delete alert', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    try {
      await api.del(`/brokers/clusters/${cluster.id}/lag-alerts/${alert.id}`);
      alerts = alerts.filter((a) => a.id !== alert.id);
      toasts.success('Alert deleted');
    } catch (e) {
      toastError('Couldn’t delete the alert', e);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`lag-alerts`,});
   { svelteHTML.createElement("h5", {});  }
   { svelteHTML.createElement("p", { "class":`muted small`,});
               
               
   }

  
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`lag alerts`,"variant":`compact`,loading,"error":loadError,"empty":alerts.length === 0,"onretry":loadAlerts,emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`muted pad`,});    }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
       { svelteHTML.createElement("table", {});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  }
             { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {}); }
           }
         }
         { svelteHTML.createElement("tbody", {});
             for(let a of __sveltets_2_ensureArray(alerts)){a.id;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("td", { "class":`mono`,});a.topic; }
               { svelteHTML.createElement("td", { "class":`mono`,});a.group_name; }
               { svelteHTML.createElement("td", {});a.threshold.toLocaleString(); }
               { svelteHTML.createElement("td", {});
                if(a.breach_lag !== undefined){
                   { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":`bad`,"label":`Breached (${a.breach_lag.toLocaleString()})`,}});}
                } else if (a.enabled){
                   { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":`ok`,"label":`Active`,}});}
                }else{
                   { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {  "label":`Disabled`,}});}
                }
               }
               { svelteHTML.createElement("td", {});
                 { svelteHTML.createElement("button", {       "class":`btn small danger`,"onclick":() => deleteAlert(a),"title":`Delete alert`,});  }
               }
             }
          }
         }
       }
   LoadState}

  if(alerts.length > 0 || (!loading && !loadError)){
     { svelteHTML.createElement("h5", { "class":`create-head`,});  }
     { svelteHTML.createElement("div", { "class":`create-row`,});
       { svelteHTML.createElement("input", {            "dir":`ltr`,"type":`text`,"bind:value":newTopic,"placeholder":`orders.events`,"class":`field`,"aria-label":`Topic`,});/*Ωignore_startΩ*/() => newTopic = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("input", {            "dir":`auto`,"type":`text`,"bind:value":newGroup,"placeholder":`billing-service`,"class":`field wide`,"aria-label":`Consumer group`,});/*Ωignore_startΩ*/() => newGroup = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("label", { "class":`thresh-label`,});
        
         { svelteHTML.createElement("input", {        "type":`number`,"bind:value":newThreshold,"min":`1`,"class":`narrow`,});/*Ωignore_startΩ*/() => newThreshold = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":createAlert,"disabled":creating || addBlock !== null,"title":addBlock ?? 'Add lag alert',});
        creating ? 'Creating…' : 'Add';
       }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LagAlertsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LagAlertsPanel__SvelteComponent_ = ReturnType<typeof LagAlertsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default LagAlertsPanel__SvelteComponent_;