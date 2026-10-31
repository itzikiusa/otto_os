///<reference types="svelte" />
;// Shared region switcher for the regional AWS views (A-1). Seeds from the
// last region picked for this account + service (localStorage, per viewer),
// falling back to the account's default region. `allowAll` adds the
// "All enabled regions" option (EC2 / EKS / RDS fan out server-side).

import { untrack } from 'svelte';
import { aws } from '../../lib/stores/aws.svelte';
import { ALL_REGIONS } from '../../lib/api/aws';
import type { AwsAccount } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    /** Storage key part — `sqs`, `athena`, `ec2`, … */
    service: string;
    region: string;
    allowAll?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { account, service, region = $bindable(), allowAll = false }: Props = $props()/*Ωignore_startΩ*/;region;/*Ωignore_endΩ*/;

  const key = (a: AwsAccount) => `otto.aws.region.${a.id}.${service}`;

  // Restore once per mount (AwsPage remounts the view per account + service).
  $effect(() => {
    untrack(() => {
      void aws.loadRegions();
      try {
        const saved = localStorage.getItem(key(account));
        if (saved && (saved !== ALL_REGIONS || allowAll)) region = saved;
      } catch {
        // storage blocked (private window / preview) — keep the default
      }
    });
  });

  function pick(v: string): void {
    region = v;
    try {
      if (v === account.region) localStorage.removeItem(key(account));
      else localStorage.setItem(key(account), v);
    } catch {
      // ignore — the pick still applies for this view
    }
  }
;
async () => {

 { svelteHTML.createElement("label", { "class":`sel`,});
   { svelteHTML.createElement("span", { "class":`lbl`,});  }
  if(aws.regions.length){
     { svelteHTML.createElement("select", {       "value":region,"onchange":(e) => pick((e.currentTarget as HTMLSelectElement).value),"aria-label":`Region`,"title":`Region`,});
      if(allowAll){ { svelteHTML.createElement("option", { "value":ALL_REGIONS,});   }}
         for(let r of __sveltets_2_ensureArray(aws.regions)){r.code;
         { svelteHTML.createElement("option", { "value":r.code,});r.code;r.code === account.region ? ' (default)' : ''; }
      }
     }
  }else{
     { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`mono`,"value":region,"onchange":(e) => pick((e.currentTarget as HTMLInputElement).value.trim() || account.region),"aria-label":`Region`,"size":12,});}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('region'), slots: {}, events: {} }}
const RegionPicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RegionPicker__SvelteComponent_ = ReturnType<typeof RegionPicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default RegionPicker__SvelteComponent_;