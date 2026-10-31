///<reference types="svelte" />
;// History retention (workspace setting, opt-in): keep the newest N requests
// and/or drop requests older than D days, and the newest N runs per
// automation. 0 = no limit. Saving applies the limits right away.

import Modal from '../../lib/components/Modal.svelte';
import { toastError } from '../../lib/toastError';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { toasts } from '../../lib/toast.svelte';
import { formatBytes } from './storageGauge';
;

  interface Props { onclose: () => void };function $$render() {

  
  
  
  
  
  
  
  

  let { onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let rows = $state(String(apiClient.historyRetention.rows));
  // svelte-ignore state_referenced_locally
  let days = $state(String(apiClient.historyRetention.days));
  // svelte-ignore state_referenced_locally
  let runs = $state(String(apiClient.runsKeep));
  let busy = $state(false);

  const parse = (v: string): number | null => {
    const n = Number(v.trim());
    return v.trim() !== '' && Number.isInteger(n) && n >= 0 ? n : null;
  };
  /** 100 years: the daemon rejects/clamps anything larger (a huge age once
   *  overflowed the prune cutoff and panicked every prune — backend-correctness-2-01). */
  const MAX_DAYS = 36_500;
  const r = $derived(parse(rows));
  const daysRaw = $derived(parse(days));
  const daysTooBig = $derived(daysRaw !== null && daysRaw > MAX_DAYS);
  const d = $derived(daysTooBig ? null : daysRaw);
  const k = $derived(parse(runs));
  $effect(() => { void apiClient.loadStorage(); });
  const gauge = $derived(apiClient.storage);

  async function save(): Promise<void> {
    if (r === null || d === null || k === null) return;
    busy = true;
    try {
      await apiClient.setHistoryRetention(r, d);
      if (k !== apiClient.runsKeep) await apiClient.setRunsKeep(k);
      const applied = r || d || k ? await apiClient.applyRetention() : true;
      if (applied) toasts.success('Retention saved', r || d || k ? 'Applied now, and after every request or run.' : 'Everything is kept.');
      onclose();
    } catch (e) {
      toastError('Couldn’t save history retention', e);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`History retention`,"width":420,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":save,"disabled":busy || r === null || d === null || k === null,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("p", { "class":`lead`,});                        }
  if(gauge){
     { svelteHTML.createElement("p", { "class":`gauge`,});
       gauge.history_rows.toLocaleString();  formatBytes(gauge.history_bytes);  gauge.run_rows.toLocaleString();   formatBytes(gauge.run_bytes);
     }
  }
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ret-rows`,});   }
     { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ret-rows`,"class":`input`,"inputmode":`numeric`,"bind:value":rows,});/*Ωignore_startΩ*/() => rows = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("span", {  "class":`hint`,});r === null;r === null ? 'Enter a whole number (0 = no limit).' : r === 0 ? 'No limit on the number of requests.' : `Keep ${r} requests.`; }
   }
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ret-days`,});     }
     { svelteHTML.createElement("input", {              "dir":`auto`,"id":`ret-days`,"class":`input`,"inputmode":`numeric`,"bind:value":days,"aria-invalid":d === null ? 'true' : undefined,"aria-describedby":`ret-days-hint`,});/*Ωignore_startΩ*/() => days = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("span", {    "class":`hint`,"id":`ret-days-hint`,});d === null;daysTooBig ? `Enter ${MAX_DAYS.toLocaleString()} days (100 years) or fewer.` : d === null ? 'Enter a whole number (0 = never).' : d === 0 ? 'Never delete by age.' : `Delete after ${d} days.`; }
   }
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ret-runs`,});      }
     { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ret-runs`,"class":`input`,"inputmode":`numeric`,"bind:value":runs,});/*Ωignore_startΩ*/() => runs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("span", {  "class":`hint`,});k === null;k === null ? 'Enter a whole number (0 = no limit).' : k === 0 ? 'Keep every run.' : `Keep the newest ${k} finished runs of each automation.`; }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RetentionDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RetentionDialog__SvelteComponent_ = ReturnType<typeof RetentionDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default RetentionDialog__SvelteComponent_;